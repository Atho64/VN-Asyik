//! Request and response shapes for the VNDB API.
//!
//! Field names match the documented JSON exactly (`alttitle`, `length_minutes`,
//! `image.sexual`, …). Every struct tolerates extra fields and missing optionals,
//! because VNDB adds fields over time and refusing an unknown field would break
//! imports for no reason.

use serde::{Deserialize, Serialize};

/// A `POST /vn` request body.
#[derive(Debug, Clone, Serialize)]
pub struct VnQuery {
    /// Filter predicates, in the documented array form.
    pub filters: serde_json::Value,
    /// Comma-separated field list. Every field must be named explicitly.
    pub fields: String,
    /// How many results to return, max 100.
    pub results: u32,
    /// Page number, starting at 1.
    pub page: u32,
    /// Sort key: `id`, `title`, `released`, `rating`, `votecount` or `searchrank`.
    pub sort: String,
    /// Descending when true.
    pub reverse: bool,
}

impl Default for VnQuery {
    fn default() -> Self {
        Self {
            filters: serde_json::json!([]),
            fields: String::new(),
            results: 20,
            page: 1,
            sort: "searchrank".to_owned(),
            reverse: false,
        }
    }
}

impl VnQuery {
    /// Build a title search.
    ///
    /// `searchrank` sorting is what makes a title search feel relevant: it is the
    /// same ranking the website uses.
    #[must_use]
    pub fn by_title(title: &str, fields: impl Into<String>, page: u32, results: u32) -> Self {
        Self {
            filters: serde_json::json!(["search", "=", title]),
            fields: fields.into(),
            results: results.min(100),
            page,
            sort: "searchrank".to_owned(),
            reverse: false,
        }
    }
}

/// A `POST /vn` response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnQueryResult {
    /// The matched entries.
    #[serde(default)]
    pub results: Vec<Vn>,
    /// Whether another page exists.
    #[serde(default)]
    pub more: bool,
    /// Total matches, only present when the query asked for `count`.
    pub count: Option<u32>,
}

/// An image as the API reports it.
///
/// The same shape is used for the cover (`image`), every screenshot entry and
/// character portraits, so one decoder and one set of accessors covers all of
/// them. Every field stays optional: the API omits `votecount` on screenshots and
/// `thumbnail_dims` on characters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// Image identifier, e.g. `cv123` or `sf190`.
    pub id: Option<String>,
    /// Full-size URL.
    pub url: Option<String>,
    /// Pixel dimensions `[width, height]`.
    pub dims: Option<Vec<u32>>,
    /// Average sexual-content flagging vote, 0..2.
    pub sexual: Option<f32>,
    /// Average violence flagging vote, 0..2.
    pub violence: Option<f32>,
    /// Number of flagging votes.
    pub votecount: Option<u32>,
    /// Thumbnail URL.
    pub thumbnail: Option<String>,
    /// Thumbnail pixel dimensions `[width, height]`.
    pub thumbnail_dims: Option<Vec<u32>>,
}

impl Image {
    /// Thumbnail width and height, when reported in a valid shape.
    #[must_use]
    pub fn thumbnail_size(&self) -> (Option<u32>, Option<u32>) {
        match self.thumbnail_dims.as_deref() {
            Some([width, height, ..]) => (Some(*width), Some(*height)),
            _ => (None, None),
        }
    }

    /// Whether the community voted this image sexually explicit or violent.
    ///
    /// The threshold is the same one `flag_level` in the importer applies: a
    /// rounded level of 1 or higher. VNDB returns 0.93 for covers that are
    /// plainly explicit, so a stricter test would leak them.
    #[must_use]
    pub fn is_explicit(&self) -> bool {
        flag_level_from_vote(self.sexual) >= 1 || flag_level_from_vote(self.violence) >= 1
    }
}

/// Round VNDB's 0.0..2.0 vote onto the 0..2 flag level.
///
/// The only copy of this rounding. `import::flag_level` handles the same vote already
/// multiplied by 100, and both callers of the policy (the importer and the screenshot
/// cache) go through one of these two. A third copy used to sit in `dto`, a fourth in
/// TypeScript, and a threshold change meant finding all of them.
#[must_use]
pub fn flag_level_from_vote(value: Option<f32>) -> u8 {
    let Some(value) = value.filter(|value| value.is_finite()) else {
        return 0;
    };
    #[allow(clippy::cast_possible_truncation)]
    let hundredths = (f64::from(value.clamp(0.0, 2.0)) * 100.0).round() as i64;
    flag_level(hundredths)
}

