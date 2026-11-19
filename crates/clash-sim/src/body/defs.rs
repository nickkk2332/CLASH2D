//! The definition types of 06 §6.1 to §6.3: `body_part`, `body_plan` and
//! `body_function`, and [`BodyDefs`], the tables worked out from them and
//! from the world's materials once mods have loaded.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Dur, LookupMap, MICROS_PER_SECOND, Real, define_id};
use clash_data::{
    CompileCx, CrossCx, Def, DefLabel, Defs, DefsView, Diagnostic, Length, Located, Mass, Ref,
    Scope, Span, TagSet, Text, ValidateCx, code,
};
use clash_expr::{DefTypeId, KindId, PropertyId};
use clash_world::defs::{DamageTypeDef, DamageTypeId, MaterialDef, MaterialId, Resist};
use clash_world::{Footprint, WorldDefs};
use serde::Deserialize;

use super::BodyError;
use super::props::{self, Prop};
use crate::action::{ChannelDef, ChannelId};
use crate::stats::{PoolDef, PoolId};

define_id! {
    /// A `body_part` definition (06 §6.1).
    pub struct BodyPartId(u16);
    /// A `body_plan` definition (06 §6.2).
    pub struct BodyPlanId(u16);
    /// A `body_function` definition (06 §6.3).
    pub struct BodyFunctionId(u16);
}

/// The most layers one part may be built of. A wound's depth is a count
/// of them.
pub const MAX_LAYERS: usize = 16;

/// The most parts one body may have, organs included.
pub const MAX_PARTS: usize = 1024;

/// How deep parts may sit inside parts by their definitions' `inside`
/// lists. Deeper than this is what a part that contains itself looks
/// like.
pub const MAX_NESTING: usize = 8;

/// The longest a step of healing or a wound's closing is waited for, in
/// microseconds of local time. Anything slower (about 146,000 years)
/// does not happen: the timeline could not hold its due tick.
pub(super) const MAX_WAIT_MICROS: i64 = i64::MAX / 2;

/// The parts a layer's damage is counted in, as an element's is
/// (02 §12.2).
pub const DAMAGE_PARTS: u8 = 255;

fn one() -> Real {
    Real::ONE
}

fn half() -> Real {
    Real::of(0.5)
}

/// One layer of a part, as its definition writes it (06 §6.1).
#[derive(Debug, Deserialize)]
pub struct LayerDef {
    /// What the layer is made of.
    pub material: Ref<MaterialDef>,
    /// How thick it is, at scale 1.
    pub thickness: Length,
    /// How much the part's working depends on this layer, from 0 to 1.
    /// Layers that leave it out share what the others leave of 1, by
    /// thickness.
    #[serde(default)]
    pub weight: Option<Real>,
}

/// Where other parts may attach to a part (06 §6.1).
#[derive(Debug, Deserialize)]
pub struct SocketDef {
    /// How many parts it takes.
    #[serde(default = "one_slot")]
    pub count: u16,
    /// The part definitions it takes. Empty for any.
    #[serde(default)]
    pub accepts: Vec<Ref<BodyPartDef>>,
}

fn one_slot() -> u16 {
    1
}

/// Something a part can strike with (06 §6.1; 07 §3.2). The body model
/// carries it for combat and gives it no meaning.
#[derive(Debug, Deserialize)]
pub struct SurfaceDef {
    /// Its name among the part's surfaces.
    pub id: String,
    /// The damage type it delivers.
    pub kind: Ref<DamageTypeDef>,
    /// Contact area, in square metres.
    pub area: Real,
    /// The distance over which a strike with it is stopped.
    pub give: Length,
    /// The length of contact across the target.
    #[serde(default)]
    pub span: Length,
}

/// An organ that comes with a part wherever the part is used
/// (06 §6.1, `inside`).
#[derive(Debug, Deserialize)]
pub struct InsideDef {
    /// Its name within the part: in a body it is `<part>.<id>`.
    pub id: Located<String>,
    /// What it is.
    pub part: Ref<BodyPartDef>,
}

/// What a part leaves when it is severed or destroyed (06 §7.5).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Leaves {
    /// An item made of its materials.
    #[default]
    Item,
    /// Nothing.
    Nothing,
}

