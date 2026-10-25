//! Live check that the cast actually arrives from VNDB.
//!
//! Ignored by default: it needs the network. Run with
//!
//! ```text
//! cargo test --test vndb_cast_live -- --ignored --nocapture
//! ```
//!
//! This exists because the cast card shipped with an empty list for every game:
//! `characters_json` was written as `[]` because `DETAIL_FIELDS` never requested
//! `va.character.*`, and `POST /vn` has no top-level `characters` field at all.
//! The unit tests could not catch that — the field list is a string, and a field
//! missing from it is not a compile error, it is a silently blank row.

use vn_tracker_lib::vndb::VndbClient;
use vn_tracker_lib::vndb::dto::DETAIL_FIELDS;

#[tokio::test]
#[ignore = "needs the network"]
async fn the_detail_field_list_brings_back_a_cast() {
    let client = VndbClient::new().expect("client");

    // Steins;Gate: a VN with a well-populated cast.
    let vn = client
        .get_vn("v17", DETAIL_FIELDS)
        .await
        .expect("request")
        .expect("v17 exists");

    assert!(
        !vn.va.is_empty(),
        "the field list must request va.character.*, or the cast is always empty"
    );

    let cast = client
        .get_vn_cast("v17", &vn.va)
        .await
        .expect("cast request");

    // Deduplicated: one entry per character, not one per voice actor.
    let mut ids: Vec<String> = cast.iter().filter_map(|c| c.id.clone()).collect();
    let before = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(before, ids.len(), "the cast must not repeat a character");

    assert!(cast.len() >= 5, "got {} characters", cast.len());

    // A role has to survive the second request, or the card shows no labels.
    let with_role = cast.iter().filter(|c| c.role.is_some()).count();
    assert!(with_role > 0, "no character carried a role for this VN");

    // Names and portraits are what the card actually paints.
    let named = cast.iter().filter(|c| c.name.is_some()).count();
    assert!(named > 0, "no character had a name");
    let pictured = cast
        .iter()
        .filter(|c| c.image.as_ref().and_then(|i| i.url.as_ref()).is_some())
        .count();
    assert!(pictured > 0, "no character had a portrait");

    // The detail panel renders these, and each is easy to lose silently: they
    // ride on the same query, so a dropped field is a blank row, not an error.
    let with_traits = cast.iter().filter(|c| !c.traits.is_empty()).count();
    assert!(with_traits > 0, "no character carried any traits");
    let with_birthday = cast.iter().filter(|c| c.birthday.is_some()).count();
    assert!(with_birthday > 0, "no character had a birthday");
    let safe_traits = cast
        .iter()
        .flat_map(|c| c.traits.iter())
        .filter(|t| (t.spoiler.unwrap_or(0)) <= 0)
        .count();
    assert!(
        safe_traits > 0,
        "every trait was spoiler-gated; the default ceiling would show nothing"
    );
    let with_va = cast.iter().filter(|c| c.voiced_by.is_some()).count();
    assert!(with_va > 0, "no character was joined to a voice actor");

    println!(
        "{} characters, {} with a role, {} with a portrait",
        cast.len(),
        with_role,
        pictured
    );
    println!(
        "{with_traits} with traits, {with_birthday} with a birthday, {with_va} with a voice actor, {safe_traits} safe traits"
    );
    for character in cast.iter().take(3) {
        println!(
            "  {:32} role={:8} birthday={:?} height={:?} va={:?} traits={}",
            character.name.as_deref().unwrap_or("-"),
            character.role.as_deref().unwrap_or("-"),
            character.birthday,
            character.height,
            character.voiced_by.as_deref().unwrap_or("-"),
            character.traits.len(),
        );
    }
}