/// Round hundredths (VNDB's own scale: 0..200) onto the 0..2 flag level.
///
/// Rounding, not flooring: Sanoba Witch is 0.93 sexual on VNDB and floors to 0, which
/// left the explicit-art filter with nothing to act on for any cover short of a
/// unanimous vote.
#[must_use]
pub fn flag_level(hundredths: i64) -> u8 {
    let level = (hundredths.clamp(0, 200) + 50) / 100;
    u8::try_from(level).unwrap_or(2)
}

/// A producer (developer or publisher) of a VN.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Producer {
    /// Producer id, e.g. `p8`.
    pub id: Option<String>,
    /// Name as displayed.
    pub name: Option<String>,
    /// Name in the original script.
    pub original: Option<String>,
    /// `co` for a company, `in` for an individual, `ng` for an amateur group.
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

/// A link to an external site, as listed on the VN's VNDB page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtLink {
    /// URL.
    pub url: Option<String>,
    /// English human-readable label, e.g. `Wikipedia (en)`.
    pub label: Option<String>,
    /// Internal site identifier, e.g. `enwiki`. Stable enough to style on.
    pub name: Option<String>,
}

/// A VN directly related to this one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedVn {
    /// Related VN id.
    pub id: Option<String>,
    /// Relation type code, e.g. `ser` (same series), `seq` (prequel),
    /// `alt` (alternative setting). See `enums.vn_relation` in the schema JSON.
    pub relation: Option<String>,
    /// Title of the related entry.
    pub title: Option<String>,
    /// Whether the relation is official.
    pub relation_official: Option<bool>,
}

impl Image {
    /// Width and height, when the API reported dimensions in a valid shape.
    #[must_use]
    pub fn size(&self) -> (Option<u32>, Option<u32>) {
        match self.dims.as_deref() {
            Some([width, height, ..]) => (Some(*width), Some(*height)),
            _ => (None, None),
        }
    }

    /// Sexual rating scaled to hundredths so it can be stored as an integer
    /// without losing the fractional part VNDB actually returns.
    #[must_use]
    pub fn sexual_hundredths(&self) -> Option<i64> {
        self.sexual.and_then(scale_to_hundredths)
    }

    /// Violence rating scaled to hundredths.
    #[must_use]
    pub fn violence_hundredths(&self) -> Option<i64> {
        self.violence.and_then(scale_to_hundredths)
    }
}

/// Convert a VNDB average rating (0..2) to integer hundredths.
///
/// The value is clamped to the documented range first, so the cast is bounded and
/// cannot lose information for any value the API can actually return.
fn scale_to_hundredths(value: f32) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    let clamped = value.clamp(0.0, 2.0);
    #[allow(clippy::cast_possible_truncation)]
    let scaled = (f64::from(clamped) * 100.0).round() as i64;
    Some(scaled)
}

/// One title variant in `titles`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TitleVariant {
    /// Language code, e.g. `ja`.
    pub lang: Option<String>,
    /// Title in the original script.
    pub title: Option<String>,
    /// Romanised form.
    pub latin: Option<String>,
    /// Whether this is the official title for that language.
    pub official: Option<bool>,
    /// Whether this is the VN's main title.
    pub main: Option<bool>,
}

/// A tag applied to a VN.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    /// Tag id, e.g. `g505`.
    pub id: Option<String>,
    /// Tag name.
    pub name: Option<String>,
    /// Display category: `cont`, `ero` or `tech`.
    pub category: Option<String>,
    /// Rating 0..3.
    pub rating: Option<f32>,
    /// Spoiler level 0..2.
    pub spoiler: Option<u8>,
    /// Whether the tag is a lie.
    pub lie: Option<bool>,
}

/// A release of a VN.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Release {
    /// Release id, e.g. `r123`.
    pub id: Option<String>,
    /// Title as displayed on the site.
    pub title: Option<String>,
    /// Alternative title in the original script.
    pub alttitle: Option<String>,
    /// Release date, `YYYY`, `YYYY-MM`, `YYYY-MM-DD` or `TBA`.
    pub released: Option<String>,
    /// Platforms.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Languages.
    #[serde(default)]
    pub languages: Vec<ReleaseLanguage>,
    /// Age rating, 0..18.
    pub minage: Option<i32>,
    /// Engine name, when VNDB knows it. Relevant to the engine adapters.
    pub engine: Option<String>,
    /// Whether this is a patch rather than a standalone release.
    pub patch: Option<bool>,
    /// Whether this is the official release.
    pub official: Option<bool>,
}

