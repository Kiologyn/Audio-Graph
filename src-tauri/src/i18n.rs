//! Язык сообщений, которые видит пользователь: проблемы графа, ошибки устройств, меню трея.
//! Журнал (log.rs) всегда по-русски.
//!
//! Словари общие с интерфейсом: src/locales/<язык>.json, ключ — русский текст, как он написан
//! в коде. Чего нет в словаре языка, показывается по-английски, а нет и там — по-русски.

use crate::settings::Language;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

static LANGUAGE: AtomicU8 = AtomicU8::new(Language::Ru as u8);

type Dictionary = HashMap<String, String>;

fn dictionary(language: Language) -> Option<&'static Dictionary> {
    static DICTIONARIES: OnceLock<HashMap<Language, Dictionary>> = OnceLock::new();
    let all = DICTIONARIES.get_or_init(|| {
        let sources = [
            (Language::En, include_str!("../../src/locales/en.json")),
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
        .unwrap_or(Language::Ru)
}

/// Текст по ключу на текущем языке
pub fn text(key: &'static str) -> &'static str {
    let found = |language| dictionary(language).and_then(|d| d.get(key)).map(String::as_str);
    match language() {
        Language::Ru => key,
        language => found(language).or_else(|| found(Language::En)).unwrap_or(key),
    }
}

/// Подставляет значения вместо {имя}; за один проход, чтобы фигурные скобки в самих
/// значениях (имя устройства) не приняли за подстановку
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

/// Строка на текущем языке: tr!("Петля: звук кабеля «{cable}»…", cable = name) → String.
/// Подстановки только именованные: порядок слов в языках разный
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

    #[test]
    fn dictionaries_parse_and_fill() {
        for language in Language::ALL.into_iter().filter(|l| *l != Language::Ru) {
            let d = dictionary(language).expect("словарь");
            assert!(d.contains_key("Запустить"), "{language:?}");
        }
        assert_eq!(fill("Кабель «{cable}», {n}", &[("cable", &"A"), ("n", &2)]), "Кабель «A», 2");
    }
}
