//! The cursor over the map: what the `camera` effect does, what a click
//! on a view picks, and what a screen is told about the place under the
//! cursor (12 §6.3, §9.3, §9.4).
//!
//! The pickers themselves are screens, in data. What a screen cannot do
//! by itself is here, and it is small: move the camera's cursor, cycle
//! it through what could be chosen, and say what the party knows of the
//! place it is on.
//!
//! # What a screen is told
//!
//! A screen that declares any of these variables has them kept up to
//! date while it is on top. A picker shows them; so does look mode.
//!
//! | Variable | Holds |
//! |----------|-------|
//! | `cursor_sight` | What the view model knows of the cell under the cursor: `seen`, `remembered` or `unknown` |
//! | `cursor_distance` | How far the cursor is from the followed character, in metres |
//! | `cursor_target` | What the reader calls the perceived thing under the cursor; empty if there is none |
//! | `cursor_reach` | While a parameter is asked for: `yes` if the action's `requires` holds with the place under the cursor as that parameter, as the actor believes; `no` if it does not; empty if there is nothing to say |
//! | `cursor_why` | With `cursor_reach` at `no`: the action's `fail_text` |
//! | `cursor_advice` | What the parameter's `check` says of the place under the cursor; empty if it is sound |
//! | `cursor_index`, `cursor_count` | Which of the things that could be chosen the cursor is on, from 1, or 0; and how many there are |
//! | `picker_entries` | For a list parameter: how many entries it has so far |
//!
//! Everything here comes from the view model and from expressions
//! evaluated in the actor's mind. Nothing reads the world.

use clash_core::{Real, Value};
use clash_expr::{Type, Value as ExprValue};
use clash_sim::action::ActionDef;
use clash_world::dims::CELL_M;
use clash_world::{CellPos, Location};

use crate::control::{Ask, Collecting};
use crate::frame::Rect;
use crate::scene::{Scene, SceneHost, UiCommand, UiOutcome};
use crate::ui::{CameraEffect, UiEffects, ViewKind};

/// The control a click on a map view runs where the view has no
/// `on_pick` of its own (12 §9.4).
pub const CONTROL_CLICK: &str = "click";
/// The control a click with the second mouse button on a map view runs.
pub const CONTROL_CLICK_SECOND: &str = "click_second";

/// `sight`: the cell is in view now.
pub const SIGHT_SEEN: &str = "seen";
/// `sight`: the cell was seen and is remembered.
pub const SIGHT_REMEMBERED: &str = "remembered";
/// `sight`: the party knows nothing of the cell.
pub const SIGHT_UNKNOWN: &str = "unknown";
/// `reach`: the action could be done with this choice.
pub const REACH_YES: &str = "yes";
/// `reach`: it could not.
pub const REACH_NO: &str = "no";

/// The variables a screen may declare to be told about the cursor.
pub const STATUS_VARS: [&str; 9] = [
    "cursor_sight",
    "cursor_distance",
    "cursor_target",
    "cursor_reach",
    "cursor_why",
    "cursor_advice",
    "cursor_index",
    "cursor_count",
    "picker_entries",
];

/// What the scene knows of the place under the cursor.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CursorStatus {
    sight: &'static str,
    distance: f64,
    target: String,
    reach: &'static str,
    why: String,
    advice: String,
    index: usize,
    count: usize,
    entries: usize,
}

impl CursorStatus {
    fn vars(&self) -> [(&'static str, Value); 9] {
        let text = |text: &str| Value::Str(text.to_owned());
        let count = |n: usize| Value::Int(n as i64);
        // Tenths of a metre are as fine as a cursor on cells gets.
        let distance = (self.distance * 10.0).round() / 10.0;
        let values = [
            text(self.sight),
            Value::Real(Real::new(distance).unwrap_or(Real::ZERO)),
            text(&self.target),
            text(self.reach),
            text(&self.why),
            text(&self.advice),
            count(self.index),
            count(self.count),
            count(self.entries),
        ];
        let mut named = values.into_iter();
        STATUS_VARS.map(|name| (name, named.next().unwrap_or(Value::Bool(false))))
    }
}

/// The kind of subject a type is, by name.
fn kind_of(ty: &Type, registry: &clash_expr::Registry) -> Option<String> {
    match ty.non_null() {
        Type::Subject(kind) => registry.kind(*kind).map(|kind| kind.spec.name.clone()),
        _ => None,
    }
}

impl<H: SceneHost> Scene<H> {
    /// Tells the top screen about the place under the cursor, in the
    /// variables it declares for that ([`STATUS_VARS`]).
    pub(crate) fn push_status(&mut self) {
        let wants = self
            .screens
            .last()
            .is_some_and(|top| STATUS_VARS.iter().any(|name| top.has_var(name)));
        if !wants {
            return;
        }
        let status = self.cursor_status();
        let Some(top) = self.screens.last_mut() else {
            return;
        };
        for (name, value) in status.vars() {
            if top.has_var(name) && top.var(name) != Some(&value) {
                top.set_var(name, value);
                self.stale = true;
            }
        }
    }

