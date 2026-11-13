//! The strike (07 §3.4, §6): one attack resolved against a target, in
//! order.
//!
//! | Step | Here |
//! |------|------|
//! | 1. Hook `strike_resolving` | A handler may change whom it strikes, the part, the energy and the effort, or cancel |
//! | 2. Reach | The gap between the two cells' middles, less the cell they take between them, is within the attack's `reach`; otherwise it misses |
//! | 3. Clash | Not built (C6): there is never one |
//! | 4. Aim | [`aim`](mod@super::aim) |
//! | 5. Defences | [`defence`](mod@super::defence): those in force for the target, in order; one that avoids makes it miss, one that interposes turns some aside and puts a surface in the way of the rest |
//! | 6. Delivery | A [`Hit`] through [`Sim::hit`], with what the target wears; or a packet on an item; recoil into the surface that struck |
//! | 7. Push | [`push`](mod@super::push) |
//! | 8. Results | `strike_landed`, `strike_missed`, `strike_stopped`; `on_land`, `on_miss`, `on_stopped`; the action's `$landed`, `$absorbed`, `$passed` |
//!
//! # Delivery
//!
//! ```text
//! packet.kind      = surface.kind
//! packet.energy    = energy × effort × (1 − push)
//! packet.intensity = packet.energy / (surface.area × surface.give)
//! hit.area         = surface.area
//! hit.span         = surface.span
//! hit.at           = the part found by aim, or one the body picks
//! hit.dir          = from the attacker's cell to the target's
//! ```
//!
//! What the target neither absorbs nor passes is **recoil** (07 §6.3):
//! it comes back into the surface that struck as a packet of its own,
//! of the surface's kind, at `returned / (area × give)`: a hit on the
//! attacker's own part, or damage to the item.
//!
//! # Passing through
//!
//! What exits the far side of a body strikes the block of the next cell
//! along the strike's direction (06 §7.4, the body wiring's). Where that
//! cell has no block it carries on: with `through`, to the next target
//! of the arc; with no arc, to the first creature in that cell, and so
//! on, up to [`MAX_CARRIED`] creatures. Where a block stood behind, it
//! took what it took and the strike ends there. An arc without
//! `through` strikes each of its targets with the attack's whole
//! energy, each a strike of its own.
//!
//! # Energy
//!
//! Invariant C2: what the targets absorbed, what they and what stood
//! behind them took, what was pushed, what came back and what passed
//! beyond the last add up to no more than what was put in. For a strike
//! that reached every target it is exactly what was put in.

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_data::{Defs, Runner};
use clash_expr::Value;
use clash_world::dims::{CELL_M, STOREY_M};
use clash_world::{Cause, CellPos, DamagePacket, DamageResult, Layer, Location, OccupantFilter};

use super::aim::{self, Aimed};
use super::attack::{ArcDef, ArcShape, AttackDef, RESULT_LOCALS};
use super::defence::{self, Defended, Incoming};
use super::events::{
    STRIKE_LANDED, STRIKE_MISSED, STRIKE_RECOILED, STRIKE_RESOLVING, STRIKE_STOPPED,
};
use super::push::{self, Push};
use super::reason;
use super::surface::{self, Striker, Surface};
use crate::Sim;
use crate::action::{ActionDef, ActionDefId};
use crate::body::props::{part_handle, part_of_handle};
use crate::body::{Hit, HitAt, PartKey};
use crate::bus::{EventPayload, FieldValue};
use crate::entity::Place;
use crate::event::{Subject, Where};
use crate::host::Bound;
use crate::item::Item;
use crate::life::events::{field_handle, handle_field, id_field};
use crate::reason::Reason;
use crate::rule::{StrikeCall, StrikeOutcome};

/// How many creatures standing behind one another a blow that goes
/// through may reach, beyond the one it was aimed at.
pub const MAX_CARRIED: usize = 16;

/// How deeply strikes may be resolved inside one another: a strike whose
/// `on_land` starts an action that strikes at once, and so on. Deeper
/// is refused (`combat.too_deep`).
pub const MAX_STRIKE_DEPTH: u8 = 8;

/// Half a cell: how far from a line a cell's middle may lie for the line
/// to pass through it.
const HALF_CELL: Real = Real::of(0.5);
/// Millimetres in a metre.
const MM_PER_M: Real = Real::of(1000.0);
/// Degrees in a half turn.
const HALF_TURN_DEG: Real = Real::of(180.0);

/// What a strike is asked to do: the effect's arguments, read.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StrikeArgs {
    /// Whom it strikes.
    pub target: Option<EntityId>,
    /// Where it strikes instead.
    pub at: Option<Location>,
    /// The part aimed at: its owner and its key.
    pub part: Option<(EntityId, PartKey)>,
    /// The share of the attack's energy put in, from 0 to 1.
    pub effort: Real,
    /// Whether it skips aim.
    pub sure: bool,
}

/// What came of a strike, with the energy accounted for (invariant C2).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct StrikeTally {
    /// The joules put in: the attack's energy at the effort given, once
    /// for each target an arc without `through` reached for.
    pub put_in: Real,
    /// Whether it landed on anyone or anything.
    pub landed: bool,
    /// Joules the bodies, what they wore, items and elements took.
    pub absorbed: Real,
    /// Joules that went on beyond the last thing struck.
    pub passed: Real,
    /// Joules that came back into the surface that struck.
    pub returned: Real,
    /// Joules set aside to push.
    pub pushed: Real,
    /// Joules a defence turned aside. Zero until defences are built.
    pub turned: Real,
    /// Joules a clash spilled. Zero until clashes are built.
    pub spilled: Real,
    /// How many targets it landed on.
    pub hits: u16,
}

impl StrikeTally {
    /// The joules accounted for: absorbed, passed on, returned, pushed,
    /// turned aside and spilled.
    pub fn accounted(&self) -> Real {
        self.absorbed + self.passed + self.returned + self.pushed + self.turned + self.spilled
    }
}

