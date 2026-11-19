//! The `control` definition (12 §9.2).

use core::fmt;

use clash_data::CompileCx;
use clash_expr::Type;
use serde::Deserialize;
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};

use crate::ui::bind::{BindCx, ItType};
use crate::ui::{UiEffect, UiEffects};

/// A `control` definition (12 §9.2): something the player can do, by a
/// name that a keymap gives to a key.
///
/// ```json
/// { "type": "control", "id": "west", "name": "Walk west",
///   "do": { "act": { "action": "step", "params": { "dir": "'west'" } } } }
/// ```
///
/// A control is separate from the key that runs it: the keymap says
/// which key means `west` in which context, and the player may change
/// that without the control changing.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlDef {
    /// What the control is called where the player binds keys.
    #[serde(default)]
    pub name: Option<String>,
    /// What it does, in a sentence, for the key binding screen and the
    /// reference.
    #[serde(default)]
    pub doc: String,
    /// What it does: one interface effect, or a list of them, run in the
    /// screen that has the input. Where the control is run by a click on
    /// a view of the world, `it` is the place picked.
    #[serde(rename = "do", deserialize_with = "one_or_many")]
    pub effects: UiEffects,
}

/// Reads one effect or a list of effects.
fn one_or_many<'de, D: Deserializer<'de>>(deserializer: D) -> Result<UiEffects, D::Error> {
    struct DoVisitor;

    impl<'de> Visitor<'de> for DoVisitor {
        type Value = UiEffects;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an effect, or a list of effects")
        }

        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<UiEffects, A::Error> {
            UiEffect::deserialize(MapAccessDeserializer::new(map))
                .map(|effect| UiEffects(vec![effect]))
        }

        fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<UiEffects, A::Error> {
            Vec::<UiEffect>::deserialize(SeqAccessDeserializer::new(seq)).map(UiEffects)
        }
    }

    deserializer.deserialize_any(DoVisitor)
}

impl ControlDef {
    /// Compiles the expressions of the control's effects. A control may
    /// be run in any screen, so, like a `ui_part`, it sees `view`,
    /// `party`, `app` and the screen's variables unchecked, not a
    /// screen's parameters, and is held to what an ordinary screen may
    /// know. `it` is the place picked, where a click runs the control,
    /// and null otherwise.
    pub fn compile_bindings(&mut self, cx: &mut CompileCx<'_>) {
        let mut bx = BindCx {
            cx,
            debug: false,
            params: Vec::new(),
            vars: None,
            it: Some(ItType {
                ty: Type::Any,
                own: false,
            }),
        };
        self.effects.compile(&mut bx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_control_does_one_effect_or_a_list() {
        let one: ControlDef =
            serde_json::from_str(r#"{ "name": "Inventory", "do": { "open": "inventory" } }"#)
                .unwrap();
        assert_eq!(one.effects.names().collect::<Vec<_>>(), ["open"]);
        assert_eq!(one.name.as_deref(), Some("Inventory"));
        let many: ControlDef = serde_json::from_str(
            r#"{ "do": [ { "open": "look" }, { "camera": { "move": [0, 0] } } ] }"#,
        )
        .unwrap();
        assert_eq!(many.effects.names().collect::<Vec<_>>(), ["open", "camera"]);
        let wrong = serde_json::from_str::<ControlDef>(r#"{ "do": "open" }"#).unwrap_err();
        assert!(wrong.to_string().contains("an effect"), "{wrong}");
        let wrong = serde_json::from_str::<ControlDef>(r#"{ "does": [] }"#).unwrap_err();
        assert!(wrong.to_string().contains("does"), "{wrong}");
    }
}
