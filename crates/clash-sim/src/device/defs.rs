//! The `device` definition type (16 §4.2), as far as package E12 needs
//! it, and the check that every element's `device` names one.

use clash_core::define_id;
use clash_data::{CompileCx, CrossCx, Def, DefLabel, Diagnostic, Scope, Text, code};
use clash_world::defs::{BlockDef, FixtureDef, FloorDef};
use serde::Deserialize;

use crate::item::HoldsDef;

define_id! {
    /// A `device` definition (16 §4.2).
    pub struct DeviceId(u16);
}

/// What an element becomes when it needs state of its own, or a machine
/// standing alone (16 §4.2).
///
/// ```json
/// { "type": "device", "id": "locker_device", "name": "locker",
///   "holds": { "volume": 0.6, "mass": "200 kg" },
///   "tags": ["storage"] }
/// ```
///
/// Package G2 adds modes, ports, stores, config, signals, slots and
/// rules.
#[derive(Debug, Deserialize)]
pub struct DeviceDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What it holds (06 §9.2): its room by volume and mass and the tags
    /// it takes. Left out, nothing can be put in it.
    #[serde(default)]
    pub holds: Option<HoldsDef>,
    /// Tags the device entity carries.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Def for DeviceDef {
    type Id = DeviceId;
    const TYPE: &'static str = "device";
    const DOC: &'static str = "What an element becomes when it needs state of its own, or a machine standing alone: what it holds and the tags it carries (16 §4.2).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
    }
}

/// The namespace of a qualified id.
fn namespace_of(qualified: &str) -> &str {
    qualified
        .split_once(':')
        .map_or("", |(namespace, _)| namespace)
}

/// Whether a reference written in a definition of `owner` names a
/// definition of `T`: as written if qualified, else in the owner's
/// namespace.
pub(crate) fn names<T: Def>(cx: &CrossCx<'_>, owner: &str, written: &str) -> bool {
    let defs = cx.defs();
    if written.contains(':') {
        return defs.lookup::<T>(written).is_some();
    }
    defs.lookup::<T>(&format!("{}:{written}", namespace_of(owner)))
        .is_some()
}

/// Reports every element whose `device` names no `device` definition
/// (02 §5.5). A cross check of the loader.
pub fn check_element_devices(cx: &mut CrossCx<'_>) {
    let defs = cx.defs();
    let mut missing: Vec<(&'static str, String, String)> = Vec::new();
    let mut look = |ty: &'static str, name: &str, device: Option<&str>| {
        if let Some(device) = device
            && !names::<DeviceDef>(cx, name, device)
        {
            missing.push((ty, name.to_owned(), device.to_owned()));
        }
    };
    for (id, def) in defs.iter::<FloorDef>() {
        let name = defs.name::<FloorDef>(id).unwrap_or_default();
        look(
            FloorDef::TYPE,
            name,
            def.device.as_ref().map(|d| d.as_str()),
        );
    }
    for (id, def) in defs.iter::<BlockDef>() {
        let name = defs.name::<BlockDef>(id).unwrap_or_default();
        look(
            BlockDef::TYPE,
            name,
            def.device.as_ref().map(|d| d.as_str()),
        );
    }
    for (id, def) in defs.iter::<FixtureDef>() {
        let name = defs.name::<FixtureDef>(id).unwrap_or_default();
        look(
            FixtureDef::TYPE,
            name,
            def.device.as_ref().map(|d| d.as_str()),
        );
    }
    for (ty, element, device) in missing {
        let diagnostic = Diagnostic::new(
            code::UNKNOWN_REFERENCE,
            format!("there is no device definition `{device}` for this element to become"),
        )
        .in_def(Some(&DefLabel::new(ty, element)))
        .with_path("device");
        cx.report(diagnostic);
    }
}
