//! Row structs that mirror the `SQLite` tables, plus conversion to domain types.
//!
//! Keeping this separate from `vn-core` means the domain model can evolve
//! without every change rewriting `SQL`, and the mapping is a single testable seam.

use sqlx::FromRow;
use vn_core::clock::Timestamp;
use vn_core::ids::{DeviceId, EntityId};
use vn_core::model::{
    Chapter, Ending, Game, GameStatus, ModelError, NsfwMode, SessionSource, SyncMeta,
};

/// A row of `games`.
#[derive(Debug, Clone, FromRow)]
pub struct GameRow {
    pub id: String,
    pub title: String,
    pub title_original: Option<String>,
    pub vndb_id: Option<String>,
    pub status: String,
    pub rating: Option<i64>,
    pub notes: Option<String>,
    pub length_minutes: Option<i64>,
    pub manual_playtime_ms: i64,
    pub accent_color: Option<String>,
    pub cover_url: Option<String>,
    pub cover_sexual: Option<i64>,
    pub cover_violence: Option<i64>,
    /// App-data-relative path to a user-chosen cover. Added by migration 0023.
    pub custom_cover_path: Option<String>,
    /// 1 when the custom cover came from VNDB. `NOT NULL DEFAULT 0` upstream, so
    /// the read type is plain `i64` rather than an `Option`.
    pub custom_cover_source_vndb: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub updated_at: i64,
    pub device_id: String,
    pub deleted: i64,
}

/// A row of `chapters`.
#[derive(Debug, Clone, FromRow)]
pub struct ChapterRow {
    pub id: String,
    pub game_id: String,
    pub name: String,
    pub sort_index: i64,
    pub reached: i64,
    pub first_seen_at: Option<i64>,
    pub updated_at: i64,
    pub device_id: String,
    pub deleted: i64,
    /// Selected with COALESCE in the queries so older rows read as defaults.
    pub display_name: Option<String>,
    pub route_key: Option<String>,
    pub route_name: Option<String>,
    pub position: Option<String>,
    pub renamed: Option<i64>,
    /// Linked route, added by migration 0021. Read with an alias in queries so an
    /// old row that predates the column still lists, exactly as `display_name` is.
    pub route_id: Option<String>,
}

/// A row of `play_sessions`.
#[derive(Debug, Clone, FromRow)]
pub struct SessionRow {
    pub id: String,
    pub game_id: String,
    pub chapter_id: Option<String>,
    /// Route the user timed directly, added by migration 0022.
    pub route_id: Option<String>,
    pub started_at: i64,
    pub ended_at: i64,
    pub counted_ms: i64,
    pub afk_ms: i64,
    pub source: String,
    pub updated_at: i64,
    pub device_id: String,
    pub deleted: i64,
}

/// A row of `endings`.
#[derive(Debug, Clone, FromRow)]
pub struct EndingRow {
    pub id: String,
    pub game_id: String,
    pub name: String,
    pub route: Option<String>,
    pub achieved: i64,
    pub achieved_at: Option<i64>,
    pub updated_at: i64,
    pub device_id: String,
    pub deleted: i64,
    /// Linked route, added by migration 0021.
    pub route_id: Option<String>,
}

fn sync_meta(
    id: &str,
    updated_at: i64,
    device_id: &str,
    deleted: i64,
) -> Result<SyncMeta, ModelError> {
    Ok(SyncMeta {
        id: EntityId::parse(id)?,
        updated_at: Timestamp::from_millis(updated_at),
        device_id_device: vn_core::model::DeviceTag::new(&DeviceId::parse(device_id)?),
        deleted: deleted != 0,
    })
}

/// Read a stored rating, which is in decivotes: 10 is 1.0, 100 is 10.0.
fn rating_from(value: Option<i64>) -> Result<Option<u8>, ModelError> {
    match value {
        None => Ok(None),
        Some(raw) => u8::try_from(raw)
            .ok()
            .filter(|v| (10..=100).contains(v))
            .map(Some)
            .ok_or(ModelError::RatingOutOfRange(
                raw.unsigned_abs().min(255) as u8
            )),
    }
}

fn small_u8(value: Option<i64>) -> Option<u8> {
    value.and_then(|v| u8::try_from(v).ok())
}

impl TryFrom<GameRow> for Game {
    type Error = ModelError;