    /// The value a screen is given for a place: a map with the `cell`,
    /// the perceived `entity` there or null, and `x`, `y` and `z`.
    pub(crate) fn place_value(&mut self, at: Location) -> ExprValue {
        let sprite = self
            .world
            .as_ref()
            .and_then(|world| world.model().top_sprite_id_at(at.grid, at.cell));
        let entity = sprite.map_or(ExprValue::Null, |id| self.host.sprite_value(id));
        ExprValue::map([
            ("cell", self.host.cell_value(at)),
            ("entity", entity),
            ("x", ExprValue::Int(i64::from(at.cell.x))),
            ("y", ExprValue::Int(i64::from(at.cell.y))),
            ("z", ExprValue::Int(i64::from(at.cell.z))),
        ])
    }

    /// What the party knows of the place under the cursor, and what the
    /// actor believes of choosing it.
    fn cursor_status(&mut self) -> CursorStatus {
        let mut status = CursorStatus {
            sight: SIGHT_UNKNOWN,
            ..CursorStatus::default()
        };
        let Some(world) = &self.world else {
            return status;
        };
        let camera = *world.camera();
        let (grid, at, subject) = (camera.grid(), camera.centre(), camera.subject());
        status.sight = match world.model().cell_in(grid, at) {
            Some(cell) if cell.seen() => SIGHT_SEEN,
            Some(_) => SIGHT_REMEMBERED,
            None => SIGHT_UNKNOWN,
        };
        let squared: f64 = [at.x - subject.x, at.y - subject.y, at.z - subject.z]
            .iter()
            .map(|d| f64::from(*d) * f64::from(*d))
            .sum();
        status.distance = squared.sqrt() * CELL_M.get();
        let place = self.place_value(Location::new(grid, at));
        let entity = place
            .as_map()
            .and_then(|map| map.get("entity"))
            .cloned()
            .unwrap_or_default();
        let cell = place
            .as_map()
            .and_then(|map| map.get("cell"))
            .cloned()
            .unwrap_or_default();
        if !entity.is_null() {
            status.target = self.name_of(&entity);
        }
        if let Some(collecting) = &self.collecting {
            status.count = collecting.candidates.len();
            status.index = collecting
                .candidates
                .iter()
                .position(|candidate| candidate.cell == at)
                .map_or(0, |index| index + 1);
            status.entries = collecting.list.len();
        }
        self.preview(&mut status, &cell, &entity);
        status
    }

    /// What the actor believes of choosing the place under the cursor
    /// for the parameter being asked for: the action's `requires`, and
    /// the parameter's `check` (03 §6.1, §6.3). Both are compiled in the
    /// actor's mind, so this reads belief.
    fn preview(&mut self, status: &mut CursorStatus, cell: &ExprValue, entity: &ExprValue) {
        let Some(collecting) = &self.collecting else {
            return;
        };
        let Collecting {
            def: Some(def_id),
            actor,
            asks,
            next,
            locals,
            list,
            ..
        } = collecting
        else {
            return;
        };
        let (Some(ask), Some(defs)) = (asks.get(*next), self.loaded()) else {
            return;
        };
        let registry = defs.exprs().registry();
        let candidate = match kind_of(ask.entry_type(), registry).as_deref() {
            Some("loc") => cell.clone(),
            Some("entity") => entity.clone(),
            _ => return,
        };
        if candidate.is_null() {
            return;
        }
        let def = defs.get::<ActionDef>(*def_id);
        let (actor, ask, later) = (actor.clone(), ask.clone(), asks[*next + 1..].to_vec());
        let mut locals = locals.clone();
        locals[ask.slot] = match ask.list {
            Some(_) => ExprValue::list(list.iter().cloned().chain([candidate])),
            None => candidate,
        };
        // What is not yet asked for stands at its default.
        let default_of = |ask: &Ask| {
            ask.param
                .and_then(|index| def.params().get(index))
                .and_then(|param| param.default.as_ref())
        };
        for after in &later {
            if let Some(default) = default_of(after) {
                locals[after.slot] =
                    self.eval_action(&defs, &actor, &locals, |eval| default.value(eval));
            }
        }
        let check = ask
            .param
            .and_then(|index| def.params().get(index))
            .and_then(|param| param.check.as_ref());
        if let Some(check) = check {
            let advice = self.eval_action(&defs, &actor, &locals, |eval| check.value(eval));
            if !advice.is_null() {
                status.advice = advice.to_string();
            }
        }
        // An action that requires nothing has nothing to preview.
        if def.requires.constant_value() != Some(true) {
            let holds = self.eval_action(&defs, &actor, &locals, |eval| def.requires.holds(eval));
            status.reach = if holds { REACH_YES } else { REACH_NO };
            if let (false, Some(text)) = (holds, &def.fail_text) {
                status.why = self.eval_action(&defs, &actor, &locals, |eval| text.render(eval));
            }
        }
    }

