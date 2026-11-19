//! A field that keeps its place under the loader and is read plainly by
//! any other deserialiser (04 §7.1): `by_site_or_plain`.

use core::fmt;

use clash_data::{
    Diagnostics, FromSite, Kind, KindValue, Limits, Site, SourceMap, by_site_or_plain, from_node,
    parse_json,
};
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};

/// A text, or a number, that knows where it was written if the loader
/// read it.
#[derive(Debug, PartialEq)]
struct Note {
    text: String,
    site: Option<Site>,
}

struct Plain;

impl Visitor<'_> for Plain {
    type Value = Note;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a note")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Note, E> {
        Ok(Note {
            text: text.to_owned(),
            site: None,
        })
    }

    fn visit_u64<E: de::Error>(self, number: u64) -> Result<Note, E> {
        Ok(Note {
            text: number.to_string(),
            site: None,
        })
    }
}

impl<'de> FromSite<'de> for Note {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Note, A::Error> {
        let text = match kind {
            Kind::Number => value.read::<u64>()?.to_string(),
            _ => value.read::<String>()?,
        };
        Ok(Note {
            text,
            site: Some(site),
        })
    }
}

impl<'de> Deserialize<'de> for Note {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Note, D::Error> {
        by_site_or_plain(deserializer, Plain)
    }
}

#[derive(Debug, Deserialize)]
struct Page {
    title: Note,
    #[serde(default)]
    lines: Vec<Note>,
}

const PAGE: &str = r#"{ "title": "Ash",
  "lines": [ "one", 2 ] }"#;

#[test]
fn the_loader_gives_the_field_its_place_and_its_path() {
    let mut sources = SourceMap::new();
    let file = sources.add("page.json", PAGE);
    let (tree, _) = parse_json(file, sources.text(file), &Limits::default());
    let mut diagnostics = Diagnostics::new();
    let page: Page = from_node(&tree.unwrap(), &sources, &mut diagnostics, None).unwrap();
    assert!(diagnostics.is_empty(), "{}", diagnostics.render());

    let site = page.title.site.as_ref().expect("read by the loader");
    assert_eq!(page.title.text, "Ash");
    assert_eq!(site.path, "title");
    let span = site.span.expect("written in a file");
    assert_eq!(&PAGE[span.start as usize..span.end as usize], "\"Ash\"");

    let paths: Vec<&str> = page
        .lines
        .iter()
        .map(|line| line.site.as_ref().unwrap().path.as_str())
        .collect();
    assert_eq!(paths, ["lines[0]", "lines[1]"]);
    assert_eq!(page.lines[1].text, "2");
    let second = page.lines[1].site.as_ref().unwrap().span.unwrap();
    assert_eq!(&PAGE[second.start as usize..second.end as usize], "2");
}

#[test]
fn any_other_deserialiser_reads_the_field_plainly() {
    let page: Page = serde_json::from_str(PAGE).unwrap();
    assert_eq!(
        page.title,
        Note {
            text: "Ash".to_owned(),
            site: None
        }
    );
    assert_eq!(page.lines[1].text, "2");
    assert!(page.lines.iter().all(|line| line.site.is_none()));
    // What the plain visitor does not take is refused as usual.
    let error = serde_json::from_str::<Page>(r#"{ "title": true }"#).unwrap_err();
    assert!(error.to_string().contains("expected a note"), "{error}");
}

#[test]
fn a_value_of_the_wrong_kind_is_reported_where_it_is() {
    let mut sources = SourceMap::new();
    let file = sources.add("page.json", r#"{ "title": [1] }"#);
    let (tree, _) = parse_json(file, sources.text(file), &Limits::default());
    let mut diagnostics = Diagnostics::new();
    let page: Option<Page> = from_node(&tree.unwrap(), &sources, &mut diagnostics, None);
    assert!(page.is_none());
    let text = diagnostics.render();
    assert!(text.contains("--> page.json:1:12"), "{text}");
}