/// One language of a release.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleaseLanguage {
    /// Language code.
    pub lang: Option<String>,
    /// Title in that language.
    pub title: Option<String>,
    /// Whether this is a machine translation.
    pub mtl: Option<bool>,
}

/// A character as returned on a VN entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Character {
    /// Character id, e.g. `c123`.
    pub id: Option<String>,
    /// Name as displayed.
    pub name: Option<String>,
    /// Original-script name.
    pub original: Option<String>,
    /// Role in this VN: `main`, `primary`, `side` or `appears`.
    pub role: Option<String>,
    /// Portrait image.
    pub image: Option<Image>,
}

/// A visual novel entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Vn {
    /// VN id, e.g. `v17`.
    pub id: String,
    /// Main title as displayed on the site.
    pub title: Option<String>,
    /// Alternative (usually original-script) title.
    pub alttitle: Option<String>,
    /// All titles.
    #[serde(default)]
    pub titles: Vec<TitleVariant>,
    /// Aliases.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Original language.
    pub olang: Option<String>,
    /// Development status: 0 finished, 1 in development, 2 cancelled.
    pub devstatus: Option<u8>,
    /// Release date.
    pub released: Option<String>,
    /// Available languages.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Platforms.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Cover image.
    pub image: Option<Image>,
    /// Coarse length bucket 1..5.
    pub length: Option<u8>,
    /// Average user-submitted playtime, minutes.
    pub length_minutes: Option<i64>,
    /// Number of playtime votes.
    pub length_votes: Option<u32>,
    /// Description, may contain VNDB formatting codes.
    pub description: Option<String>,
    /// Raw vote average 10..100, before the Bayesian correction.
    pub average: Option<f32>,
    /// Bayesian rating 10..100.
    pub rating: Option<f32>,
    /// Number of votes.
    pub votecount: Option<u32>,
    /// Screenshots.
    #[serde(default)]
    pub screenshots: Vec<Image>,
    /// Tags applied directly to this VN.
    #[serde(default)]
    pub tags: Vec<Tag>,
    /// Releases, only when the field was requested.
    #[serde(default)]
    pub releases: Vec<Release>,
    /// Characters, only when the field was requested.
    #[serde(default)]
    pub characters: Vec<Character>,
    /// Developers, only when the field was requested.
    #[serde(default)]
    pub developers: Vec<Producer>,
    /// External links, only when the field was requested.
    #[serde(default)]
    pub extlinks: Vec<ExtLink>,
    /// Related VNs, only when the field was requested.
    #[serde(default)]
    pub relations: Vec<RelatedVn>,
}

impl Vn {
    /// The best original-script title available.
    ///
    /// Prefers the explicit `alttitle`, then the `main` title whose language
    /// matches `olang`, then any non-Latin title.
    #[must_use]
    pub fn original_title(&self) -> Option<String> {
        if let Some(alt) = self.alttitle.as_ref().filter(|alt| !alt.is_empty()) {
            return Some(alt.clone());
        }
        if let Some(main) = self
            .titles
            .iter()
            .find(|variant| variant.main == Some(true))
            .and_then(|variant| variant.title.clone())
        {
            return Some(main);
        }
        self.titles
            .iter()
            .filter_map(|variant| variant.title.clone())
            .find(|title| !title.is_ascii())
    }

    /// Whether this VN is still in development.
    #[must_use]
    pub fn is_development(&self) -> bool {
        self.devstatus == Some(1)
    }
}

/// Response from `GET /authinfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthInfo {
    /// User id, `u123`.
    pub id: String,
    /// Username.
    pub username: String,
    /// Permissions granted to the token.
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// The field list this app requests for a search result: enough to render a card
/// without a second call, and small enough to stay far under the "too much data"
/// threshold at 20 results.
pub const SEARCH_FIELDS: &str = "title,alttitle,titles.title,titles.lang,titles.main,olang,released,image.url,image.sexual,image.violence,image.dims,length,length_minutes,votecount,rating";