    /// The `camera` effect: moves the cursor and the view of the world.
    pub(crate) fn move_camera(&mut self, effect: &CameraEffect) -> Result<(), String> {
        let Some(world) = &mut self.world else {
            return Err("the scene has no view of the world".to_owned());
        };
        if effect.is_idle() {
            return Err("it says nothing to do: give `move`, `storey`, `zoom`, `cycle`, `release` or `turn`".to_owned());
        }
        let camera = world.camera_mut();
        if effect.release {
            camera.release_cursor();
            camera.reset_storey();
        }
        if let Some([east, south]) = effect.by {
            camera.move_cursor(east, south);
        }
        if effect.storey != 0 {
            camera.shift_storey(effect.storey);
        }
        if effect.zoom != 0 {
            let zoom = i64::from(camera.zoom()) + i64::from(effect.zoom);
            camera.set_zoom(u32::try_from(zoom.max(1)).unwrap_or(1));
        }
        if effect.turn {
            camera.set_section_axis(camera.section_axis().turned());
        }
        if effect.cycle != 0 {
            let (grid, storey, subject) = (camera.grid(), camera.storey(), camera.subject());
            let at = camera.centre();
            // A picker's candidates; anywhere else, what the party
            // perceives on the viewed storey, nearest first.
            let cells: Vec<CellPos> = match &self.collecting {
                Some(collecting) if !collecting.candidates.is_empty() => collecting
                    .candidates
                    .iter()
                    .map(|candidate| candidate.cell)
                    .collect(),
                _ => {
                    let mut cells: Vec<CellPos> = world
                        .model()
                        .sprites()
                        .filter(|(_, sprite)| {
                            sprite.loc.grid == grid && sprite.loc.cell.z == storey
                        })
                        .map(|(_, sprite)| sprite.loc.cell)
                        .filter(|cell| *cell != subject)
                        .collect();
                    cells.sort_by_key(|cell| {
                        let (dx, dy) = (
                            i64::from(cell.x) - i64::from(subject.x),
                            i64::from(cell.y) - i64::from(subject.y),
                        );
                        (dx * dx + dy * dy, cell.y, cell.x)
                    });
                    cells.dedup();
                    cells
                }
            };
            if !cells.is_empty() {
                let count = cells.len() as i64;
                let step = i64::from(effect.cycle.signum());
                let next = match cells.iter().position(|cell| *cell == at) {
                    Some(index) => (index as i64 + step).rem_euclid(count),
                    None if step > 0 => 0,
                    None => count - 1,
                };
                let cell = cells[next as usize];
                let camera = world.camera_mut();
                camera.shift_storey(cell.z.saturating_sub(camera.storey()));
                camera.set_cursor(cell.x, cell.y);
            }
        }
        self.stale = true;
        Ok(())
    }
}

impl<H> Scene<H>
where
    H: SceneHost,
    H::Command: From<UiCommand>,
    H::Refusal: core::fmt::Display,
{
    /// A place in a map view was picked (12 §9.4). A click where a free
    /// cursor is on show elsewhere moves the cursor there; a click on
    /// the cursor, a click with no cursor on show, and `accept` pick the
    /// place: the view's `on_pick` runs with the place as `it`, or, for
    /// a view with none, the control `click`. The second button runs the
    /// control `click_second`.
    #[allow(
        clippy::too_many_arguments,
        reason = "what the toolkit says of the click, as it says it"
    )]
    pub(crate) fn view_picked(
        &mut self,
        kind: ViewKind,
        zoom: u32,
        area: Rect,
        at: Option<(i32, i32)>,
        second: bool,
        effects: &UiEffects,
        out: &mut Vec<UiOutcome>,
    ) {
        if kind != ViewKind::Map {
            return;
        }
        let Some(world) = &mut self.world else {
            return;
        };
        let grid = world.camera().grid();
        let cell = match at {
            Some((x, y)) => match world.map_cell_at(zoom, area, x, y) {
                Some(cell) => cell,
                None => return,
            },
            None => world.camera().centre(),
        };
        if let (Some(_), false, Some(cursor)) = (at, second, world.camera().cursor())
            && (cursor.x, cursor.y) != (cell.x, cell.y)
        {
            world.camera_mut().set_cursor(cell.x, cell.y);
            self.stale = true;
            out.push(UiOutcome::Camera(CameraEffect::default()));
            return;
        }
        let place = self.place_value(Location::new(grid, cell));
        let control = |scene: &Scene<H>, name: &str| {
            scene
                .ui
                .as_ref()
                .and_then(|ui| ui.control(name, &scene.keymap_ns))
                .map(|(_, effects)| effects)
        };
        let effects = if second {
            control(self, CONTROL_CLICK_SECOND)
        } else if effects.is_empty() {
            control(self, CONTROL_CLICK)
        } else {
            Some(effects.clone())
        };
        if let Some(effects) = effects {
            self.run_into(&effects, Some(place), out);
        }
    }
}
