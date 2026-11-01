//! Language of the messages the user sees: graph problems, device errors, the tray menu.
//! The log (log.rs) is always in English.
//!
//! Strings are referenced by codes, like Android string resources: tr!("issue_loop").
//! src/locales/<language>.json maps codes to a language's strings; the files are shared with the
//! interface. English (en.json) is the base: a string missing in a language is shown in English.

use crate::settings::Language;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

static LANGUAGE: AtomicU8 = AtomicU8::new(Language::En as u8);

type Dictionary = HashMap<String, String>;

fn dictionary(language: Language) -> Option<&'static Dictionary> {
    static DICTIONARIES: OnceLock<HashMap<Language, Dictionary>> = OnceLock::new();
    let all = DICTIONARIES.get_or_init(|| {
        let sources = [
            (Language::En, include_str!("../../src/locales/en.json")),
            (Language::Ru, include_str!("../../src/locales/ru.json")),
            (Language::Es, include_str!("../../src/locales/es.json")),
            (Language::Zh, include_str!("../../src/locales/zh.json")),
            (Language::Ja, include_str!("../../src/locales/ja.json")),
        ];
        sources
            .into_iter()
            .map(|(language, json)| (language, serde_json::from_str(json).unwrap_or_default()))
            .collect()
    });
    all.get(&language)
}

pub fn set_language(language: Language) {
    LANGUAGE.store(language as u8, Ordering::Relaxed);
}

pub fn language() -> Language {
    Language::ALL
        .into_iter()
        .find(|l| *l as u8 == LANGUAGE.load(Ordering::Relaxed))
        .unwrap_or(Language::En)
}

/// The string of a code in the current language; missing in it: English; missing in English too: the code
pub fn text(code: &'static str) -> &'static str {
    let found = |language| dictionary(language).and_then(|d| d.get(code)).map(String::as_str);
    found(language()).or_else(|| found(Language::En)).unwrap_or(code)
}

/// Puts the values in place of {name} in one pass, so that braces inside the values themselves
/// (a device name) are not taken for placeholders
pub fn fill(template: &str, values: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after
            .find('}')
            .and_then(|close| values.iter().find(|(name, _)| *name == &after[..close]).map(|(_, v)| (close, v)));
        match value {
            Some((close, v)) => {
                out.push_str(&v.to_string());
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A string in the current language: tr!("issue_cable_into_itself", cable = name) → String.
/// Placeholders are named only: word order differs between languages
#[macro_export]
macro_rules! tr {
    ($key:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::fill(
            $crate::i18n::text($key),
            &[$((stringify!($name), &$value as &dyn std::fmt::Display)),*],
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeSet;
    use std::path::Path;

    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split('{').skip(1).filter_map(|s| s.split_once('}').map(|(name, _)| name)).collect()
    }

    #[test]
    fn every_language_has_every_string() {
        let en = dictionary(Language::En).expect("en.json");
        for language in Language::ALL {
            let d = dictionary(language).unwrap_or_else(|| panic!("{language:?}: the file doesn't parse"));
            let missing: Vec<_> = en.keys().filter(|code| !d.contains_key(*code)).collect();
            let extra: Vec<_> = d.keys().filter(|code| !en.contains_key(*code)).collect();
            let placeholders_differ: Vec<_> =
                d.iter().filter(|(code, text)| en.get(*code).is_some_and(|e| placeholders(e) != placeholders(text))).collect();
            assert!(missing.is_empty(), "{language:?}: missing {missing:?}");
            assert!(extra.is_empty(), "{language:?}: not in en.json {extra:?}");
            assert!(placeholders_differ.is_empty(), "{language:?}: other placeholders than en.json {placeholders_differ:?}");
        }
    }

    /// Codes in t("…") (interface) and tr!("…") (Rust) calls
    fn used_codes(dir: &Path, call: &str, out: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).expect("source folder").flatten() {
            let path = entry.path();
            if path.is_dir() {
                used_codes(&path, call, out);
                continue;
            }
            let source = matches!(path.extension().and_then(|e| e.to_str()), Some("rs" | "ts" | "tsx"));
            if !source || path.ends_with("i18n.rs") || path.ends_with("i18n.ts") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("source file");
            for (at, _) in text.match_indices(call) {
                // t( must not be the end of a longer name, such as set(
                let before = text[..at].chars().next_back();
                if call == "t(" && before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                    continue;
                }
                let rest = text[at + call.len()..].trim_start();
                if let Some(code) = rest.strip_prefix('"').and_then(|r| r.split_once('"')).map(|(code, _)| code) {
                    out.insert(code.to_string());
                }
            }
        }
    }

    #[test]
    fn used_codes_exist() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut codes = BTreeSet::new();
        used_codes(&root.join("src"), "tr!(", &mut codes);
        used_codes(&root.join("../src"), "t(", &mut codes);
        assert!(codes.len() > 100, "too few codes found: {}", codes.len());
        let en = dictionary(Language::En).expect("en.json");
        let missing: Vec<_> = codes.iter().filter(|code| !en.contains_key(*code)).collect();
        assert!(missing.is_empty(), "codes not in en.json: {missing:?}");
    }

    #[test]
    fn fills_placeholders() {
        assert_eq!(fill("Cable \"{cable}\", {n}", &[("cable", &"A"), ("n", &2)]), "Cable \"A\", 2");
        assert_eq!(fill("{a} {missing}", &[("a", &"{b}"), ("b", &1)]), "{b} {missing}");
    }
}