    fn try_from(row: GameRow) -> Result<Self, Self::Error> {
        let status = GameStatus::parse(&row.status).map_err(|_| ModelError::TextTooLong {
            field: "status",
            len: row.status.len(),
            max: 16,
        })?;
        Ok(Self {
            meta: sync_meta(&row.id, row.updated_at, &row.device_id, row.deleted)?,
            title: row.title,
            title_original: row.title_original,
            vndb_id: row.vndb_id,
            status,
            rating: rating_from(row.rating)?,
            notes: row.notes,
            length_minutes: row.length_minutes,
            manual_playtime_ms: row.manual_playtime_ms,
            accent_color: row.accent_color,
            cover_url: row.cover_url,
            cover_sexual: small_u8(row.cover_sexual),
            cover_violence: small_u8(row.cover_violence),
            custom_cover_path: row.custom_cover_path,
            custom_cover_source_vndb: row.custom_cover_source_vndb != 0,
            started_at: row.started_at.map(Timestamp::from_millis),
            finished_at: row.finished_at.map(Timestamp::from_millis),
        })
    }
}

impl TryFrom<ChapterRow> for Chapter {
    type Error = ModelError;

    fn try_from(row: ChapterRow) -> Result<Self, Self::Error> {
        Ok(Self {
            meta: sync_meta(&row.id, row.updated_at, &row.device_id, row.deleted)?,
            game_id: EntityId::parse(&row.game_id)?,
            name: row.name,
            sort_index: i32::try_from(row.sort_index).unwrap_or(i32::MAX),
            reached: row.reached != 0,
            first_seen_at: row.first_seen_at.map(Timestamp::from_millis),
            display_name: row.display_name,
            route_key: row.route_key,
            route_name: row.route_name,
            route_id: row.route_id.map(|raw| EntityId::parse(&raw)).transpose()?,
            position: row.position,
            renamed: row.renamed.unwrap_or(0) != 0,
        })
    }
}

/// A session plus the chapter it was attributed to, as read back for the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionView {
    /// The session itself.
    pub session: vn_core::model::PlaySession,
    /// Extracted here rather than on `PlaySession` to keep the model free of
    /// `Option<Option<_>>` ambiguity between "no chapter" and "chapter deleted".
    pub chapter_name: Option<String>,
}

impl TryFrom<SessionRow> for vn_core::model::PlaySession {
    type Error = ModelError;

    fn try_from(row: SessionRow) -> Result<Self, Self::Error> {
        let source = SessionSource::parse(&row.source).map_err(|_| ModelError::TextTooLong {
            field: "source",
            len: row.source.len(),
            max: 16,
        })?;
        Ok(Self {
            meta: sync_meta(&row.id, row.updated_at, &row.device_id, row.deleted)?,
            game_id: EntityId::parse(&row.game_id)?,
            chapter_id: row.chapter_id.as_deref().map(EntityId::parse).transpose()?,
            route_id: row.route_id.as_deref().map(EntityId::parse).transpose()?,
            started_at: Timestamp::from_millis(row.started_at),
            ended_at: Timestamp::from_millis(row.ended_at),
            counted_ms: row.counted_ms,
            afk_ms: row.afk_ms,
            source,
        })
    }
}

impl TryFrom<EndingRow> for Ending {
    type Error = ModelError;

    fn try_from(row: EndingRow) -> Result<Self, Self::Error> {
        Ok(Self {
            meta: sync_meta(&row.id, row.updated_at, &row.device_id, row.deleted)?,
            game_id: EntityId::parse(&row.game_id)?,
            name: row.name,
            route: row.route,
            route_id: row.route_id.map(|raw| EntityId::parse(&raw)).transpose()?,
            achieved: row.achieved != 0,
            achieved_at: row.achieved_at.map(Timestamp::from_millis),
        })
    }
}

/// A row of `routes`.
#[derive(Debug, Clone, FromRow)]
pub struct RouteRow {
    pub id: String,
    pub game_id: String,
    pub name: String,
    pub sort_order: Option<i64>,
    pub origin: String,
    pub merged_into: Option<String>,
    pub updated_at: i64,
    pub device_id: String,
    pub deleted: i64,
}

impl TryFrom<RouteRow> for vn_core::model::Route {
    type Error = ModelError;

    fn try_from(row: RouteRow) -> Result<Self, Self::Error> {
        Ok(Self {
            meta: sync_meta(&row.id, row.updated_at, &row.device_id, row.deleted)?,
            game_id: EntityId::parse(&row.game_id)?,
            name: row.name,
            sort_order: row
                .sort_order
                .map(|value| i32::try_from(value).unwrap_or(i32::MAX)),
            origin: vn_core::model::RouteOrigin::from_stored(&row.origin),
            merged_into: row
                .merged_into
                .map(|raw| EntityId::parse(&raw))
                .transpose()?,
        })
    }
}