fn refuse(code: &'static str, message: impl Into<String>) -> Reason {
    Reason::new(code, message)
}

fn bad_number(what: &str, value: Real) -> Reason {
    refuse(
        reason::BAD_NUMBER,
        format!("the {what} of a strike came out as {value}, which it cannot use"),
    )
    .with("what", what)
    .with("value", value)
}

/// Runs the `strike` effect (07 §3.4): finds the action it is written
/// in and the attack it names, reads its arguments and resolves it.
pub fn run_strike(
    sim: &mut Sim,
    call: &StrikeCall<'_>,
    cause: Cause,
) -> Result<StrikeOutcome, Reason> {
    let defs = Arc::clone(sim.defs());
    let written = defs
        .exprs()
        .origin(call.origin)
        .map(|origin| origin.definition.clone());
    let no_attack = || {
        refuse(
            reason::NO_ATTACK,
            "`strike` is written in a list that belongs to no action",
        )
    };
    let label = written.ok_or_else(no_attack)?;
    if label.type_name != <ActionDef as clash_data::Def>::TYPE {
        return Err(no_attack().with("action", label.id.as_str()));
    }
    let action = defs.lookup::<ActionDef>(&label.id).ok_or_else(no_attack)?;
    let def = crate::action::run::action_def(&defs, action).ok_or_else(no_attack)?;
    let index = match call.attack {
        Some(name) => def.attacks.find(name).map(|(index, _)| index),
        None if def.attacks.0.len() == 1 => Some(0),
        None => None,
    }
    .ok_or_else(|| {
        refuse(
            reason::NO_ATTACK,
            format!(
                "the action `{}` has no attack {}",
                label.id,
                call.attack
                    .map_or("that is its only one".to_owned(), |name| format!(
                        "`{name}`"
                    ))
            ),
        )
        .with("action", label.id.as_str())
        .with("attack", call.attack.unwrap_or(""))
    })?;
    let actor = call
        .by
        .subject("self")
        .and_then(|value| sim.subject_of(value))
        .and_then(|subject| subject.entity())
        .ok_or_else(|| {
            refuse(
                reason::NO_TARGET,
                "a strike is made by `self`, and it is no entity",
            )
        })?;
    let entity_of = |value: &Value| match sim.subject_of(value) {
        Some(Subject::Entity(id)) => Some(id),
        _ => None,
    };
    let target = entity_of(&call.target);
    let at = match sim.subject_of(&call.at) {
        Some(Subject::Loc(loc)) => Some(loc),
        _ => None,
    };
    let part = call
        .part
        .as_subject()
        .and_then(|handle| part_of_handle(&sim.components().body, handle));
    let effort = if call.effort.is_null() {
        Real::ONE
    } else {
        call.effort.as_number().unwrap_or(Real::ZERO)
    };
    let sure = call.sure.as_bool().unwrap_or(false);
    let args = StrikeArgs {
        target,
        at,
        part,
        effort,
        sure,
    };
    let tally = strike(sim, actor, action, index, &args, call.locals, cause)?;
    Ok(StrikeOutcome {
        landed: tally.landed,
        absorbed: tally.absorbed,
        passed: tally.passed,
    })
}

/// Resolves one attack of an action (07 §3.4): see the
/// [module documentation](self). `locals` are the running action's, in
/// slot order: the attack's expressions read them.
///
/// Refused, having changed nothing, when what strikes cannot be found,
/// when a number it needs is not one it can use, when it names neither
/// a target nor a cell, and when it is aimed at a part that is not its
/// target's.
pub fn strike(
    sim: &mut Sim,
    actor: EntityId,
    action: ActionDefId,
    attack: usize,
    args: &StrikeArgs,
    locals: &[Value],
    cause: Cause,
) -> Result<StrikeTally, Reason> {
    if sim.strike_depth >= MAX_STRIKE_DEPTH {
        return Err(refuse(
            reason::TOO_DEEP,
            format!("a strike was resolved inside {MAX_STRIKE_DEPTH} others"),
        )
        .with("limit", i64::from(MAX_STRIKE_DEPTH)));
    }
    sim.strike_depth += 1;
    let result = resolve(sim, actor, action, attack, args, locals, cause);
    sim.strike_depth -= 1;
    result
}

/// What one strike works with.
struct Striking {
    defs: Arc<Defs>,
    actor: EntityId,
    action: ActionDefId,
    attack: usize,
    action_name: String,
    attack_name: String,
    surface: Surface,
    weapon: Value,
    limb: Value,
    locals: Vec<Value>,
    effort: Real,
    sure: bool,
    cause: Cause,
}

/// How one target came out.
enum AtTarget {
    /// It landed. What exited the far side, and whether a block stood
    /// behind to take it.
    Landed { exits: Real, walled: bool },
    /// It missed, or reached and delivered nothing: the energy is still
    /// the strike's.
    Missed,
    /// What was put in its way took all of it (07 §5.3): nothing goes
    /// on.
    Spent,
}

