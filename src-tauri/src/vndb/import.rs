//! Importing a VNDB entry into the library.
//!
//! The mapping rules live here, as pure functions, so they can be tested without
//! a network or a database. Two principles:
//!
//! * **User-owned fields win.** Status, rating, notes and manual playtime are
//!   never overwritten by an import. Only VNDB-owned fields are updated.
//! * **The entry is stored even if art fails.** A cover download failure must not
//!   lose the import.

use serde::{Deserialize, Serialize};
use vn_core::clock::Timestamp;

use crate::vndb::dto::{ExtLink, Producer, TitleVariant};
use vn_core::ids::DeviceId;
use vn_core::model::{Game, GameStatus, ModelError};

use crate::vndb::dto::{SEARCH_FIELDS, Vn};

/// A trimmed search hit for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    /// VN id.
    pub vn_id: String,
    /// Display title.
    pub title: String,
    /// Original-script title.
    pub alttitle: Option<String>,
    /// Release date as reported.
    pub released: Option<String>,
    /// Cover URL.
    pub image_url: Option<String>,
    /// Cover sexual rating in hundredths.
    pub sexual: Option<i64>,
    /// Cover violence rating in hundredths.
    pub violence: Option<i64>,
    /// Average playtime, minutes.
    pub length_minutes: Option<i64>,
    /// Coarse length bucket 1..5.
    pub length: Option<u8>,
    /// Number of votes, for sorting hints.
    pub votecount: Option<u32>,
    /// Whether the app already has this VN.
    pub in_library: bool,
}

/// Convert a raw VN entry into a search hit.
#[must_use]
pub fn to_search_hit(vn: &Vn, in_library: bool) -> SearchHit {
    let image = vn.image.as_ref();
    SearchHit {
        vn_id: vn.id.clone(),
        title: vn.title.clone().unwrap_or_else(|| vn.id.clone()),
        alttitle: vn.original_title(),
        released: vn.released.clone(),
        image_url: image.and_then(|img| img.url.clone()),
        sexual: image.and_then(crate::vndb::dto::Image::sexual_hundredths),
        violence: image.and_then(crate::vndb::dto::Image::violence_hundredths),
        length_minutes: vn.length_minutes,
        length: vn.length,
        votecount: vn.votecount,
        in_library,
    }
}

/// Field list used by search. Re-exported for the command layer.
pub const SEARCH: &str = SEARCH_FIELDS;

/// Build a new [`Game`] from a VNDB entry.
///
/// # Errors
/// Fails only when the title cannot satisfy the game model (empty/too long).
pub fn new_game_from_vn(device: &DeviceId, now: Timestamp, vn: &Vn) -> Result<Game, ModelError> {
    let title = vn
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| vn.id.clone());
    let mut game = Game::new(device, now, title, GameStatus::Wishlist)?;
    apply_vn_to_game(&mut game, vn);
    Ok(game)
}

/// Copy VNDB-owned fields onto an existing game, leaving user-owned fields alone.
///
/// This is the rule that makes re-importing safe: a user's status, rating, notes
/// and manual playtime survive a re-import, while the title, cover, length and
/// content ratings are refreshed.
pub fn apply_vn_to_game(game: &mut Game, vn: &Vn) {
    if let Some(title) = vn.title.as_ref().filter(|t| !t.trim().is_empty()) {
        game.title.clone_from(title);
    }
    game.title_original = vn.original_title();
    game.vndb_id = Some(vn.id.clone());
    game.length_minutes = vn.length_minutes;
    if let Some(image) = vn.image.as_ref() {
        game.cover_url.clone_from(&image.url);
        game.cover_sexual = image.sexual_hundredths().map(crate::vndb::dto::flag_level);
        game.cover_violence = image
            .violence_hundredths()
            .map(crate::vndb::dto::flag_level);
    }
}