/// A kind of body part (06 §6.1): what it is built of, what it supplies
/// and where others attach to it.
#[derive(Debug, Deserialize)]
pub struct BodyPartDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// Its mass at scale 1.
    pub mass: Mass,
    /// How long it is at scale 1.
    pub length: Length,
    /// How far across it is at scale 1. A cut whose span is at least
    /// this goes all the way through.
    pub girth: Length,
    /// What it is built of, outermost first.
    pub layers: Vec<LayerDef>,
    /// What it contributes to the body's functions, when in perfect
    /// condition (06 §6.3).
    #[serde(default)]
    pub provides: BTreeMap<Ref<BodyFunctionDef>, Real>,
    /// The channel slots it supplies while it works (03 §6.2).
    #[serde(default)]
    pub channels: BTreeMap<Ref<ChannelDef>, u16>,
    /// Where other parts may attach, by socket name.
    #[serde(default)]
    pub sockets: BTreeMap<String, SocketDef>,
    /// What it can strike with.
    #[serde(default)]
    pub surfaces: Vec<SurfaceDef>,
    /// The share of the body's height it occupies, from the ground up:
    /// `[low, high]`, each from 0 to 1.
    #[serde(default)]
    pub band: Option<[Real; 2]>,
    /// Free-form tags.
    #[serde(default)]
    pub tags: TagSet,
    /// How likely it is to be struck by an unaimed hit, relative to its
    /// size. Default 1.
    #[serde(default = "one")]
    pub exposure: Real,
    /// The condition below which it stops working. Default 0.5.
    #[serde(default = "half")]
    pub disable_below: Real,
    /// Organs that come with it wherever it is used.
    #[serde(default)]
    pub inside: Vec<InsideDef>,
    /// What it leaves when it is severed or destroyed.
    #[serde(default)]
    pub leaves: Leaves,
    /// Each layer's weight, those left out filled in. Worked out when
    /// the definition is checked for the first time it is read.
    #[serde(skip)]
    weights: Vec<Real>,
}

impl BodyPartDef {
    /// How much the part's working depends on each layer, outermost
    /// first: what was written, and for layers that left it out their
    /// share, by thickness, of what the others leave of 1.
    pub fn weights(&self) -> &[Real] {
        &self.weights
    }

    /// The area the part presents at scale 1, in square metres: its
    /// silhouette, `length × girth`.
    pub fn area(&self) -> Real {
        self.length.get() * self.girth.get()
    }

    /// The channels the part supplies slots of.
    pub fn channel_set(&self) -> crate::action::ChannelSet {
        self.channels.keys().map(|channel| channel.id()).collect()
    }

    fn work_out_weights(&mut self) {
        let given: Real = self
            .layers
            .iter()
            .filter_map(|layer| layer.weight)
            .fold(Real::ZERO, |sum, weight| sum + weight);
        let left = (Real::ONE - given).max(Real::ZERO);
        let unweighted: Real = self
            .layers
            .iter()
            .filter(|layer| layer.weight.is_none())
            .fold(Real::ZERO, |sum, layer| sum + layer.thickness.get());
        self.weights = self
            .layers
            .iter()
            .map(|layer| match layer.weight {
                Some(weight) => weight,
                None if unweighted > Real::ZERO => left * layer.thickness.get() / unweighted,
                None => Real::ZERO,
            })
            .collect();
    }
}

/// Whether a number is finite and at least zero.
fn amount(value: Real) -> bool {
    value.is_finite() && value >= Real::ZERO
}

/// Whether a number is finite and above zero.
fn positive(value: Real) -> bool {
    value.is_finite() && value > Real::ZERO
}

impl Def for BodyPartDef {
    type Id = BodyPartId;
    const TYPE: &'static str = "body_part";
    const DOC: &'static str =
        "A kind of body part: layers of material, what it supplies, where others attach (06 §6.1).";