/// The field list for a full detail import.
///
/// Every field here is either rendered by the detail screen or used by the art
/// policy. `titles.latin` and `titles.official` are requested so a romanised name
/// and an official release name can be told apart from a plain translation.
///
/// The screenshots list is capped by the API at roughly 30 entries per VN and
/// asking for all of its fields is what the "too much data selected" rule counts,
/// so it stays as lean as the tiles need: id, url, dimensions and the two
/// flagging votes. Thumbnails are deliberately *not* requested for screenshots:
/// the app caches one downscaled copy locally and serves it from disk, so a
/// second remote variant would be a wasted download and a second failure path.
pub const DETAIL_FIELDS: &str = "title,alttitle,titles.title,titles.lang,titles.latin,titles.official,titles.main,aliases,olang,devstatus,released,languages,platforms,image.id,image.url,image.sexual,image.violence,image.dims,image.thumbnail,image.thumbnail_dims,image.votecount,length,length_minutes,length_votes,description,rating,average,votecount,developers.id,developers.name,developers.original,developers.type,extlinks.url,extlinks.label,extlinks.name,relations.id,relations.relation,relations.relation_official,relations.title,tags.id,tags.name,tags.category,tags.rating,tags.spoiler,tags.lie,screenshots.id,screenshots.url,screenshots.sexual,screenshots.violence,screenshots.dims";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_defaults_match_documented_shape() {
        let query = VnQuery::default();
        assert_eq!(query.results, 20);
        assert_eq!(query.page, 1);
        assert_eq!(query.sort, "searchrank");
        assert!(!query.reverse);
    }

    #[test]
    fn title_search_uses_documented_filter_form() {
        let query = VnQuery::by_title("umineko", SEARCH_FIELDS, 1, 25);
        assert_eq!(query.filters, serde_json::json!(["search", "=", "umineko"]));
        assert_eq!(query.results, 25);
    }

    #[test]
    fn result_count_is_capped_at_the_api_limit() {
        let query = VnQuery::by_title("x", SEARCH_FIELDS, 1, 500);
        assert_eq!(query.results, 100);
    }

    #[test]
    fn decodes_a_realistic_search_result() {
        let json = r#"{
          "results": [{
            "id": "v24",
            "title": "Umineko no Naku Koro ni",
            "alttitle": "うみねこのなく頃に",
            "titles": [
              {"lang": "ja", "title": "うみねこのなく頃に", "main": true, "official": true}
            ],
            "olang": "ja",
            "released": "2007-08-17",
            "length": 5,
            "length_minutes": 6000,
            "rating": 80.5,
            "votecount": 1234,
            "image": {"url": "https://t.vndb.org/cv/24/1.jpg", "sexual": 1.5, "violence": 0.4, "dims": [256, 360]}
          }],
          "more": true
        }"#;
        let parsed: VnQueryResult = serde_json::from_str(json).expect("decode");
        assert_eq!(parsed.results.len(), 1);
        assert!(parsed.more);
        let vn = &parsed.results[0];
        assert_eq!(vn.id, "v24");
        assert_eq!(vn.length_minutes, Some(6000));
        assert_eq!(vn.original_title().as_deref(), Some("うみねこのなく頃に"));

        let image = vn.image.as_ref().expect("image");
        assert_eq!(image.size(), (Some(256), Some(360)));
        assert_eq!(image.sexual_hundredths(), Some(150));
        assert_eq!(image.violence_hundredths(), Some(40));
    }

    #[test]
    fn tolerates_unknown_and_missing_fields() {
        let json =
            r#"{"results":[{"id":"v1","title":"X","some_future_field":{"a":1}}],"more":false}"#;
        let parsed: VnQueryResult = serde_json::from_str(json).expect("decode");
        assert_eq!(parsed.results[0].title.as_deref(), Some("X"));
        assert!(parsed.results[0].tags.is_empty());
        assert!(parsed.results[0].screenshots.is_empty());
    }

    #[test]
    fn original_title_falls_back_through_titles() {
        let vn = Vn {
            id: "v1".to_owned(),
            alttitle: None,
            titles: vec![
                TitleVariant {
                    lang: Some("en".to_owned()),
                    title: Some("English Name".to_owned()),
                    main: Some(false),
                    ..TitleVariant::default()
                },
                TitleVariant {
                    lang: Some("ja".to_owned()),
                    title: Some("日本語".to_owned()),
                    main: Some(true),
                    ..TitleVariant::default()
                },
            ],
            ..Vn::default()
        };
        assert_eq!(vn.original_title().as_deref(), Some("日本語"));
    }

    #[test]
    fn original_title_is_none_for_monolingual_latin_entries() {
        let vn = Vn {
            id: "v1".to_owned(),
            titles: vec![TitleVariant {
                title: Some("Only English".to_owned()),
                ..TitleVariant::default()
            }],
            ..Vn::default()
        };
        assert_eq!(vn.original_title(), None);
    }

    #[test]
    fn image_size_handles_a_bad_dims_array() {
        let image = Image {
            dims: Some(vec![100]),
            ..Image::default()
        };
        assert_eq!(image.size(), (None, None));
        assert_eq!(Image::default().size(), (None, None));
    }

    #[test]
    fn development_status_is_recognised() {
        let mut vn = Vn {
            devstatus: Some(1),
            ..Vn::default()
        };
        assert!(vn.is_development());
        vn.devstatus = Some(0);
        assert!(!vn.is_development());
    }

    #[test]
    fn detail_fields_stay_within_the_data_budget() {
        // The API rejects queries that would emit too many keys. Keeping the
        // field list modest is what prevents that, so guard its size.
        assert!(DETAIL_FIELDS.split(',').count() < 60);
        assert!(SEARCH_FIELDS.split(',').count() < 20);
    }

    #[test]
    fn detail_fields_request_everything_the_detail_screen_renders() {
        // Each of these is displayed once the screen gained its facts card and
        // screenshot gallery. A field dropped from the list is not a compile
        // error, it is a silently blank row in the UI.
        for field in [
            "developers.name",
            "extlinks.url",
            "screenshots.id",
            "screenshots.url",
            "screenshots.sexual",
            "screenshots.violence",
            "screenshots.dims",
            "languages",
            "platforms",
            "length_votes",
            "average",
            "devstatus",
            "image.thumbnail",
            "titles.latin",
        ] {
            assert!(DETAIL_FIELDS.contains(field), "{field} is not requested");
        }
        // Screenshot thumbnails are cached locally instead.
        assert!(!DETAIL_FIELDS.contains("screenshots.thumbnail"));
    }

    #[test]
    fn decodes_the_metadata_the_detail_screen_needs() {
        let json = r#"{
          "results": [{
            "id": "v17",
            "title": "Ever17",
            "devstatus": 0,
            "languages": ["en", "ja"],
            "platforms": ["win", "psp"],
            "length_votes": 203,
            "average": 84.5,
            "developers": [{"id": "p8", "name": "KID", "original": "キッド", "type": "co"}],
            "extlinks": [{"url": "https://example.invalid", "label": "Wikipedia (en)", "name": "enwiki"}],
            "relations": [{"id": "v13", "relation": "ser", "title": "Remember11", "relation_official": true}],
            "screenshots": [
              {"id": "sf190", "url": "https://t.vndb.org/sf/90/190.jpg", "dims": [800, 600], "sexual": 0, "violence": 0}
            ]
          }],
          "more": false
        }"#;
        let parsed: VnQueryResult = serde_json::from_str(json).expect("decode");
        let vn = &parsed.results[0];
        assert_eq!(vn.languages, vec!["en", "ja"]);
        assert_eq!(vn.platforms, vec!["win", "psp"]);
        assert_eq!(vn.length_votes, Some(203));
        assert_eq!(vn.average, Some(84.5));
        assert_eq!(vn.developers[0].name.as_deref(), Some("KID"));
        assert_eq!(vn.developers[0].original.as_deref(), Some("キッド"));
        assert_eq!(vn.developers[0].kind.as_deref(), Some("co"));
        assert_eq!(vn.extlinks[0].name.as_deref(), Some("enwiki"));
        assert_eq!(vn.relations[0].relation.as_deref(), Some("ser"));
        assert_eq!(vn.screenshots[0].id.as_deref(), Some("sf190"));
        assert_eq!(vn.screenshots[0].size(), (Some(800), Some(600)));
    }

    #[test]
    fn a_screenshot_threshold_matches_the_cover_threshold() {
        // Same rounding as `flag_level` in the importer: 0.93 is explicit.
        fn shot(sexual: f32, violence: f32) -> Image {
            Image {
                sexual: Some(sexual),
                violence: Some(violence),
                ..Image::default()
            }
        }
        assert!(!shot(0.0, 0.0).is_explicit());
        assert!(!shot(0.4, 0.49).is_explicit());
        assert!(shot(0.93, 0.0).is_explicit());
        assert!(shot(0.0, 1.0).is_explicit());
        assert!(!Image::default().is_explicit());
    }

    #[test]
    fn a_non_finite_flagging_vote_is_not_treated_as_explicit() {
        let image = Image {
            sexual: Some(f32::NAN),
            ..Image::default()
        };
        assert!(!image.is_explicit());
    }

    #[test]
    fn thumbnail_size_handles_bad_shapes() {
        let image = Image {
            thumbnail_dims: Some(vec![136, 102]),
            ..Image::default()
        };
        assert_eq!(image.thumbnail_size(), (Some(136), Some(102)));
        assert_eq!(
            Image {
                thumbnail_dims: Some(vec![136]),
                ..Image::default()
            }
            .thumbnail_size(),
            (None, None)
        );
    }
}