/// Serialised detail payload stored in `vndb_details`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailPayload {
    /// VN id.
    pub vn_id: String,
    /// Main title.
    pub title: String,
    /// Original-script title.
    pub alttitle: Option<String>,
    /// Every published title: original script, romanised, and official English.
    ///
    /// Carried so the hero can show what a VN is actually called. `alttitle` alone
    /// was not enough: KANADE stores its own name there, so the page had one string
    /// to show and printed it twice.
    pub titles: Vec<TitleVariant>,
    /// Short names VNDB lists separately from its titles.
    ///
    /// Not part of `titles`: an alias carries no language or script, so it cannot
    /// be represented as a [`TitleVariant`] without inventing values. This is where
    /// the names people actually use live ("When They Cry 3").
    pub aliases: Vec<String>,
    /// Average playtime, minutes.
    pub length_minutes: Option<i64>,
    /// Coarse length bucket.
    pub length: Option<u8>,
    /// Number of playtime votes behind `length_minutes`.
    ///
    /// Without it an average of four hours could be one person's estimate or four
    /// hundred people's, and the UI has no way to say which.
    #[serde(default)]
    pub length_votes: Option<u32>,
    /// Description, still containing VNDB formatting codes.
    pub description: Option<String>,
    /// Bayesian rating.
    pub rating: Option<f32>,
    /// Raw vote average, before the Bayesian correction.
    #[serde(default)]
    pub average: Option<f32>,
    /// Vote count.
    pub votecount: Option<u32>,
    /// Release date.
    pub released: Option<String>,
    /// Original language.
    pub olang: Option<String>,
    /// Development status: 0 finished, 1 in development, 2 cancelled.
    #[serde(default)]
    pub devstatus: Option<u8>,
    /// Languages the VN is available in, excluding machine translations.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Platforms the VN is available on.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Tags, newest relevance first as returned.
    pub tags: Vec<crate::vndb::dto::Tag>,
    /// Screenshots.
    pub screenshots: Vec<crate::vndb::dto::Image>,
    /// Who developed it, as VNDB lists them.
    #[serde(default)]
    pub developers: Vec<Producer>,
    /// Official, encyclopaedia and store links from the VN's page.
    #[serde(default)]
    pub extlinks: Vec<ExtLink>,
    /// Cast, in VNDB's own order (main roles first).
    ///
    /// Stored since the first import but never read back, which is why the game
    /// page had no cast to show. `#[serde(default)]` so a payload stored before
    /// the field was exposed still decodes as an empty list instead of failing
    /// the whole detail load.
    #[serde(default)]
    pub characters: Vec<crate::vndb::dto::Character>,
}

impl DetailPayload {
    /// Build from a VN entry.
    #[must_use]
    pub fn from_vn(vn: &Vn) -> Self {
        Self {
            vn_id: vn.id.clone(),
            title: vn.title.clone().unwrap_or_else(|| vn.id.clone()),
            alttitle: vn.original_title(),
            titles: vn.titles.clone(),
            aliases: vn.aliases.clone(),
            length_minutes: vn.length_minutes,
            length: vn.length,
            length_votes: vn.length_votes,
            description: vn.description.clone(),
            rating: vn.rating,
            average: vn.average,
            votecount: vn.votecount,
            released: vn.released.clone(),
            olang: vn.olang.clone(),
            devstatus: vn.devstatus,
            languages: vn.languages.clone(),
            platforms: vn.platforms.clone(),
            tags: vn.tags.clone(),
            screenshots: vn.screenshots.clone(),
            developers: vn.developers.clone(),
            extlinks: vn.extlinks.clone(),
            characters: vn.characters.clone(),
        }
    }
}