    fn finish(&mut self, _cx: &mut clash_data::FinishCx<'_>) {
        self.work_out_weights();
    }

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !amount(self.mass.get()) {
            cx.error("`mass` must be a finite amount of zero or more");
        }
        if !positive(self.length.get()) {
            cx.error("`length` must be above zero");
        }
        if !positive(self.girth.get()) {
            cx.error("`girth` must be above zero");
        }
        if self.layers.is_empty() {
            cx.error("a part is built of at least one layer");
        }
        if self.layers.len() > MAX_LAYERS {
            cx.error(format!(
                "a part has at most {MAX_LAYERS} layers, and this has {}",
                self.layers.len()
            ));
        }
        let mut given = Real::ZERO;
        for (index, layer) in self.layers.iter().enumerate() {
            if !positive(layer.thickness.get()) {
                cx.error(format!("layer {index}: `thickness` must be above zero"));
            }
            if let Some(weight) = layer.weight {
                if !(Real::ZERO..=Real::ONE).contains(&weight) {
                    cx.error(format!("layer {index}: `weight` is from 0 to 1"));
                }
                given += weight;
            }
        }
        // A little room for sums such as 0.05 + 0.5 + 0.45.
        if given > Real::ONE + Real::of(1.0e-9) {
            cx.error(format!(
                "the layers' weights add up to {given}, which is more than 1"
            ));
        }
        for amount_provided in self.provides.values() {
            if !amount(*amount_provided) {
                cx.error("what a part `provides` must be a finite amount of zero or more");
            }
        }
        if self.channels.values().any(|count| *count == 0) {
            cx.error("a part supplies at least one slot of each channel it lists");
        }
        for (name, socket) in &self.sockets {
            if socket.count == 0 {
                cx.error(format!("socket `{name}`: `count` must be at least 1"));
            }
        }
        for (index, surface) in self.surfaces.iter().enumerate() {
            if !positive(surface.area) || !positive(surface.give.get()) {
                cx.error(format!(
                    "surface `{}`: `area` and `give` must be above zero",
                    surface.id
                ));
            }
            if !amount(surface.span.get()) {
                cx.error(format!(
                    "surface `{}`: `span` must not be negative",
                    surface.id
                ));
            }
            if self.surfaces[..index]
                .iter()
                .any(|other| other.id == surface.id)
            {
                cx.error(format!("the surface `{}` is listed twice", surface.id));
            }
        }
        if let Some([low, high]) = self.band
            && !(Real::ZERO <= low && low <= high && high <= Real::ONE)
        {
            cx.error("`band` is `[low, high]`, shares of the body's height from 0 to 1");
        }
        if !amount(self.exposure) {
            cx.error("`exposure` must be a finite amount of zero or more");
        }
        if !(Real::ZERO..=Real::ONE).contains(&self.disable_below) {
            cx.error("`disable_below` is a condition, from 0 to 1");
        }
        for (index, organ) in self.inside.iter().enumerate() {
            if self.inside[..index]
                .iter()
                .any(|other| other.id.value == organ.id.value)
            {
                cx.error_at(
                    organ.id.span,
                    format!("`{}` is listed inside this part twice", organ.id.value),
                );
            }
        }
        // The pools a tissue names are text in the world's definition,
        // which cannot name a pool: they are checked where they are used.
        let view = cx.defs();
        for (index, layer) in self.layers.iter().enumerate() {
            let material = layer.material.id();
            let (Some(def), Some(name)) = (
                view.get::<MaterialDef>(material),
                view.name::<MaterialDef>(material),
            ) else {
                continue;
            };
            let pools = [
                ("leak", def.leak.as_ref().map(|leak| leak.pool.as_str())),
                (
                    "signal",
                    def.signal.as_ref().map(|signal| signal.pool.as_str()),
                ),
            ];
            for (extra, pool) in pools {
                if let Some(pool) = pool
                    && pool_in_view(view, name, pool).is_none()
                {
                    cx.error(format!(
                        "layer {index}: the material `{name}` names the pool `{pool}` in its \
                         `{extra}`, and there is no such pool"
                    ));
                }
            }
        }
    }
}

/// One part of a plan (06 §6.2).
#[derive(Debug, Deserialize)]
pub struct PlanPartDef {
    /// The part's name within a body of this plan.
    pub id: Located<String>,
    /// What it is.
    pub part: Ref<BodyPartDef>,
    /// The part it is attached to from outside: lose that and this goes
    /// too.
    #[serde(default)]
    pub on: Option<Located<String>>,
    /// The part it sits inside: it is reached only through that part's
    /// layers.
    #[serde(default, rename = "in")]
    pub inside: Option<Located<String>>,
    /// The socket of the part it is `on` that it takes.
    #[serde(default)]
    pub socket: Option<Located<String>>,
    /// A label telling this part from its twin: `"left"`. Free text.
    #[serde(default)]
    pub side: Option<String>,
    /// What a reader calls this part of a body of the plan: `left arm`.
    /// Left out, its side and its definition's name, or the definition's
    /// name alone (decision D-863).
    #[serde(default)]
    pub name: Option<String>,
    /// Its size relative to the rest of the body. Default 1.
    #[serde(default = "one")]
    pub scale: Real,
}

/// A body plan (06 §6.2): the parts a body starts with and how they are
/// joined. An instance owns its own tree afterwards.
#[derive(Debug, Deserialize)]
pub struct BodyPlanDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// How tall a body of this plan stands at scale 1.
    #[serde(default)]
    pub height: Length,
    /// The cells a body of this plan fills, along x, y and storeys.
    /// Default one cell.
    #[serde(default)]
    pub footprint: Option<[u8; 3]>,
    /// The parts. Exactly one is the root, attached to nothing.
    pub parts: Vec<PlanPartDef>,
    /// The functions a body of this plan cannot do without, and what
    /// having none of one does to an owner that has a mind (06 §8.3):
    /// `unconscious` or `dead`.
    #[serde(default)]
    pub needs: BTreeMap<Ref<BodyFunctionDef>, crate::mind::Stop>,
}

impl BodyPlanDef {
    /// The cells a body of this plan fills.
    pub fn footprint(&self) -> Footprint {
        self.footprint
            .map_or(Footprint::SINGLE, |[x, y, z]| Footprint::new(x, y, z))
    }
}

