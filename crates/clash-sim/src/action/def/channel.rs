//! The `channel` definition type and what an action uses of channels
//! (03 §6.2).

use std::fmt;

use clash_data::{
    CompileCx, Def, Diagnostic, FinishCx, FromKind, Kind, KindValue, Located, Ref, Scope, Span,
    Text, by_kind, code,
};
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};

use crate::action::ids::{ChannelId, ChannelSet};

/// A channel (03 §6.2): something an action needs exclusive use of.
///
/// ```json
/// { "type": "channel", "id": "arm", "name": "arm" }
/// ```
///
/// What slots of a channel an entity has is the body's to say
/// (document 06). The engine knows that channels exist and never which.
#[derive(Debug, Default, Deserialize)]
pub struct ChannelDef {
    /// What the channel is called where a player is told of it, as in the
    /// reason an action cannot start. Left out, its id is shown.
    #[serde(default)]
    pub name: Option<Text>,
}

impl Def for ChannelDef {
    type Id = ChannelId;
    const TYPE: &'static str = "channel";
    const DOC: &'static str =
        "Something an action needs exclusive use of: an arm, the voice, attention (03 §6.2).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        // A name is about nothing in particular: nothing is bound.
        cx.compile(&mut self.name, &Scope::new().pure());
    }

    fn placeholder() -> Option<ChannelDef> {
        Some(ChannelDef::default())
    }
}

/// How many slots of one channel are needed (03 §6.2).
///
/// In data, a whole number of at least one, or `"all"`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Slots {
    /// This many free slots.
    Count(u16),
    /// Every slot of the channel the entity has, and at least one.
    All,
}

/// The word that asks for every slot of a channel.
const ALL_SLOTS: &str = "all";

impl<'de> FromKind<'de> for Slots {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Slots, A::Error> {
        let expected = "a number of slots, or \"all\"";
        match kind {
            Kind::Number => match value.read::<u16>()? {
                0 => Err(<A::Error as de::Error>::custom(
                    "an action uses at least one slot of a channel it names; leave the channel out to use none",
                )),
                count => Ok(Slots::Count(count)),
            },
            Kind::String => {
                let word = value.read::<String>()?;
                if word == ALL_SLOTS {
                    Ok(Slots::All)
                } else {
                    Err(<A::Error as de::Error>::custom(format!(
                        "expected {expected}, found \"{word}\""
                    )))
                }
            }
            other => Err(other.unexpected(expected)),
        }
    }
}

impl<'de> Deserialize<'de> for Slots {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Slots, D::Error> {
        by_kind(deserializer)
    }
}

/// The slots of one channel that an action, or one role of it, needs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ChannelNeed {
    /// The channel.
    pub channel: ChannelId,
    /// How many of its slots.
    pub slots: Slots,
    /// Where the channel was named.
    pub span: Span,
}

/// What an action, or one role of a joint action, uses of channels for as
/// long as it runs (03 §6.2): the `uses` of a definition.
///
/// ```json
/// "uses": { "arm": 2, "legs": "all" }
/// ```
///
/// [`set`](ChannelUse::set) is the channels as a bitset, for the quick
/// test of whether two actions can be in each other's way;
/// [`needs`](ChannelUse::needs) is how many slots of each, in order of
/// channel id.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ChannelUse {
    set: ChannelSet,
    needs: Vec<ChannelNeed>,
}

impl ChannelUse {
    /// The channels used, whatever the number of slots.
    pub fn set(&self) -> ChannelSet {
        self.set
    }

    /// The slots needed of each channel, in order of channel id.
    pub fn needs(&self) -> &[ChannelNeed] {
        &self.needs
    }

    /// The slots needed of one channel, if any.
    pub fn need(&self, channel: ChannelId) -> Option<Slots> {
        self.needs
            .binary_search_by_key(&channel, |need| need.channel)
            .ok()
            .map(|at| self.needs[at].slots)
    }

    /// Whether no channel is used.
    pub fn is_empty(&self) -> bool {
        self.needs.is_empty()
    }

    /// Puts the needs in order of channel id and refuses a channel named
    /// twice (as `arm` and as `core:arm`). `path` is the JSON path of the
    /// `uses` object.
    pub(crate) fn finish(&mut self, path: &str, cx: &mut FinishCx<'_>) {
        // A stable sort: of two entries for one channel the first written
        // stays first, and the second is the one reported.
        self.needs.sort_by_key(|need| need.channel);
        let mut repeats = Vec::new();
        self.needs.dedup_by(|later, first| {
            let same = later.channel == first.channel;
            if same {
                repeats.push(later.span);
            }
            same
        });
        for span in repeats {
            let diagnostic = Diagnostic::new(
                code::VALIDATION,
                "this channel is named twice in one `uses`; say once how many of its slots are needed",
            )
            .at(cx.sources(), span)
            .with_path(path);
            cx.report(diagnostic);
        }
    }
}

impl<'de> Deserialize<'de> for ChannelUse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ChannelUse, D::Error> {
        struct UseVisitor;

        impl<'de> Visitor<'de> for UseVisitor {
            type Value = ChannelUse;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object of channels, each with the number of slots needed")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ChannelUse, A::Error> {
                let mut used = ChannelUse::default();
                while let Some(channel) = map.next_key::<Located<Ref<ChannelDef>>>()? {
                    let slots = map.next_value::<Slots>()?;
                    let id = channel.value.id();
                    used.set.insert(id);
                    used.needs.push(ChannelNeed {
                        channel: id,
                        slots,
                        span: channel.span,
                    });
                }
                Ok(used)
            }
        }

        deserializer.deserialize_map(UseVisitor)
    }
}