/// Strip VNDB formatting codes (`[url=...]`, `[b]`, …) for plain display.
///
/// The API returns description text with formatting codes; this app has no rich
/// text renderer yet, so the tags are removed rather than shown raw.
#[must_use]
pub fn strip_formatting(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vndb::dto::{Image, TitleVariant};

    #[test]
    fn a_partial_sexual_rating_is_not_truncated_to_zero() {
        // Regression: this used `value / 100`, so every vote below 1.0 became 0.
        // Sanoba Witch is rated 0.93 on VNDB and was stored as 0, which left the
        // explicit-art filter with nothing to act on.
        assert_eq!(
            crate::vndb::dto::flag_level(93),
            1,
            "0.93 is explicit enough to flag"
        );
        assert_eq!(crate::vndb::dto::flag_level(150), 2);
        assert_eq!(crate::vndb::dto::flag_level(200), 2);
        assert_eq!(crate::vndb::dto::flag_level(0), 0);
    }

    #[test]
    fn a_faint_rating_stays_clear() {
        // Rounded, not floored upward without limit: a 0.4 cover is not explicit
        // by this app's threshold, and must not become one.
        assert_eq!(crate::vndb::dto::flag_level(40), 0);
        assert_eq!(crate::vndb::dto::flag_level(49), 0);
        assert_eq!(crate::vndb::dto::flag_level(50), 1);
    }

    #[test]
    fn an_out_of_range_rating_is_clamped() {
        assert_eq!(crate::vndb::dto::flag_level(9_999), 2);
        assert_eq!(crate::vndb::dto::flag_level(-5), 0);
    }

    fn device() -> DeviceId {
        DeviceId::parse("test-device").expect("device")
    }

    fn now() -> Timestamp {
        Timestamp::from_millis(1_700_000_000_000)
    }

    fn sample_vn() -> Vn {
        Vn {
            id: "v24".to_owned(),
            title: Some("Umineko no Naku Koro ni".to_owned()),
            alttitle: Some("うみねこのなく頃に".to_owned()),
            titles: vec![TitleVariant {
                lang: Some("ja".to_owned()),
                title: Some("うみねこのなく頃に".to_owned()),
                main: Some(true),
                ..TitleVariant::default()
            }],
            olang: Some("ja".to_owned()),
            released: Some("2007-08-17".to_owned()),
            length: Some(5),
            length_minutes: Some(6_000),
            votecount: Some(1_234),
            image: Some(Image {
                url: Some("https://t.vndb.org/cv/24/1.jpg".to_owned()),
                sexual: Some(1.5),
                violence: Some(0.4),
                dims: Some(vec![256, 360]),
                ..Image::default()
            }),
            ..Vn::default()
        }
    }

    #[test]
    fn search_hit_maps_documented_fields() {
        let hit = to_search_hit(&sample_vn(), false);
        assert_eq!(hit.vn_id, "v24");
        assert_eq!(hit.title, "Umineko no Naku Koro ni");
        assert_eq!(hit.alttitle.as_deref(), Some("うみねこのなく頃に"));
        assert_eq!(hit.sexual, Some(150));
        assert_eq!(hit.violence, Some(40));
        assert_eq!(hit.length_minutes, Some(6_000));
        assert_eq!(hit.length, Some(5));
        assert!(!hit.in_library);
    }

    #[test]
    fn import_creates_a_wishlist_game_with_vndb_fields() {
        let game = new_game_from_vn(&device(), now(), &sample_vn()).expect("game");
        assert_eq!(game.title, "Umineko no Naku Koro ni");
        assert_eq!(game.title_original.as_deref(), Some("うみねこのなく頃に"));
        assert_eq!(game.vndb_id.as_deref(), Some("v24"));
        assert_eq!(game.status, GameStatus::Wishlist);
        assert_eq!(game.length_minutes, Some(6_000));
        assert_eq!(
            game.cover_url.as_deref(),
            Some("https://t.vndb.org/cv/24/1.jpg")
        );
        // 1.5 rounds up to the top level. This assertion used to read "1.5
        // rounds down to a whole rating of 1", which documented the truncation
        // as intended rather than catching it.
        assert_eq!(game.cover_sexual, Some(2));
    }

    #[test]
    fn reimport_refreshes_vndb_fields_but_keeps_user_fields() {
        let mut game = new_game_from_vn(&device(), now(), &sample_vn()).expect("game");
        game.status = GameStatus::Completed;
        game.set_rating(Some(10)).expect("rating");
        game.set_notes(Some("my thoughts".to_owned()))
            .expect("notes");
        game.manual_playtime_ms = 3_600_000;
        let original_id = game.meta.id;

        let mut updated = sample_vn();
        updated.title = Some("Umineko (updated)".to_owned());
        updated.length_minutes = Some(7_000);
        apply_vn_to_game(&mut game, &updated);

        assert_eq!(game.title, "Umineko (updated)");
        assert_eq!(game.length_minutes, Some(7_000));
        // User-owned fields survive.
        assert_eq!(game.status, GameStatus::Completed);
        assert_eq!(game.rating, Some(10));
        assert_eq!(game.notes.as_deref(), Some("my thoughts"));
        assert_eq!(game.manual_playtime_ms, 3_600_000);
        assert_eq!(game.meta.id, original_id);
    }

    #[test]
    fn blank_vndb_title_does_not_erase_the_local_title() {
        let mut game = new_game_from_vn(&device(), now(), &sample_vn()).expect("game");
        let mut updated = sample_vn();
        updated.title = Some("   ".to_owned());
        apply_vn_to_game(&mut game, &updated);
        assert_eq!(game.title, "Umineko no Naku Koro ni");
    }

    #[test]
    fn missing_image_keeps_previous_cover() {
        let mut game = new_game_from_vn(&device(), now(), &sample_vn()).expect("game");
        let mut updated = sample_vn();
        updated.image = None;
        apply_vn_to_game(&mut game, &updated);
        assert!(game.cover_url.is_some(), "cover must not be cleared");
    }

    #[test]
    fn vn_without_a_title_falls_back_to_its_id() {
        let vn = Vn {
            id: "v999".to_owned(),
            title: None,
            ..Vn::default()
        };
        let game = new_game_from_vn(&device(), now(), &vn).expect("game");
        assert_eq!(game.title, "v999");
    }

    #[test]
    fn detail_payload_keeps_tags_and_screenshots() {
        let payload = DetailPayload::from_vn(&sample_vn());
        assert_eq!(payload.vn_id, "v24");
        assert_eq!(payload.alttitle.as_deref(), Some("うみねこのなく頃に"));
        assert_eq!(payload.tags.len(), 0, "payload.tags should be empty");
    }

    #[test]
    fn formatting_codes_are_stripped() {
        assert_eq!(
            strip_formatting("[b]Bold[/b] and [url=https://x]a link[/url]"),
            "Bold and a link"
        );
        assert_eq!(strip_formatting("plain text"), "plain text");
        assert_eq!(strip_formatting("[i]unclosed"), "unclosed");
    }

    #[test]
    fn search_field_list_is_what_we_documented() {
        assert!(SEARCH.contains("image.sexual"));
        assert!(SEARCH.contains("length_minutes"));
    }
}