impl Def for BodyPlanDef {
    type Id = BodyPlanId;
    const TYPE: &'static str = "body_plan";
    const DOC: &'static str = "The parts a body starts with and how they are joined (06 §6.2).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !amount(self.height.get()) {
            cx.error("`height` must be a finite amount of zero or more");
        }
        for part in &self.parts {
            if !positive(part.scale) {
                cx.error_at(part.id.span, "a part's `scale` must be above zero");
            }
        }
        let view = cx.defs();
        if let Err(problem) = lay_out(self, |id| view.get::<BodyPartDef>(id)) {
            match problem.span {
                Some(span) => cx.error_at(span, problem.message),
                None => cx.error(problem.message),
            }
        }
    }
}

/// How a function's capacity is made from what the parts contribute
/// (06 §6.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Combine {
    /// The contributions added up.
    #[default]
    Sum,
    /// The least of them: every contributor is needed.
    Min,
    /// The greatest of them: any one will do.
    Max,
    /// Their average.
    Mean,
}

/// Something a body as a whole can do to some degree (06 §6.3).
#[derive(Debug, Default, Deserialize)]
pub struct BodyFunctionDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// How the parts' contributions are combined. Default `sum`.
    #[serde(default)]
    pub combine: Combine,
}

impl Def for BodyFunctionDef {
    type Id = BodyFunctionId;
    const TYPE: &'static str = "body_function";
    const DOC: &'static str = "Something a body as a whole can do to some degree, from what its working parts contribute (06 §6.3).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
    }

    fn placeholder() -> Option<BodyFunctionDef> {
        Some(BodyFunctionDef::default())
    }
}

/// How a part is joined to its parent (06 §6.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// Attached from outside: lose the parent and it goes too.
    On,
    /// Inside: reached only through the parent's layers.
    In,
}

