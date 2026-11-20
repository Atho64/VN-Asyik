//! Name preparation and text-only editing for walkthrough translation.

use super::WalkthroughRoute;
use std::collections::{BTreeMap, HashMap};

/// Apply text-only edits while keeping route/item identities and source metadata.
pub fn apply_translated_texts(
    routes: &[WalkthroughRoute],
    texts: &[String],
) -> Result<Vec<WalkthroughRoute>, &'static str> {
    let expected = routes
        .iter()
        .map(|route| 1 + route.items.len())
        .sum::<usize>();
    if texts.len() != expected {
        return Err("Translation must include every route title and step in source order");
    }
    if texts
        .iter()
        .any(|text| text.trim().is_empty() || text.len() > 8192)
        || texts.iter().map(String::len).sum::<usize>() > 1_048_576
    {
        return Err("Translation text must be nonempty and within the size limit");
    }
    let mut result = routes.to_vec();
    let mut translated = texts.iter();
    for route in &mut result {
        route.title.clone_from(translated.next().unwrap());
        for item in &mut route.items {
            item.text.clone_from(translated.next().unwrap());
        }
    }
    Ok(result)
}

/// VNDB's original and romanized name order is preserved. Conflicting aliases
/// are omitted so a shared given name never selects an arbitrary character.
#[must_use]
pub fn character_glossary(
    names: impl IntoIterator<Item = (String, String)>,
) -> HashMap<String, String> {
    let mut entries = BTreeMap::<String, Option<String>>::new();
    let mut insert = |source: String, target: String| {
        entries
            .entry(source)
            .and_modify(|current| {
                if current.as_ref() != Some(&target) {
                    *current = None;
                }
            })
            .or_insert(Some(target));
    };
    for (original, name) in names {
        let original_parts: Vec<_> = original.split_whitespace().collect();
        let name_parts: Vec<_> = name.split_whitespace().collect();
        if original_parts.is_empty() || name_parts.is_empty() {
            continue;
        }
        let name = name_parts.join(" ");
        for separator in [" ", "　", ""] {
            insert(original_parts.join(separator), name.clone());
        }
        // Kana docs leave `name` unstructured. Only derive a short form for the
        // verified two-part Japanese/romanized shape, keeping the same order.
        if original_parts.len() == 2 && name_parts.len() == 2 {
            let given = original_parts[1];
            if given.chars().count() >= 2 {
                insert(given.to_owned(), name_parts[1].to_owned());
            }
        }
    }
    entries
        .into_iter()
        .filter_map(|(source, target)| target.map(|target| (source, target)))
        .collect()
}

/// Longest matches win, and inserted names are never processed again.
#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn replace_names(text: &str, glossary: &HashMap<String, String>) -> String {
    let mut entries: Vec<_> = glossary
        .iter()
        .filter(|(source, _)| !source.is_empty())
        .collect();
    entries.sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()).then(a.cmp(b)));
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while !remaining.is_empty() {
        if let Some((source, target)) = entries
            .iter()
            .find(|(source, _)| remaining.starts_with(source.as_str()))
        {
            result.push_str(target);
            remaining = &remaining[source.len()..];
        } else if let Some(ch) = remaining.chars().next() {
            result.push(ch);
            remaining = &remaining[ch.len_utf8()..];
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_texts_preserve_identity_notes_and_reject_incomplete_edits() {
        use crate::walkthrough::{ItemKind, WalkthroughItem};
        let routes = vec![WalkthroughRoute {
            id: "route_0".into(),
            title: "小鳥".into(),
            items: vec![WalkthroughItem {
                id: "c_1".into(),
                kind: ItemKind::Choice,
                text: "小鳥を選ぶ".into(),
                note: Some("条件".into()),
            }],
        }];
        let edited =
            apply_translated_texts(&routes, &["Kotori".into(), "Choose Kotori".into()]).unwrap();
        assert_eq!(edited[0].id, routes[0].id);
        assert_eq!(edited[0].items[0].id, "c_1");
        assert_eq!(edited[0].items[0].kind, ItemKind::Choice);
        assert_eq!(edited[0].items[0].note, routes[0].items[0].note);
        assert_eq!(edited[0].items[0].text, "Choose Kotori");
        for texts in [
            vec!["Kotori".into()],
            vec!["Kotori".into(), " ".into()],
            vec!["Kotori".into(), "x".repeat(8193)],
        ] {
            assert!(apply_translated_texts(&routes, &texts).is_err());
        }
    }

    fn glossary() -> HashMap<String, String> {
        // Recorded Kana /character response for v751, 2026-11-15.
        character_glossary([("神戸 小鳥".into(), "Kanbe Kotori".into())])
    }

    #[test]
    fn given_names_follow_vndb_order_and_full_names_win() {
        assert_eq!(
            replace_names("神戸 小鳥と小鳥END", &glossary()),
            "Kanbe KotoriとKotoriEND"
        );
    }

    #[test]
    fn names_match_ascii_full_width_and_missing_spaces() {
        for source in ["神戸 小鳥", "神戸　小鳥", "神戸小鳥"] {
            assert_eq!(replace_names(source, &glossary()), "Kanbe Kotori");
        }
    }

    #[test]
    fn shared_given_names_are_omitted_in_either_cast_order() {
        let names = vec![
            ("神戸 小鳥".into(), "Kanbe Kotori".into()),
            ("山田 小鳥".into(), "Yamada Kotori".into()),
        ];
        // Even an identical romanization is ambiguous as an alias, but replacing
        // with the same literal name is safe and does not choose either surname.
        assert_eq!(
            character_glossary(names.clone())
                .get("小鳥")
                .map(String::as_str),
            Some("Kotori")
        );
        let names = vec![
            ("神戸 小鳥".into(), "Kanbe Kotori".into()),
            ("山田 小鳥".into(), "Yamada Kori".into()),
        ];
        assert!(!character_glossary(names.clone()).contains_key("小鳥"));
        assert_eq!(
            character_glossary(names.clone()),
            character_glossary(names.into_iter().rev())
        );
    }

    #[test]
    fn replacement_is_single_pass_and_ignores_empty_keys() {
        let names = HashMap::from([
            (String::new(), "bad".into()),
            ("小鳥".into(), "Kotori".into()),
            ("Kotori".into(), "wrong".into()),
        ]);
        assert_eq!(replace_names("小鳥END", &names), "KotoriEND");
    }

    #[test]
    fn a_longer_name_wins_over_a_matching_prefix() {
        let names = HashMap::from([
            ("ルチア".into(), "Lucia".into()),
            ("ルチア マリア".into(), "Lucia Maria".into()),
        ]);
        assert_eq!(replace_names("ルチア マリアEND", &names), "Lucia MariaEND");
    }
}