impl Striking {
    fn attack<'d>(&self, defs: &'d Defs) -> Option<&'d AttackDef> {
        crate::action::run::action_def(defs, self.action)?
            .attacks
            .0
            .get(self.attack)
    }

    /// Evaluates an expression of the attack against truth, with
    /// `target` bound, drawing from the rule stream.
    fn eval(
        &self,
        sim: &mut Sim,
        target: Option<EntityId>,
        expr: impl FnOnce(&AttackDef) -> Option<&clash_data::Expr<clash_data::expr::Number>>,
    ) -> Option<Real> {
        let defs = Arc::clone(&self.defs);
        let attack = self.attack(&defs)?;
        let expr = expr(attack)?;
        let bound = Bound {
            subjects: vec![
                sim.subject_value(Subject::Entity(self.actor)),
                self.weapon.clone(),
                self.limb.clone(),
                target.map_or(Value::Null, |id| sim.subject_value(Subject::Entity(id))),
            ],
            locals: self.locals.clone(),
            fields: Vec::new(),
        };
        let (value, warnings) = sim.with_random(bound, |host| {
            let mut runner = Runner::new();
            let value = expr.eval(&mut runner.on(&defs, host));
            (value, host.take_warnings())
        });
        for warning in warnings {
            sim.report_warning(warning);
        }
        value
    }

    fn payload(&self, name: &str) -> EventPayload {
        EventPayload::new(name)
            .subject("self", Subject::Entity(self.actor))
            .with("action", self.action_name.as_str())
            .with("attack", self.attack_name.as_str())
    }
}

#[allow(clippy::too_many_lines)]
fn resolve(
    sim: &mut Sim,
    actor: EntityId,
    action: ActionDefId,
    attack: usize,
    args: &StrikeArgs,
    locals: &[Value],
    cause: Cause,
) -> Result<StrikeTally, Reason> {
    let defs = Arc::clone(sim.defs());
    let action_name = crate::action::run::action_name(&defs, action);
    let def = crate::action::run::action_def(&defs, action)
        .and_then(|def| def.attacks.0.get(attack))
        .ok_or_else(|| {
            refuse(
                reason::NO_ATTACK,
                format!("the action `{action_name}` has no such attack"),
            )
            .with("action", action_name.as_str())
        })?;
    let attack_name = def.id.value.clone();
    sim.known(actor)?;
    let surface = surface::find(sim, actor, &def.with, &action_name, &attack_name)?;
    let body_defs = sim.life.body_defs().ok();
    let weapon = match surface.striker {
        Striker::Item { item, .. } => sim.subject_value(Subject::Entity(item)),
        _ => Value::Null,
    };
    let limb = match (surface.striker, &body_defs) {
        (Striker::Part(key), Some(body_defs)) => {
            Value::Subject(part_handle(body_defs.kinds(), actor, key))
        }
        _ => Value::Null,
    };
    let striking = Striking {
        defs: Arc::clone(&defs),
        actor,
        action,
        attack,
        action_name,
        attack_name,
        surface,
        weapon,
        limb,
        locals: locals.to_vec(),
        effort: args.effort,
        sure: args.sure,
        cause,
    };

    // Step 1: the hook, with what a handler may change.
    let mut target = args.target;
    let mut part = args.part;
    let mut effort = args.effort;
    let mut energy = striking
        .eval(sim, target, |attack| Some(&attack.energy))
        .unwrap_or(Real::ZERO);
    let payload = striking
        .payload(STRIKE_RESOLVING)
        .with(
            "target",
            target.map_or(FieldValue::Null, |id| {
                FieldValue::Subject(Subject::Entity(id))
            }),
        )
        .with(
            "part",
            match (part, &body_defs) {
                (Some((owner, key)), Some(body_defs)) => {
                    handle_field(part_handle(body_defs.kinds(), owner, key))
                }
                _ => FieldValue::Null,
            },
        )
        .with("energy", energy)
        .with("effort", effort);
    let raised = sim.raise(Where::Entity(actor), cause, payload)?;
    if raised.cancelled.is_some() {
        return Ok(StrikeTally::default());
    }
    if let Some(changed) = raised.field("energy").and_then(FieldValue::as_real) {
        energy = changed;
    }
    if let Some(changed) = raised.field("effort").and_then(FieldValue::as_real) {
        effort = changed;
    }
    match raised.field("target") {
        Some(FieldValue::Subject(Subject::Entity(id))) => target = Some(*id),
        Some(FieldValue::Null) | None => target = None,
        Some(other) => {
            target = field_handle(Some(other))
                .and_then(|handle| sim.exprs().subject(handle))
                .and_then(|subject| subject.entity());
        }
    }
    match field_handle(raised.field("part")) {
        Some(handle) => part = part_of_handle(&sim.components().body, handle),
        None => part = None,
    }
    if !(energy.is_finite() && energy >= Real::ZERO) {
        return Err(bad_number("energy", energy));
    }
    if !(effort.is_finite() && effort >= Real::ZERO && effort <= Real::ONE) {
        return Err(bad_number("effort", effort));
    }
    let striking = Striking { effort, ..striking };
    let put_in = energy * effort;

    // A strike at a cell.
    if target.is_none() {
        let Some(cell) = args.at else {
            return Err(refuse(
                reason::NO_TARGET,
                "a strike names whom or where it strikes",
            ));
        };
        return strike_cell(sim, &striking, cell, put_in);
    }
    let Some(target) = target.filter(|id| {
        sim.truth()
            .place(*id)
            .is_some_and(|place| place.loc().is_some())
    }) else {
        return Err(
            refuse(reason::NO_TARGET, "a strike's target is not in the world").with(
                "target",
                args.target.map_or(String::new(), |id| id.to_string()),
            ),
        );
    };
    if let Some((owner, _)) = part
        && owner != target
    {
        return Err(refuse(
            reason::NOT_ITS_PART,
            format!("the part a strike is aimed at is not a part of {target}"),
        )
        .with("target", target.to_string()));
    }
    let part_name = part.and_then(|(owner, key)| {
        sim.components()
            .body
            .get(owner)
            .and_then(|body| body.part(key))
            .map(|part| part.name().to_owned())
    });

    // The targets, in the order the surface meets them.
    let arc = def.arc.as_ref();
    let targets = match arc {
        Some(arc) => {
            let reach = reach_of(sim, &striking, target)?;
            arc_targets(sim, actor, target, arc, reach)
        }
        None => vec![target],
    };
    let through = arc.is_some_and(|arc| arc.through);
    let chained = arc.is_none() || through;

    let mut tally = StrikeTally::default();
    let mut carried = put_in;
    let mut struck: Vec<EntityId> = vec![actor];
    if chained {
        tally.put_in = put_in;
    }
    let mut queue: std::collections::VecDeque<EntityId> = targets.into_iter().collect();
    let mut beyond = 0;
    while let Some(next) = queue.pop_front() {
        if struck.contains(&next) {
            continue;
        }
        struck.push(next);
        let energy_here = if chained { carried } else { put_in };
        if !chained {
            tally.put_in += put_in;
        }
        if chained && energy_here <= Real::ZERO {
            break;
        }
        let aimed_at = if next == target {
            part.map(|(_, key)| key)
        } else {
            part_name.as_deref().and_then(|name| {
                sim.components()
                    .body
                    .get(next)
                    .and_then(|body| body.part_named(name))
                    .map(crate::body::PartInst::key)
            })
        };
        let behind_allowed = queue.is_empty() && arc.is_none();
        // Someone behind the target is reached by what went through it:
        // the blow is not aimed at them and does not push them.
        let carried_on = arc.is_none() && next != target;
        let outcome = at_target(
            sim,
            &striking,
            next,
            aimed_at,
            energy_here,
            carried_on,
            &mut tally,
        )?;
        match outcome {
            AtTarget::Landed { exits, walled } => {
                if chained {
                    carried = exits;
                    tally.passed = exits;
                } else {
                    tally.passed += exits;
                }
                if walled {
                    break;
                }
                // With no arc, what goes through carries on to whoever
                // stands behind.
                if behind_allowed
                    && exits > Real::ZERO
                    && beyond < MAX_CARRIED
                    && let Some(behind) = standing_behind(sim, &striking, next, &struck)
                {
                    beyond += 1;
                    queue.push_back(behind);
                }
            }
            AtTarget::Missed => {}
            AtTarget::Spent => {
                if chained {
                    tally.passed = Real::ZERO;
                    break;
                }
            }
        }
    }
    Ok(tally)
}