impl clash_core::StateHash for Relation {
    fn state_hash(&self, hasher: &mut clash_core::StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// One part of a plan laid out: where it goes in a body made from the
/// plan. Its position in [`PlanLayout::parts`] is the key it gets.
#[derive(Clone, PartialEq, Debug)]
pub struct PlannedPart {
    /// Its name in the body.
    pub name: String,
    /// What it is.
    pub def: BodyPartId,
    /// The position of its parent, and how it is joined to it. `None`
    /// for the root.
    pub parent: Option<(usize, Relation)>,
    /// The socket of the parent it takes.
    pub socket: Option<String>,
    /// Its side label.
    pub side: Option<String>,
    /// Its size relative to the rest of the body.
    pub scale: Real,
}

/// A plan laid out: every part a body made from it starts with, the
/// plan's own in the order written and then the organs their
/// definitions bring.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PlanLayout {
    /// The parts. The root is the one with no parent.
    pub parts: Vec<PlannedPart>,
}

/// Why a plan cannot be laid out.
pub(super) struct PlanProblem {
    span: Option<Span>,
    message: String,
}

fn problem<T>(span: Option<Span>, message: impl Into<String>) -> Result<T, PlanProblem> {
    Err(PlanProblem {
        span,
        message: message.into(),
    })
}

/// Lays a plan out, or says what is wrong with it.
pub(super) fn lay_out<'a>(
    plan: &BodyPlanDef,
    part_def: impl Fn(BodyPartId) -> Option<&'a BodyPartDef>,
) -> Result<PlanLayout, PlanProblem> {
    if plan.parts.is_empty() {
        return problem(None, "a plan has at least one part");
    }
    let mut parts: Vec<PlannedPart> = Vec::with_capacity(plan.parts.len());
    for (index, entry) in plan.parts.iter().enumerate() {
        if plan.parts[..index]
            .iter()
            .any(|other| other.id.value == entry.id.value)
        {
            return problem(
                Some(entry.id.span),
                format!("the part `{}` is listed twice", entry.id.value),
            );
        }
        if let (Some(_), Some(inside)) = (&entry.on, &entry.inside) {
            return problem(
                Some(inside.span),
                format!(
                    "the part `{}` is both `on` a part and `in` one",
                    entry.id.value
                ),
            );
        }
        parts.push(PlannedPart {
            name: entry.id.value.clone(),
            def: entry.part.id(),
            parent: None,
            socket: entry.socket.as_ref().map(|socket| socket.value.clone()),
            side: entry.side.clone(),
            scale: entry.scale,
        });
    }
    // Parents, by name.
    let mut root: Option<usize> = None;
    for (index, entry) in plan.parts.iter().enumerate() {
        let joined = match (&entry.on, &entry.inside) {
            (Some(on), _) => Some((on, Relation::On)),
            (None, Some(inside)) => Some((inside, Relation::In)),
            (None, None) => None,
        };
        let Some((parent, relation)) = joined else {
            if let Some(first) = root {
                return problem(
                    Some(entry.id.span),
                    format!(
                        "`{}` and `{}` are both attached to nothing: a plan has one root",
                        plan.parts[first].id.value, entry.id.value
                    ),
                );
            }
            if let Some(socket) = &entry.socket {
                return problem(Some(socket.span), "the root takes no socket");
            }
            root = Some(index);
            continue;
        };
        let Some(at) = plan
            .parts
            .iter()
            .position(|other| other.id.value == parent.value)
        else {
            return problem(
                Some(parent.span),
                format!("there is no part `{}` in this plan", parent.value),
            );
        };
        if at == index {
            return problem(Some(parent.span), "a part cannot be joined to itself");
        }
        parts[index].parent = Some((at, relation));
        if let Some(socket) = &entry.socket {
            if relation == Relation::In {
                return problem(Some(socket.span), "a part `in` another takes no socket");
            }
            let Some(parent_def) = part_def(parts[at].def) else {
                continue;
            };
            let Some(socket_def) = parent_def.sockets.get(&socket.value) else {
                return problem(
                    Some(socket.span),
                    format!(
                        "the part `{}` has no socket `{}`",
                        parent.value, socket.value
                    ),
                );
            };
            if !socket_def.accepts.is_empty()
                && !socket_def
                    .accepts
                    .iter()
                    .any(|accepted| accepted.id() == entry.part.id())
            {
                return problem(
                    Some(socket.span),
                    format!(
                        "the socket `{}` of `{}` does not accept this kind of part",
                        socket.value, parent.value
                    ),
                );
            }
            let taken = plan.parts[..index]
                .iter()
                .enumerate()
                .filter(|(other, _)| parts[*other].parent.map(|(parent, _)| parent) == Some(at))
                .filter(|(_, other)| {
                    other.socket.as_ref().map(|socket| &socket.value) == Some(&socket.value)
                })
                .count();
            if taken >= usize::from(socket_def.count) {
                return problem(
                    Some(socket.span),
                    format!(
                        "the socket `{}` of `{}` takes {} and is full",
                        socket.value, parent.value, socket_def.count
                    ),
                );
            }
        }
    }
    if root.is_none() {
        return problem(
            None,
            "every part is attached to another: a plan has one root, attached to nothing",
        );
    }
    // No part may be its own ancestor.
    for (index, entry) in plan.parts.iter().enumerate() {
        let mut at = index;
        for _ in 0..=plan.parts.len() {
            match parts[at].parent {
                Some((parent, _)) => at = parent,
                None => break,
            }
        }
        if parts[at].parent.is_some() {
            return problem(
                Some(entry.id.span),
                format!(
                    "the part `{}` is attached to itself, round in a circle",
                    entry.id.value
                ),
            );
        }
    }
    // The organs the parts' definitions bring, after the plan's own.
    let mut next = 0;
    let mut depth: Vec<usize> = vec![0; parts.len()];
    while next < parts.len() {
        let Some(def) = part_def(parts[next].def) else {
            next += 1;
            continue;
        };
        for organ in &def.inside {
            if depth[next] >= MAX_NESTING {
                return problem(
                    Some(organ.id.span),
                    format!(
                        "parts sit inside parts more than {MAX_NESTING} deep by their `inside` \
                         lists: one contains itself"
                    ),
                );
            }
            let name = format!("{}.{}", parts[next].name, organ.id.value);
            if parts.iter().any(|other| other.name == name) {
                return problem(
                    Some(organ.id.span),
                    format!("the plan lists `{name}`, which the part brings with it"),
                );
            }
            parts.push(PlannedPart {
                name,
                def: organ.part.id(),
                parent: Some((next, Relation::In)),
                socket: None,
                side: parts[next].side.clone(),
                scale: parts[next].scale,
            });
            depth.push(depth[next] + 1);
            if parts.len() > MAX_PARTS {
                return problem(None, format!("a body has at most {MAX_PARTS} parts"));
            }
        }
        next += 1;
    }
    if parts.len() > MAX_PARTS {
        return problem(None, format!("a body has at most {MAX_PARTS} parts"));
    }
    Ok(PlanLayout { parts })
}

/// The namespace of a qualified id.
fn namespace_of(qualified: &str) -> Option<&str> {
    qualified.split_once(':').map(|(namespace, _)| namespace)
}