/// A serialisable snapshot of everything the UI needs at startup.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LibrarySnapshot {
    /// All live games.
    pub games: Vec<Game>,
    /// Local, unsynced preferences.
    pub prefs: PrefsSnapshot,
}

/// Preferences as exchanged with the frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PrefsSnapshot {
    /// `dark` | `light` | `system`.
    pub theme: String,
    /// Interface language tag.
    pub language: String,
    /// `show` | `blur` | `hide`.
    pub nsfw: String,
    /// Swap glass for solid surfaces.
    pub reduce_effects: bool,
}

impl Default for PrefsSnapshot {
    fn default() -> Self {
        let prefs = vn_core::model::LocalPrefs::default();
        Self {
            theme: prefs.theme.as_str().to_owned(),
            language: prefs.language,
            nsfw: prefs.nsfw.as_str().to_owned(),
            reduce_effects: prefs.reduce_effects,
        }
    }
}

impl PrefsSnapshot {
    /// Parse the NSFW field, falling back to the safe default.
    #[must_use]
    pub fn nsfw_mode(&self) -> NsfwMode {
        NsfwMode::parse(&self.nsfw).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_row() -> GameRow {
        GameRow {
            id: "0192f0aa-0000-7000-8000-000000000001".to_owned(),
            title: "Umineko".to_owned(),
            title_original: Some("うみねこのなく頃に".to_owned()),
            vndb_id: Some("v24".to_owned()),
            status: "playing".to_owned(),
            rating: Some(10),
            notes: None,
            length_minutes: Some(3_000),
            manual_playtime_ms: 0,
            accent_color: Some("#8b9cff".to_owned()),
            cover_url: None,
            cover_sexual: Some(1),
            cover_violence: Some(2),
            custom_cover_path: None,
            custom_cover_source_vndb: 0,
            started_at: Some(1_000),
            finished_at: None,
            updated_at: 5_000,
            device_id: "dev-a".to_owned(),
            deleted: 0,
        }
    }

    #[test]
    fn game_row_converts() {
        let game = Game::try_from(game_row()).expect("convert");
        assert_eq!(game.title, "Umineko");
        assert_eq!(game.rating, Some(10));
        assert_eq!(game.status, GameStatus::Playing);
        assert_eq!(game.meta.updated_at.as_millis(), 5_000);
        assert_eq!(game.cover_sexual, Some(1));
        assert!(game.started_at.is_some());
    }

    #[test]
    fn out_of_range_rating_in_storage_is_rejected() {
        // 99 is a valid decivote (9.9). The rejected values are outside 10..=100.
        for bad in [9_i64, 101, -5] {
            let mut row = game_row();
            row.rating = Some(bad);
            assert!(Game::try_from(row).is_err(), "{bad} must be rejected");
        }
        let mut row = game_row();
        row.rating = Some(99);
        assert!(Game::try_from(row).is_ok(), "9.9 is a legitimate rating");
    }

    #[test]
    fn unknown_status_in_storage_is_rejected() {
        let mut row = game_row();
        row.status = "teleported".to_owned();
        assert!(Game::try_from(row).is_err());
    }

    #[test]
    fn bad_uuid_in_storage_is_rejected() {
        let mut row = game_row();
        row.id = "not-a-uuid".to_owned();
        assert!(Game::try_from(row).is_err());
    }

    #[test]
    fn deleted_flag_maps_to_boolean() {
        let mut row = game_row();
        row.deleted = 1;
        let game = Game::try_from(row).expect("convert");
        assert!(game.meta.deleted);
    }

    #[test]
    fn prefs_default_is_safe() {
        let prefs = PrefsSnapshot::default();
        assert_eq!(prefs.nsfw_mode(), NsfwMode::Blur);
        assert_eq!(prefs.theme, "dark");
        assert!(!prefs.reduce_effects);
    }

    #[test]
    fn corrupt_nsfw_value_falls_back_to_blur() {
        let prefs = PrefsSnapshot {
            nsfw: "nonsense".to_owned(),
            ..PrefsSnapshot::default()
        };
        assert_eq!(prefs.nsfw_mode(), NsfwMode::Blur);
    }
}