/// Delivers what is left of a strike to the surfaces defences put in
/// its way, in turn (07 §5.3): each takes a packet of the strike's kind
/// at the energy over its own area times its give, and passes on what
/// it cannot stop. Gives what is left for the part aimed at, what the
/// surfaces took, and what came back from one that was not affected.
fn through_guard(
    sim: &mut Sim,
    striking: &Striking,
    interposed: &[defence::Interposed],
    mut harm: Real,
) -> Result<(Real, Real, Real), Reason> {
    let (mut absorbed, mut returned) = (Real::ZERO, Real::ZERO);
    for put in interposed {
        if harm <= Real::ZERO {
            break;
        }
        let packet = DamagePacket::new(
            striking.surface.kind,
            harm,
            intensity_of(&put.surface, harm),
            striking.cause,
        );
        match put.surface.striker {
            Striker::Item { item, .. } => {
                let hit = crate::item::damage(sim, item, &packet)?;
                if hit.outcome.result == DamageResult::Unaffected {
                    returned += harm;
                    harm = Real::ZERO;
                } else {
                    absorbed += hit.outcome.absorbed;
                    harm = hit.outcome.passed;
                }
            }
            Striker::Part(key) => {
                // No direction: what exits the part goes on to the part
                // aimed at, not to the wall behind.
                let outcome = sim.hit(Hit {
                    target: put.defender,
                    at: HitAt::Part(key),
                    packet,
                    area: put.surface.area,
                    span: put.surface.span,
                    dir: None,
                })?;
                if outcome.cancelled.is_some() {
                    harm = Real::ZERO;
                    continue;
                }
                absorbed += outcome.report.absorbed;
                for worn in &outcome.report.worn {
                    absorbed += worn.absorbed;
                }
                returned += outcome.report.returned;
                harm = outcome.exited();
            }
            Striker::Inline => {}
        }
    }
    Ok((harm.max(Real::ZERO), absorbed, returned))
}

/// How far the attack reaches toward a target: its `reach`, or its
/// surface's.
fn reach_of(sim: &mut Sim, striking: &Striking, target: EntityId) -> Result<Real, Reason> {
    let reach = striking
        .eval(sim, Some(target), |attack| attack.reach.as_ref())
        .unwrap_or(striking.surface.reach);
    if reach.is_finite() && reach >= Real::ZERO {
        Ok(reach)
    } else {
        Err(bad_number("reach", reach))
    }
}

/// Where an entity stands, in metres in its grid: the middle of its cell
/// on the level, its footing in height.
pub fn point_of(sim: &Sim, entity: EntityId) -> Option<(Location, [Real; 3])> {
    let top = sim.truth().entities().outermost(entity)?;
    let Place::At { loc, alt_mm } = sim.truth().place(top)? else {
        return None;
    };
    let middle = |at: i32| (Real::from_i64(i64::from(at)) + HALF_CELL) * CELL_M;
    let height = Real::from_i64(i64::from(loc.cell.z)) * STOREY_M
        + Real::from_i64(i64::from(alt_mm)) / MM_PER_M;
    Some((loc, [middle(loc.cell.x), middle(loc.cell.y), height]))
}

/// The offset from one entity to another, in metres, the shorter way
/// round where the grid wraps. `None` for one that is not in the world,
/// or two in different grids.
pub fn offset(sim: &Sim, from: EntityId, to: EntityId) -> Option<[Real; 3]> {
    let (a, pa) = point_of(sim, from)?;
    let (b, pb) = point_of(sim, to)?;
    let delta = sim.truth().cosmos().delta(a, b)?;
    let within =
        |p: [Real; 3], loc: Location| p[2] - Real::from_i64(i64::from(loc.cell.z)) * STOREY_M;
    Some([
        Real::from_i64(i64::from(delta.x)) * CELL_M,
        Real::from_i64(i64::from(delta.y)) * CELL_M,
        Real::from_i64(i64::from(delta.z)) * STOREY_M + within(pb, b) - within(pa, a),
    ])
}