/// The pool a tissue names, among the names given: a qualified id
/// exactly; a bare one in the namespace of the material that names it,
/// then in every namespace in sorted order (as the world resolves its
/// own references, decision D-283).
pub(crate) fn pool_by_name<'a>(
    names: impl Iterator<Item = &'a str> + Clone,
    material: &str,
    pool: &str,
) -> Option<&'a str> {
    if pool.contains(':') {
        return names.clone().find(|name| *name == pool);
    }
    if let Some(namespace) = namespace_of(material)
        && let Some(found) = names
            .clone()
            .find(|name| name.split_once(':') == Some((namespace, pool)))
    {
        return Some(found);
    }
    let mut matches: Vec<&str> = names
        .filter(|name| name.split_once(':').is_some_and(|(_, bare)| bare == pool))
        .collect();
    matches.sort_unstable();
    matches.first().copied()
}

/// The pool a tissue of `material` names, while definitions load.
fn pool_in_view(view: &DefsView<'_>, material: &str, pool: &str) -> Option<PoolId> {
    let names: Vec<&str> = view
        .iter::<PoolDef>()
        .filter_map(|(id, _)| view.name::<PoolDef>(id))
        .collect();
    let found = pool_by_name(names.iter().copied(), material, pool)?;
    view.lookup::<PoolDef>(found)
}

/// The whole-set check of the bodies: a tissue's pools must exist, for
/// every material, whether a part uses it yet or not. The same mistake
/// in a material a part is built of is also reported at the part, which
/// has a position (see [`BodyPartDef`]'s own checks).
pub(super) fn check_tissues(cx: &mut CrossCx<'_>) {
    let view = cx.defs();
    for (id, def) in view.iter::<MaterialDef>() {
        let Some(name) = view.name::<MaterialDef>(id) else {
            continue;
        };
        let label = DefLabel::new(MaterialDef::TYPE, name);
        let mut report = |path: &str, message: String| {
            cx.report(
                Diagnostic::new(code::VALIDATION, message)
                    .in_def(Some(&label))
                    .with_path(path),
            );
        };
        if let Some(leak) = &def.leak {
            if pool_in_view(view, name, leak.pool.as_str()).is_none() {
                report(
                    "leak.pool",
                    format!("there is no pool `{}`", leak.pool.as_str()),
                );
            }
            if !amount(leak.rate) {
                report(
                    "leak.rate",
                    "`rate` must be a finite amount of zero or more".to_owned(),
                );
            }
            if leak.closes.is_some_and(|closes| !amount(closes.get())) {
                report(
                    "leak.closes",
                    "`closes` must be a time of zero or more".to_owned(),
                );
            }
        }
        if let Some(signal) = &def.signal {
            if pool_in_view(view, name, signal.pool.as_str()).is_none() {
                report(
                    "signal.pool",
                    format!("there is no pool `{}`", signal.pool.as_str()),
                );
            }
            if !signal.per_joule.is_finite() {
                report("signal.per_joule", "`per_joule` must be finite".to_owned());
            }
        }
        if def.heal.is_some_and(|heal| !amount(heal.get())) {
            report("heal", "`heal` must be a rate of zero or more".to_owned());
        }
    }
}

/// What a breached layer drains (06 §6.1), resolved.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Leak {
    /// The pool drained.
    pub pool: PoolId,
    /// What is drained each second for each square metre of wound.
    pub rate: Real,
    /// How long a breach leaks before it closes by itself. `None`: until
    /// something closes it.
    pub closes: Option<Dur>,
}

/// What a damaged layer adds to a pool (06 §6.1), resolved.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Signal {
    /// The pool added to.
    pub pool: PoolId,
    /// What is added for each joule the layer absorbed.
    pub per_joule: Real,
}

/// A material as the body model reads it: the world's numbers and the
/// tissue extras of 06 §6.1.
#[derive(Clone, PartialEq, Debug)]
pub struct Tissue {
    /// Energy to destroy one cubic metre, in J/m³.
    pub toughness: Real,
    /// Mass per volume, in kg/m³.
    pub density: Real,
    /// What a breached layer of it drains.
    pub leak: Option<Leak>,
    /// What a damaged layer of it adds to a pool.
    pub signal: Option<Signal>,
    /// The local time in which one 255th of a layer of it recovers.
    /// `None`: it does not recover unaided.
    pub heal_step: Option<Dur>,
    /// Its resistance to each damage type, by dense id.
    resist: Vec<Resist>,
}

impl Tissue {
    /// How it resists a damage type. Something unknown resists nothing.
    pub fn resist(&self, kind: DamageTypeId) -> Resist {
        self.resist
            .get(kind.index())
            .copied()
            .unwrap_or(Resist::NONE)
    }
}

/// A time in seconds as whole microseconds, rounded up, if the timeline
/// can hold it.
fn micros(seconds: Real) -> Option<Dur> {
    let micros = (seconds * Real::from_i64(MICROS_PER_SECOND)).ceil();
    if !micros.is_finite() || micros < Real::ZERO || micros > Real::from_i64(MAX_WAIT_MICROS) {
        return None;
    }
    Some(Dur(micros.to_i64()))
}

/// The kinds of subject this module registers, and the kind `entity`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BodyKinds {
    /// `entity`: what has a body.
    pub entity: KindId,
    /// `functions`: the body functions of one entity, reached as
    /// `entity.fn`.
    pub functions: KindId,
    /// `part`: one part of a body.
    pub part: KindId,
    /// `wound`: one wound on a part.
    pub wound: KindId,
}

/// The definition types of this module, and those it hands out ids of,
/// as the expression registry numbers them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BodyDefTypes {
    /// `body_part`.
    pub body_part: DefTypeId,
    /// `body_plan`.
    pub body_plan: DefTypeId,
    /// `body_function`.
    pub body_function: DefTypeId,
    /// `material`, if the world's types are registered.
    pub material: Option<DefTypeId>,
    /// `damage_type`, if the world's types are registered.
    pub damage_type: Option<DefTypeId>,
}

/// What the functions of [`body`](super) need of a set of loaded
/// definitions: the three registries, every plan laid out, the tissues,
/// and which registered property is which.
///
/// Built once after loading with [`BodyDefs::new`], and immutable.
#[derive(Debug)]
pub struct BodyDefs {
    defs: Arc<Defs>,
    world: Arc<WorldDefs>,
    kinds: BodyKinds,
    def_types: BodyDefTypes,
    props: LookupMap<PropertyId, Prop>,
    /// Each plan laid out, by plan id. `None` for one that cannot be,
    /// which the loader has already refused.
    layouts: Vec<Option<PlanLayout>>,
    /// Each material as tissue, by material id.
    tissues: Vec<Tissue>,
    /// Whether each damage type severs, by damage type id.
    severs: Vec<bool>,
}

impl BodyDefs {
    /// A damage type in plain words, for a reader (decision D-863): its
    /// bare id with spaces for underscores.
    pub fn damage_type_words(&self, id: DamageTypeId) -> String {
        let types = self.world.damage_types();
        if id.index() >= types.len() {
            return String::new();
        }
        let qualified = types.name(id);
        qualified
            .rsplit(':')
            .next()
            .unwrap_or(qualified)
            .replace('_', " ")
    }

    /// The tables for a set of loaded definitions and the world's tables
    /// built from the same definitions.
    ///
    /// Fails if the definitions were loaded without this crate's
    /// `register_defs` or the world's.
    pub fn new(defs: &Arc<Defs>, world: &Arc<WorldDefs>) -> Result<BodyDefs, BodyError> {
        let missing = |what: &'static str| BodyError::NotRegistered { what };
        if defs.try_registry::<BodyPartDef>().is_none() {
            return Err(missing("the definition type `body_part`"));
        }
        if defs.try_registry::<BodyPlanDef>().is_none() {
            return Err(missing("the definition type `body_plan`"));
        }
        if defs.try_registry::<BodyFunctionDef>().is_none() {
            return Err(missing("the definition type `body_function`"));
        }
        let Some(materials) = defs.try_registry::<MaterialDef>() else {
            return Err(missing("the definition type `material`"));
        };
        let registry = defs.expr_registry();
        let kind = |name: &'static str| registry.kind_id(name).ok_or(missing(name));
        let kinds = BodyKinds {
            entity: kind(props::ENTITY_KIND)?,
            functions: kind(props::FUNCTIONS_KIND)?,
            part: kind(props::PART_KIND)?,
            wound: kind(props::WOUND_KIND)?,
        };
        let def_type = |name: &'static str| registry.def_type_id(name).ok_or(missing(name));
        let def_types = BodyDefTypes {
            body_part: def_type(BodyPartDef::TYPE)?,
            body_plan: def_type(BodyPlanDef::TYPE)?,
            body_function: def_type(BodyFunctionDef::TYPE)?,
            material: registry.def_type_id(MaterialDef::TYPE),
            damage_type: registry.def_type_id(DamageTypeDef::TYPE),
        };
        let props = props::table(registry, kinds);

        let part_defs = defs.registry::<BodyPartDef>();
        let layouts = defs
            .registry::<BodyPlanDef>()
            .iter()
            .map(|(_, plan)| lay_out(plan, |id| part_defs.try_get(id)).ok())
            .collect();

        let pools = defs.try_registry::<PoolDef>();
        let pool_names: Vec<&str> = pools
            .map(|pools| pools.names().iter().map(String::as_str).collect())
            .unwrap_or_default();
        let pool = |material: &str, name: &str| -> Option<PoolId> {
            let found = pool_by_name(pool_names.iter().copied(), material, name)?;
            pools?.lookup(found)
        };
        let tissues = materials
            .iter()
            .map(|(id, def)| {
                let name = materials.name(id);
                let built = world.materials().get(id);
                Tissue {
                    toughness: built.map_or(def.toughness.0, |material| material.toughness),
                    density: built.map_or(def.density.0, |material| material.density),
                    leak: def.leak.as_ref().and_then(|leak| {
                        Some(Leak {
                            pool: pool(name, leak.pool.as_str())?,
                            rate: leak.rate.max(Real::ZERO),
                            closes: match leak.closes {
                                Some(closes) => Some(micros(closes.get())?),
                                None => None,
                            },
                        })
                    }),
                    signal: def.signal.as_ref().and_then(|signal| {
                        Some(Signal {
                            pool: pool(name, signal.pool.as_str())?,
                            per_joule: signal.per_joule,
                        })
                    }),
                    heal_step: def
                        .heal
                        .filter(|heal| heal.get() > Real::ZERO)
                        .and_then(|heal| {
                            micros(
                                Real::ONE / (Real::from_i64(i64::from(DAMAGE_PARTS)) * heal.get()),
                            )
                        })
                        .map(|step| step.max(Dur::MICROSECOND)),
                    resist: built
                        .map(|material| material.resist.clone())
                        .unwrap_or_default(),
                }
            })
            .collect();
        let severs = defs
            .try_registry::<DamageTypeDef>()
            .map(|types| types.iter().map(|(_, def)| def.severs).collect())
            .unwrap_or_default();
        Ok(BodyDefs {
            defs: Arc::clone(defs),
            world: Arc::clone(world),
            kinds,
            def_types,
            props,
            layouts,
            tissues,
            severs,
        })
    }