fn norm(v: [Real; 3]) -> Real {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn unit(v: [Real; 3]) -> Option<[Real; 3]> {
    let length = norm(v);
    (length > Real::ZERO).then(|| [v[0] / length, v[1] / length, v[2] / length])
}

/// The gap between two entities a strike must reach across (07 §3.1):
/// the distance between where they stand, less the cell the two of
/// them take up between them, half each. Zero for neighbours.
pub fn gap(sim: &Sim, from: EntityId, to: EntityId) -> Option<Real> {
    let v = offset(sim, from, to)?;
    Some((norm(v) - CELL_M).max(Real::ZERO))
}

/// The targets of an arc (07 §6.4), in the order the surface meets
/// them: creatures with bodies, other than the attacker, up to `max`.
///
/// - A **line** passes through the cells whose middles lie within half a
///   cell of the line from the attacker through the target named, ahead
///   of the attacker, no further than `line` metres beyond the cell
///   between; in order of distance.
/// - A **sweep** reaches those within the attack's reach whose bearing
///   from the attacker lies within half its angle of the target named's,
///   in order of bearing, turning from east toward south.
///
/// The target named is always among them.
pub fn arc_targets(
    sim: &Sim,
    actor: EntityId,
    target: EntityId,
    arc: &ArcDef,
    reach: Real,
) -> Vec<EntityId> {
    let Some(shape) = arc.shape() else {
        return vec![target];
    };
    let Some(axis) = offset(sim, actor, target) else {
        return vec![target];
    };
    let Some((loc, _)) = point_of(sim, actor) else {
        return vec![target];
    };
    let radius = match shape {
        ArcShape::Line(length) => length + CELL_M,
        ArcShape::Sweep(_) => reach + CELL_M,
    };
    let filter = OccupantFilter {
        except: Some(actor),
        ..OccupantFilter::ANY
    };
    let found = sim
        .truth()
        .cosmos()
        .occupants_within(loc, radius, &filter)
        .unwrap_or_default();
    let mut ids = clash_world::distinct_ids(&found);
    if !ids.contains(&target) {
        ids.push(target);
    }
    let level = |v: [Real; 3]| [v[0], v[1], Real::ZERO];
    let mut ranked: Vec<(Real, EntityId)> = Vec::new();
    for id in ids {
        if id != target && !sim.components().body.contains(id) {
            continue;
        }
        let Some(v) = offset(sim, actor, id) else {
            continue;
        };
        let key = match shape {
            ArcShape::Line(length) => {
                let Some(u) = unit(axis) else { continue };
                let along = v[0] * u[0] + v[1] * u[1] + v[2] * u[2];
                let off = [
                    v[0] - u[0] * along,
                    v[1] - u[1] * along,
                    v[2] - u[2] * along,
                ];
                if id != target
                    && (along <= Real::ZERO || norm(off) > HALF_CELL || along > length + CELL_M)
                {
                    continue;
                }
                along
            }
            ArcShape::Sweep(degrees) => {
                let (a, b) = (level(axis), level(v));
                let cross = a[0] * b[1] - a[1] * b[0];
                let dot = a[0] * b[0] + a[1] * b[1];
                let bearing = cross.atan2(dot) * HALF_TURN_DEG / Real::PI;
                let within_reach = (norm(v) - CELL_M).max(Real::ZERO) <= reach;
                if id != target && (!within_reach || bearing.abs() > degrees / Real::of(2.0)) {
                    continue;
                }
                bearing
            }
        };
        ranked.push((key, id));
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut out: Vec<EntityId> = ranked.into_iter().map(|(_, id)| id).collect();
    out.truncate(usize::from(arc.max));
    if !out.contains(&target) {
        // The target named is met whatever else is: it takes the place
        // of the last.
        out.pop();
        out.push(target);
    }
    out
}

/// The cell one step along a direction from a cell: each axis steps
/// whose share of the direction is at least half the greatest (06 §7.4).
fn next_along(loc: Location, dir: [Real; 3]) -> Option<Location> {
    let most = dir[0].abs().max(dir[1].abs()).max(dir[2].abs());
    if !most.is_finite() || most <= Real::ZERO {
        return None;
    }
    let step = |along: Real| {
        if along.abs() * Real::of(2.0) < most {
            0
        } else if along > Real::ZERO {
            1
        } else {
            -1
        }
    };
    Some(Location::new(
        loc.grid,
        CellPos::new(
            loc.cell.x + step(dir[0]),
            loc.cell.y + step(dir[1]),
            loc.cell.z + step(dir[2]),
        ),
    ))
}

/// The direction of a strike at an entity: from the attacker's place to
/// the target's, of length one.
fn direction(sim: &Sim, actor: EntityId, target: EntityId) -> Option<[Real; 3]> {
    unit(offset(sim, actor, target)?)
}

/// Whether the cell behind a target, along the strike's direction, holds
/// a block: what exits the target is spent on it (06 §7.4).
fn walled_behind(sim: &Sim, target: EntityId, dir: Option<[Real; 3]>) -> bool {
    let (Some(dir), Some((loc, _))) = (dir, point_of(sim, target)) else {
        return false;
    };
    next_along(loc, dir).is_some_and(|next| holds(sim, next, Layer::Block))
}

/// Whether a cell holds an element on a layer. A cell that is not
/// loaded holds none.
fn holds(sim: &Sim, loc: Location, layer: Layer) -> bool {
    sim.truth()
        .cosmos()
        .grid(loc.grid)
        .and_then(|view| view.element(loc.cell, layer))
        .is_ok_and(|state| !state.is_none())
}

/// The first creature with a body in the cell behind a target, along
/// the strike's direction, that has not been struck by it.
fn standing_behind(
    sim: &Sim,
    striking: &Striking,
    target: EntityId,
    struck: &[EntityId],
) -> Option<EntityId> {
    let dir = direction(sim, striking.actor, target)?;
    let (loc, _) = point_of(sim, target)?;
    let next = next_along(loc, dir)?;
    let found = sim
        .truth()
        .cosmos()
        .occupants_at(next, &OccupantFilter::ANY)
        .ok()?;
    found
        .into_iter()
        .map(|occupied| occupied.id)
        .find(|id| !struck.contains(id) && sim.components().body.contains(*id))
}

/// Steps 2 to 8 against one target with `energy` joules (07 §3.4). For
/// a creature `carried_on` to, behind the one struck, only delivery and
/// results: what went through is not aimed, needs no reach and pushes
/// nothing.
#[allow(clippy::too_many_lines)]
fn at_target(
    sim: &mut Sim,
    striking: &Striking,
    target: EntityId,
    part: Option<PartKey>,
    energy: Real,
    carried_on: bool,
    tally: &mut StrikeTally,
) -> Result<AtTarget, Reason> {
    let defs = Arc::clone(&striking.defs);
    let Some(attack) = striking.attack(&defs) else {
        return Ok(AtTarget::Missed);
    };
    let surface = striking.surface;

    // Step 2: reach.
    let within = carried_on || {
        let reach = reach_of(sim, striking, target)?;
        gap(sim, striking.actor, target).is_some_and(|gap| gap <= reach)
    };
    if !within {
        missed(sim, striking, target, "reach", None);
        return Ok(AtTarget::Missed);
    }

    // Step 3: a clash. Not built (C6).

    // Step 4: aim.
    let at = match (&attack.aim, striking.sure || carried_on) {
        (Some(aimed), false) => {
            let contest = aimed.contest.id();
            let by = striking
                .eval(sim, Some(target), |attack| {
                    attack.aim.as_ref().map(|aim| &aim.by)
                })
                .unwrap_or(Real::ZERO);
            let against = striking
                .eval(sim, Some(target), |attack| {
                    attack.aim.as_ref().map(|aim| &aim.against)
                })
                .unwrap_or(Real::ZERO);
            // A defence of the target's in force may make it harder to
            // land on squarely (07 §5.1 `evasion`).
            let evading = defence::evasion(
                sim,
                &Incoming {
                    attacker: striking.actor,
                    target,
                    part,
                    energy,
                    action: &striking.action_name,
                    attack: &striking.attack_name,
                    cause: striking.cause,
                },
            );
            aim::aim(sim, target, part, contest, by, against + evading)
        }
        _ => Aimed::Lands(aim::mark(sim, target, part), None),
    };
    let (at, margin) = match at {
        Aimed::Lands(at, margin) => (at, margin),
        Aimed::Misses(margin) => {
            missed(sim, striking, target, "aim", Some(margin));
            return Ok(AtTarget::Missed);
        }
    };

    // Step 5: defences (07 §5). What goes through a creature is not
    // aimed and meets none.
    let defended = if carried_on {
        Defended::Nothing
    } else {
        let incoming = Incoming {
            attacker: striking.actor,
            target,
            part: match at {
                HitAt::Part(key) => Some(key),
                _ => None,
            },
            energy,
            action: &striking.action_name,
            attack: &striking.attack_name,
            cause: striking.cause,
        };
        defence::defend(sim, &incoming)
    };
    let (energy, brace, interposed) = match defended {
        Defended::Nothing => (energy, Real::ZERO, Vec::new()),
        Defended::Avoided(margin) => {
            missed(sim, striking, target, "defence", margin);
            return Ok(AtTarget::Missed);
        }
        Defended::Interposed {
            turned,
            surfaces,
            brace,
        } => {
            tally.turned += turned;
            (energy - turned, brace, surfaces)
        }
    };

    // Step 6: delivery.
    let push_share = if carried_on {
        Real::ZERO
    } else {
        striking
            .eval(sim, Some(target), |attack| Some(&attack.push))
            .unwrap_or(Real::ZERO)
    };
    if !(push_share.is_finite() && push_share >= Real::ZERO && push_share <= Real::ONE) {
        return Err(bad_number("push", push_share));
    }
    let lift = striking
        .eval(sim, Some(target), |attack| Some(&attack.lift))
        .unwrap_or(Real::ZERO);
    if !lift.is_finite() {
        return Err(bad_number("lift", lift));
    }
    let harm = energy * (Real::ONE - push_share);
    let pushed = energy * push_share;
    let dir = direction(sim, striking.actor, target);
    // What was put in the way takes the rest first, and passes on what
    // it cannot stop (07 §5.3).
    let (harm, guarded, guard_returned) = through_guard(sim, striking, &interposed, harm)?;
    if !interposed.is_empty() && harm <= Real::ZERO {
        tally.absorbed += guarded;
        tally.returned += guard_returned;
        recoil(sim, striking, guard_returned, dir);
        if pushed > Real::ZERO {
            tally.pushed += pushed;
            push::push(
                sim,
                &Push {
                    target,
                    energy: pushed,
                    dir: dir.unwrap_or([Real::ZERO; 3]),
                    lift,
                    brace,
                    knock_down: None,
                },
                striking.cause,
            );
        }
        let why = Reason::new(
            defence::DEFENDED,
            "what was put in the way stopped all of it",
        )
        .with("target", target.to_string());
        stopped(sim, striking, target, &why);
        return Ok(AtTarget::Spent);
    }
    let intensity = intensity_of(&surface, harm);
    let packet = DamagePacket::new(surface.kind, harm, intensity, striking.cause);
    let walled = walled_behind(sim, target, dir);

    let (absorbed, returned, exits, landed_part) = if sim.components().body.contains(target) {
        let hit = Hit {
            target,
            at,
            packet,
            area: surface.area,
            span: surface.span,
            dir,
        };
        let outcome = sim.hit(hit)?;
        if let Some(reason) = outcome.cancelled {
            stopped(sim, striking, target, &reason);
            return Ok(AtTarget::Missed);
        }
        // A coated blade's coating (16 §9.5).
        if let super::surface::Striker::Item { item, .. } = surface.striker {
            crate::substance::after_hit(sim, item, target, &outcome, striking.cause);
        }
        let report = &outcome.report;
        let mut absorbed = report.absorbed;
        for worn in &report.worn {
            absorbed += worn.absorbed;
        }
        let landed_part = report.struck.first().map(|struck| struck.part);
        (absorbed, report.returned, outcome.exited(), landed_part)
    } else if sim.components().get::<Item>(target).is_some() {
        let hit = crate::item::damage(sim, target, &packet)?;
        match hit.outcome.result {
            DamageResult::Unaffected => (Real::ZERO, harm, Real::ZERO, None),
            _ => (hit.outcome.absorbed, Real::ZERO, hit.outcome.passed, None),
        }
    } else {
        missed(sim, striking, target, "nothing", None);
        return Ok(AtTarget::Missed);
    };
    tally.landed = true;
    tally.hits = tally.hits.saturating_add(1);
    tally.absorbed += absorbed + guarded;
    tally.returned += returned + guard_returned;
    recoil(sim, striking, returned + guard_returned, dir);

    // Step 7: push, less what a defence braced against (07 §5.1).
    if pushed > Real::ZERO {
        tally.pushed += pushed;
        let knock_down = attack.knock_down.as_ref().map(|posture| posture.id());
        push::push(
            sim,
            &Push {
                target,
                energy: pushed,
                dir: dir.unwrap_or([Real::ZERO; 3]),
                lift,
                brace,
                knock_down,
            },
            striking.cause,
        );
    }

    // Step 8: results.
    let body_defs = sim.life.body_defs().ok();
    let part_field = match (landed_part, &body_defs) {
        (Some(key), Some(body_defs)) => handle_field(part_handle(body_defs.kinds(), target, key)),
        _ => FieldValue::Null,
    };
    let kind_type = sim.exprs().registry().def_type_id("damage_type");
    let payload = striking
        .payload(STRIKE_LANDED)
        .subject("target", Subject::Entity(target))
        .with("part", part_field)
        .with("kind", id_field(kind_type, surface.kind.index()))
        .with("energy", harm)
        .with("intensity", intensity)
        .with("absorbed", absorbed)
        .with("passed", exits)
        .with("returned", returned)
        .with("push", pushed)
        .with("effort", striking.effort)
        .with("margin", margin.map_or(FieldValue::Null, FieldValue::from));
    emit(sim, striking, payload);
    run_list(
        sim,
        striking,
        Some(target),
        List::Land,
        (true, absorbed, exits),
    );
    Ok(AtTarget::Landed { exits, walled })
}

/// The intensity of a packet of so many joules from a surface (07 §6.1).
pub fn intensity_of(surface: &Surface, energy: Real) -> Real {
    let volume = surface.area * surface.give;
    if volume > Real::ZERO {
        energy / volume
    } else {
        Real::INFINITY
    }
}

/// A strike at a cell (07 §3.4): its whole energy is delivered to the
/// cell's block, else its fixture, else its floor, as a packet (02 §12);
/// what the element was not affected by comes back as recoil.
fn strike_cell(
    sim: &mut Sim,
    striking: &Striking,
    cell: Location,
    energy: Real,
) -> Result<StrikeTally, Reason> {
    let nothing = || {
        refuse(
            reason::NOTHING_THERE,
            "there is nothing in that cell to strike",
        )
        .with("cell", format!("{cell:?}"))
    };
    let layer = [Layer::Block, Layer::Fixture, Layer::Floor]
        .into_iter()
        .find(|layer| holds(sim, cell, *layer))
        .ok_or_else(nothing)?;
    let surface = striking.surface;
    let intensity = intensity_of(&surface, energy);
    let packet = DamagePacket::new(surface.kind, energy, intensity, striking.cause);
    let offset = point_of(sim, striking.actor).and_then(|(from, _)| {
        let delta = sim.truth().cosmos().delta(from, cell)?;
        Some([
            Real::from_i64(i64::from(delta.x)) * CELL_M,
            Real::from_i64(i64::from(delta.y)) * CELL_M,
            Real::from_i64(i64::from(delta.z)) * STOREY_M,
        ])
    });
    // Step 2: reach, to the cell as to anyone standing in it.
    let reach = striking
        .eval(sim, None, |attack| attack.reach.as_ref())
        .unwrap_or(striking.surface.reach);
    if !(reach.is_finite() && reach >= Real::ZERO) {
        return Err(bad_number("reach", reach));
    }
    let within = offset.is_some_and(|v| (norm(v) - CELL_M).max(Real::ZERO) <= reach);
    if !within {
        let payload = striking
            .payload(STRIKE_MISSED)
            .with("why", "reach")
            .with("margin", FieldValue::Null);
        emit(sim, striking, payload);
        return Ok(StrikeTally {
            put_in: energy,
            ..StrikeTally::default()
        });
    }
    let dir = offset.and_then(unit);
    let outcome = sim
        .world(|ctx| {
            ctx.edit(striking.cause)
                .damage_along(cell, layer, packet, dir)
        })
        .map_err(|error| Reason::from_world(&error))?;
    let mut tally = StrikeTally {
        put_in: energy,
        landed: true,
        hits: 1,
        ..StrikeTally::default()
    };
    let returned = if outcome.result == DamageResult::Unaffected {
        energy
    } else {
        Real::ZERO
    };
    tally.absorbed = outcome.absorbed;
    tally.passed = outcome.passed;
    tally.returned = returned;
    recoil(sim, striking, returned, dir);
    let kind_type = sim.exprs().registry().def_type_id("damage_type");
    let payload = striking
        .payload(STRIKE_LANDED)
        .subject("here", Subject::Loc(cell))
        .with("part", FieldValue::Null)
        .with("kind", id_field(kind_type, surface.kind.index()))
        .with("energy", energy)
        .with("intensity", intensity)
        .with("absorbed", outcome.absorbed)
        .with("passed", outcome.passed)
        .with("returned", returned)
        .with("push", Real::ZERO)
        .with("effort", striking.effort)
        .with("margin", FieldValue::Null);
    emit(sim, striking, payload);
    run_list(
        sim,
        striking,
        None,
        List::Land,
        (true, outcome.absorbed, outcome.passed),
    );
    Ok(tally)
}

/// Recoil (07 §6.3): what the target neither absorbed nor passed comes
/// back into the surface that struck.
fn recoil(sim: &mut Sim, striking: &Striking, returned: Real, dir: Option<[Real; 3]>) {
    if returned <= Real::ZERO {
        return;
    }
    let surface = striking.surface;
    let packet = DamagePacket::new(
        surface.kind,
        returned,
        intensity_of(&surface, returned),
        striking.cause,
    );
    let back = dir.map(|dir| [-dir[0], -dir[1], -dir[2]]);
    let (absorbed, part, weapon) = match surface.striker {
        Striker::Inline => return,
        Striker::Part(key) => {
            let hit = Hit {
                target: striking.actor,
                at: HitAt::Part(key),
                packet,
                area: surface.area,
                span: surface.span,
                dir: back,
            };
            match sim.hit(hit) {
                Ok(outcome) => {
                    let mut absorbed = outcome.report.absorbed;
                    for worn in &outcome.report.worn {
                        absorbed += worn.absorbed;
                    }
                    (absorbed, Some(key), None)
                }
                Err(reason) => {
                    sim.fault(reason);
                    return;
                }
            }
        }
        Striker::Item { item, .. } => match crate::item::damage(sim, item, &packet) {
            Ok(hit) => (hit.outcome.absorbed, None, Some(hit.item)),
            Err(reason) => {
                sim.fault(reason);
                return;
            }
        },
    };
    let body_defs = sim.life.body_defs().ok();
    let mut payload = striking
        .payload(STRIKE_RECOILED)
        .with(
            "part",
            match (part, &body_defs) {
                (Some(key), Some(body_defs)) => {
                    handle_field(part_handle(body_defs.kinds(), striking.actor, key))
                }
                _ => FieldValue::Null,
            },
        )
        .with("energy", returned)
        .with("absorbed", absorbed);
    if let Some(item) = weapon {
        payload = payload.subject("weapon", Subject::Entity(item));
    }
    emit(sim, striking, payload);
}

/// Raises a notification of the strike.
fn emit(sim: &mut Sim, striking: &Striking, payload: EventPayload) {
    if let Err(reason) = sim.raise(Where::Entity(striking.actor), striking.cause, payload) {
        sim.fault(reason);
    }
}

/// A miss: `strike_missed` and `on_miss`.
fn missed(sim: &mut Sim, striking: &Striking, target: EntityId, why: &str, margin: Option<Real>) {
    let payload = striking
        .payload(STRIKE_MISSED)
        .subject("target", Subject::Entity(target))
        .with("why", why)
        .with("margin", margin.map_or(FieldValue::Null, FieldValue::from));
    emit(sim, striking, payload);
    run_list(
        sim,
        striking,
        Some(target),
        List::Miss,
        (false, Real::ZERO, Real::ZERO),
    );
}

/// A strike that reached and delivered nothing: `strike_stopped` and
/// `on_stopped`.
fn stopped(sim: &mut Sim, striking: &Striking, target: EntityId, why: &Reason) {
    let payload = striking
        .payload(STRIKE_STOPPED)
        .subject("target", Subject::Entity(target))
        .with("code", why.code.as_str())
        .with("message", why.message.as_str());
    emit(sim, striking, payload);
    run_list(
        sim,
        striking,
        Some(target),
        List::Stopped,
        (false, Real::ZERO, Real::ZERO),
    );
}

/// Which of an attack's lists.
#[derive(Clone, Copy)]
enum List {
    Land,
    Miss,
    Stopped,
}

/// Runs one of the attack's lists for a target, on a copy of the
/// action's locals with what came of this target in `$landed`,
/// `$absorbed` and `$passed`.
fn run_list(
    sim: &mut Sim,
    striking: &Striking,
    target: Option<EntityId>,
    list: List,
    results: (bool, Real, Real),
) {
    let defs = Arc::clone(&striking.defs);
    let Some(action) = crate::action::run::action_def(&defs, striking.action) else {
        return;
    };
    let Some(attack) = action.attacks.0.get(striking.attack) else {
        return;
    };
    let effects = match list {
        List::Land => &attack.on_land,
        List::Miss => &attack.on_miss,
        List::Stopped => &attack.on_stopped,
    };
    if effects.is_empty() {
        return;
    }
    let mut locals = striking.locals.clone();
    let values = [
        Value::Bool(results.0),
        Value::Real(results.1),
        Value::Real(results.2),
    ];
    for ((name, _), value) in RESULT_LOCALS.iter().zip(values) {
        if let Some(slot) = action.local_slot(name)
            && let Some(local) = locals.get_mut(usize::from(slot))
        {
            *local = value;
        }
    }
    let subjects = [
        sim.subject_value(Subject::Entity(striking.actor)),
        target.map_or(Value::Null, |id| sim.subject_value(Subject::Entity(id))),
    ];
    sim.run_effects(
        effects,
        &subjects,
        &mut locals,
        Some(Subject::Entity(striking.actor)),
        striking.cause,
    );
}