    /// The definitions these tables were made from.
    pub fn defs(&self) -> &Arc<Defs> {
        &self.defs
    }

    /// The world's tables.
    pub fn world(&self) -> &Arc<WorldDefs> {
        &self.world
    }

    /// The kinds of subject bodies are read through.
    pub fn kinds(&self) -> BodyKinds {
        self.kinds
    }

    /// The definition types as the expression registry numbers them,
    /// for making an id value.
    pub fn def_types(&self) -> BodyDefTypes {
        self.def_types
    }

    /// A part's definition, if the id is one of these definitions'.
    pub fn part(&self, id: BodyPartId) -> Option<&BodyPartDef> {
        self.defs.registry::<BodyPartDef>().try_get(id)
    }

    /// A plan's definition, if the id is one of these definitions'.
    pub fn plan(&self, id: BodyPlanId) -> Option<&BodyPlanDef> {
        self.defs.registry::<BodyPlanDef>().try_get(id)
    }

    /// A function's definition, if the id is one of these definitions'.
    pub fn function(&self, id: BodyFunctionId) -> Option<&BodyFunctionDef> {
        self.defs.registry::<BodyFunctionDef>().try_get(id)
    }

    /// How many functions are defined.
    pub fn function_count(&self) -> usize {
        self.defs.registry::<BodyFunctionDef>().len()
    }

    /// The part definition with this qualified id.
    pub fn part_id(&self, qualified: &str) -> Option<BodyPartId> {
        self.defs.lookup::<BodyPartDef>(qualified)
    }

    /// The plan with this qualified id.
    pub fn plan_id(&self, qualified: &str) -> Option<BodyPlanId> {
        self.defs.lookup::<BodyPlanDef>(qualified)
    }

    /// The function with this qualified id.
    pub fn function_id(&self, qualified: &str) -> Option<BodyFunctionId> {
        self.defs.lookup::<BodyFunctionDef>(qualified)
    }

    /// A plan laid out: the parts a body made from it starts with.
    pub fn layout(&self, plan: BodyPlanId) -> Option<&PlanLayout> {
        self.layouts.get(plan.index())?.as_ref()
    }

    /// A material as tissue. `None` for an id that names none.
    pub fn tissue(&self, material: MaterialId) -> Option<&Tissue> {
        self.tissues.get(material.index())
    }

    /// Whether a damage type takes off a part it goes all the way
    /// through (06 §7.2).
    pub fn severs(&self, kind: DamageTypeId) -> bool {
        self.severs.get(kind.index()).copied().unwrap_or(false)
    }

    /// The slots of each channel a part definition supplies, in order of
    /// channel.
    pub fn channels_of(&self, part: BodyPartId) -> impl Iterator<Item = (ChannelId, u16)> + '_ {
        self.part(part).into_iter().flat_map(|def| {
            def.channels
                .iter()
                .map(|(channel, count)| (channel.id(), *count))
        })
    }

    /// What a registered property is, if it is one of this module's.
    pub(super) fn prop(&self, property: PropertyId) -> Option<Prop> {
        self.props.get(&property).copied()
    }

    /// Whether a registered property is one this module answers.
    pub fn answers(&self, property: PropertyId) -> bool {
        self.props.contains_key(&property)
    }
}
