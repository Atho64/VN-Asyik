//! Domain models shared by the database, the UI and sync.
//!
//! Every synced record carries the same four metadata fields (`SyncMeta`), which
//! is what makes last-write-wins merge and soft delete possible later.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::clock::Timestamp;
use crate::ids::{DeviceId, EntityId};

/// Errors produced while building or validating domain values.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelError {
    /// A string was not a valid `UUID`.
    #[error("invalid id: {0}")]
    InvalidId(String),
    /// A device id was blank.
    #[error("device id must not be empty")]
    EmptyDeviceId,
    /// A device id was unreasonably long.
    #[error("device id is too long: {0} bytes")]
    InvalidDeviceId(usize),
    /// The system clock returned a value outside the representable range.
    #[error("system clock is out of range")]
    ClockOutOfRange,
    /// A rating outside 10..=100 decivotes was supplied.
    #[error("rating must be between 10 and 100 decivotes (1.0 to 10.0), got {0}")]
    RatingOutOfRange(u8),
    /// A playtime value was negative.
    #[error("playtime must not be negative, got {0} ms")]
    NegativePlaytime(i64),
    /// A session ended before it started.
    #[error("session ended before it started")]
    SessionEndsBeforeStart,
    /// A chapter referenced a game that does not own it.
    #[error("chapter {chapter} does not belong to game {game}")]
    ChapterGameMismatch {
        /// Chapter id.
        chapter: EntityId,
        /// Expected game id.
        game: EntityId,
    },
    /// A title was empty after trimming.
    #[error("title must not be empty")]
    EmptyTitle,
    /// A text field exceeded its storage budget.
    #[error("{field} is too long: {len} > {max}")]
    TextTooLong {
        /// Field name.
        field: &'static str,
        /// Actual length.
        len: usize,
        /// Maximum allowed length.
        max: usize,
    },
}

/// The four fields every synced row shares.
///
/// Not `Copy`: the device tag owns a `String`, and cloning it on every touch
/// would be the only allocation in the hot path anyway. `Clone` is cheap enough
/// because the tag is shared behind an `Arc`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncMeta {
    /// Stable identity across devices.
    pub id: EntityId,
    /// Last local modification, UTC ms. Merge key, together with `device_id`.
    pub updated_at: Timestamp,
    /// Which installation wrote this revision.
    pub device_id_device: DeviceTag,
    /// Soft delete. Rows are never physically removed until a compaction pass.
    pub deleted: bool,
}

/// A device identifier that is cheap to copy into `SyncMeta`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTag(pub std::sync::Arc<DeviceId>);

// `Arc<DeviceId>` has no serde impl, and it should not get one: the tag is row
// metadata, serialised in backups and IPC as the plain device id string.
impl Serialize for DeviceTag {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for DeviceTag {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        DeviceId::parse(&text)
            .map(|id| Self(std::sync::Arc::new(id)))
            .map_err(serde::de::Error::custom)
    }
}

impl DeviceTag {
    /// Wrap a device id for storage in row metadata.
    #[must_use]
    pub fn new(device: &DeviceId) -> Self {
        Self(std::sync::Arc::new(device.clone()))
    }

    /// The wrapped identifier.
    #[must_use]
    pub fn device(&self) -> &DeviceId {
        &self.0
    }
}

impl SyncMeta {
    /// Create metadata for a brand new row.
    #[must_use]
    pub fn new(device: &DeviceId, now: Timestamp) -> Self {
        Self {
            id: EntityId::new(),
            updated_at: now,
            device_id_device: DeviceTag::new(device),
            deleted: false,
        }
    }

    /// Mark a row as locally modified at `now`.
    #[must_use]
    pub fn touch(&self, now: Timestamp) -> Self {
        Self {
            id: self.id,
            updated_at: now,
            device_id_device: self.device_id_device.clone(),
            deleted: self.deleted,
        }
    }

    /// Mark a row as deleted (soft delete) at `now`.
    #[must_use]
    pub fn tombstone(&self, now: Timestamp) -> Self {
        Self {
            id: self.id,
            updated_at: now,
            device_id_device: self.device_id_device.clone(),
            deleted: true,
        }
    }
}

/// Where a game sits in the user's reading queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameStatus {
    /// Planned to read.
    Wishlist,
    /// Currently reading.
    Playing,
    /// Finished (at least one ending).
    Completed,
    /// Paused, intend to return.
    OnHold,
    /// Abandoned.
    Dropped,
}

impl GameStatus {
    /// Every status, in display order.
    pub const ALL: [Self; 5] = [
        Self::Playing,
        Self::Wishlist,
        Self::Completed,
        Self::OnHold,
        Self::Dropped,
    ];

    /// Stable storage key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wishlist => "wishlist",
            Self::Playing => "playing",
            Self::Completed => "completed",
            Self::OnHold => "on_hold",
            Self::Dropped => "dropped",
        }
    }

    /// Parse a storage key.
    ///
    /// # Errors
    /// Returns the unrecognised text when it is not a known status.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "wishlist" => Ok(Self::Wishlist),
            "playing" => Ok(Self::Playing),
            "completed" => Ok(Self::Completed),
            "on_hold" => Ok(Self::OnHold),
            "dropped" => Ok(Self::Dropped),
            other => Err(other.to_owned()),
        }
    }
}

/// How explicit cover/screenshot art is allowed to be shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NsfwMode {
    /// Show everything.
    Show,
    /// Blur explicit art until the user reveals it. Safe default.
    #[default]
    Blur,
    /// Replace explicit art with a neutral placeholder.
    Hide,
}

impl NsfwMode {
    /// Stable storage key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Blur => "blur",
            Self::Hide => "hide",
        }
    }

    /// Parse a storage key.
    ///
    /// # Errors
    /// Returns the unrecognised text when it is not a known mode.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "show" => Ok(Self::Show),
            "blur" => Ok(Self::Blur),
            "hide" => Ok(Self::Hide),
            other => Err(other.to_owned()),
        }
    }
}

/// The minimum VNDB flag level this app conceals.
///
/// One constant, because the same number used to be written in four places and three
/// of them disagreed: covers compared `>= 2`, the screenshot cache compared the
/// rounded level `>= 1`, and the frontend's `isExplicit` compared raw `>= 1`. A cover
/// and a screenshot of the same scene could therefore be treated differently by the
/// same setting.
///
/// Level 1, not 2: VNDB's level is a rounding of an average vote, so level 1 already
/// means the community flagged the image above safe — Sanoba Witch's cover sits at
/// 0.93 sexual and must be concealed rather than shown.
pub const EXPLICIT_FLAG_LEVEL: u8 = 1;

/// Whether one stored flag level counts as explicit.
///
/// `None` is **not** explicit here: this is the in-app display policy, and a row that
/// predates the rating columns must still render. Anything that broadcasts or uploads
/// art (Discord presence) must fail closed instead and treat `None` as explicit; see
/// `crate::presence::cover_is_explicit`.
#[must_use]
pub fn is_explicit_at_level(level: Option<u8>) -> bool {
    level.is_some_and(|level| level >= EXPLICIT_FLAG_LEVEL)
}

/// A game in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Game {
    /// Row identity and sync metadata.
    pub meta: SyncMeta,
    /// Display title.
    pub title: String,
    /// Original-language title, usually Japanese.
    pub title_original: Option<String>,
    /// VNDB visual novel id (`v1234`) when linked.
    pub vndb_id: Option<String>,
    /// Where this game is in the user's queue.
    pub status: GameStatus,
    /// 1..=10, or `None` when unrated. Half points are not supported on purpose.
    /// Decivotes: 10 is 1.0, 100 is 10.0. Shares VNDB's scale.
    pub rating: Option<u8>,
    /// Free-form notes.
    pub notes: Option<String>,
    /// VNDB-reported length in minutes, used for the progress ring.
    pub length_minutes: Option<i64>,
    /// Manual playtime adjustment in ms, for time played before tracking.
    pub manual_playtime_ms: i64,
    /// Cached dominant colour (`#rrggbb`) extracted from the cover.
    pub accent_color: Option<String>,
    /// Cover image URL (VNDB) or cached local path.
    pub cover_url: Option<String>,
    /// Content rating from VNDB: 1 = all ages, 2 = sexual, 3 = explicit.
    pub cover_sexual: Option<u8>,
    /// Violence rating from VNDB.
    pub cover_violence: Option<u8>,
    /// App-data-relative path to a user-chosen cover, when the game has one.
    ///
    /// `None` means the game draws on its VNDB cover or the neutral surface.
    /// Relative, never absolute: this value travels in the sync payload, and a
    /// path from another machine would resolve to nothing.
    pub custom_cover_path: Option<String>,
    /// Whether [`Self::custom_cover_path`] holds an image downloaded from VNDB.
    ///
    /// VNDB's rating is authoritative and synced; an image the user picked by
    /// hand carries no rating we can trust. A metadata refresh checks this flag
    /// so it never overwrites artwork the user chose deliberately.
    pub custom_cover_source_vndb: bool,
    /// When the user started reading.
    pub started_at: Option<Timestamp>,
    /// When the user finished.
    pub finished_at: Option<Timestamp>,
}

impl Game {
    /// Maximum stored title length, in characters.
    pub const MAX_TITLE_CHARS: usize = 500;
    /// Maximum stored note length, in characters.
    pub const MAX_NOTES_CHARS: usize = 20_000;

    /// Build a new game with validation.
    ///
    /// # Errors
    /// Fails on an empty title, an over-long title/notes, or a rating outside 1..=10.
    pub fn new(
        device: &DeviceId,
        now: Timestamp,
        title: impl Into<String>,
        status: GameStatus,
    ) -> Result<Self, ModelError> {
        let title = title.into();
        validate_title(&title)?;
        Ok(Self {
            meta: SyncMeta::new(device, now),
            title,
            title_original: None,
            vndb_id: None,
            status,
            rating: None,
            notes: None,
            length_minutes: None,
            manual_playtime_ms: 0,
            accent_color: None,
            cover_url: None,
            cover_sexual: None,
            cover_violence: None,
            custom_cover_path: None,
            custom_cover_source_vndb: false,
            started_at: None,
            finished_at: None,
        })
    }

    /// Set the rating, validating the range.
    ///
    /// Stored in decivotes: 10 is 1.0, 75 is 7.5, 100 is 10.0. That is the same
    /// scale as VNDB's `vote`, so syncing neither rounds nor rescales.
    ///
    /// # Errors
    /// Fails when `rating` is not in 10..=100.
    pub fn set_rating(&mut self, rating: Option<u8>) -> Result<(), ModelError> {
        if let Some(value) = rating
            && !(10..=100).contains(&value)
        {
            return Err(ModelError::RatingOutOfRange(value));
        }
        self.rating = rating;
        Ok(())
    }

    /// Replace the notes, enforcing the storage budget.
    ///
    /// # Errors
    /// Fails when the notes exceed [`Self::MAX_NOTES_CHARS`].
    pub fn set_notes(&mut self, notes: Option<String>) -> Result<(), ModelError> {
        if let Some(text) = &notes {
            let len = text.chars().count();
            if len > Self::MAX_NOTES_CHARS {
                return Err(ModelError::TextTooLong {
                    field: "notes",
                    len,
                    max: Self::MAX_NOTES_CHARS,
                });
            }
        }
        self.notes = notes;
        Ok(())
    }

    /// True when the art should be treated as explicit under `mode`.
    ///
    /// Level 1 or higher, not level 2. VNDB's scale is 0 (safe) / 1 (suggestive) /
    /// 2 (explicit), and its votes are averages: an image the community clearly reads
    /// as adult art routinely lands at 1.4 or 1.6, which rounds to level 2 and hides
    /// correctly, while one at 1.1 rounds to 1 and used to pass as safe. One threshold
    /// applies to every image in the app — `crate::vndb::dto::flag_level` is the single
    /// rounding, and `is_explicit_at_level` is the single comparison.
    #[must_use]
    pub fn is_explicit(&self, mode: NsfwMode) -> bool {
        if mode == NsfwMode::Show {
            return false;
        }
        is_explicit_at_level(self.cover_sexual) || is_explicit_at_level(self.cover_violence)
    }

    /// Total playtime in ms including the manual adjustment.
    #[must_use]
    pub fn total_playtime_ms(&self, tracked_ms: i64) -> i64 {
        tracked_ms.saturating_add(self.manual_playtime_ms).max(0)
    }
}

fn validate_title(title: &str) -> Result<(), ModelError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(ModelError::EmptyTitle);
    }
    let len = trimmed.chars().count();
    if len > Game::MAX_TITLE_CHARS {
        return Err(ModelError::TextTooLong {
            field: "title",
            len,
            max: Game::MAX_TITLE_CHARS,
        });
    }
    Ok(())
}

/// A chapter or route section within a game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    /// Row identity and sync metadata.
    pub meta: SyncMeta,
    /// Owning game.
    pub game_id: EntityId,
    /// Display name, e.g. "Chapter 3" or "Mio route".
    pub name: String,
    /// Sort key; lower comes first.
    pub sort_index: i32,
    /// Whether the user has reached this chapter.
    pub reached: bool,
    /// When the chapter was first seen by tracking.
    pub first_seen_at: Option<Timestamp>,
    /// What to show, when the user renamed it. `name` stays the detection key, so
    /// a rename must not replace it or the next poll creates a duplicate row.
    pub display_name: Option<String>,
    /// Stable route identifier from the engine, e.g. `route_20`.
    pub route_key: Option<String>,
    /// Human route label, e.g. `Shiroha`.
    pub route_name: Option<String>,
    /// The route this chapter belongs to, when it has been linked to one.
    ///
    /// Distinct from `route_key`: that is the engine's detection key and changes
    /// with a game build, while this is the user-visible route a chapter, an
    /// ending and their shared playtime are all grouped under.
    pub route_id: Option<EntityId>,
    /// The position marker as the engine reported it, e.g. `7月26日`.
    pub position: Option<String>,
    /// Whether `display_name` is a user rename rather than a seeded value.
    ///
    /// Recorded rather than inferred: `display_name` is seeded on insert, so
    /// comparing it to `name` cannot tell "seeded" from "renamed".
    pub renamed: bool,
}

impl Chapter {
    /// Build a new chapter.
    ///
    /// # Errors
    /// Fails when the name is blank or over-long.
    pub fn new(
        device: &DeviceId,
        now: Timestamp,
        game_id: EntityId,
        name: impl Into<String>,
        sort_index: i32,
    ) -> Result<Self, ModelError> {
        let name = name.into();
        let len = name.trim().chars().count();
        if len == 0 {
            return Err(ModelError::EmptyTitle);
        }
        if len > 200 {
            return Err(ModelError::TextTooLong {
                field: "chapter name",
                len,
                max: 200,
            });
        }
        Ok(Self {
            meta: SyncMeta::new(device, now),
            game_id,
            name: name.trim().to_owned(),
            sort_index,
            reached: false,
            first_seen_at: None,
            display_name: None,
            route_key: None,
            route_name: None,
            route_id: None,
            position: None,
            renamed: false,
        })
    }
}

/// Which source produced a play session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSource {
    /// Desktop process/window tracking.
    Desktop,
    /// Android `UsageStats` reconciliation.
    AndroidUsage,
    /// The user typed the time in by hand.
    Manual,
    /// Imported from a backup or another tool.
    Import,
}

impl SessionSource {
    /// Stable storage key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::AndroidUsage => "android_usage",
            Self::Manual => "manual",
            Self::Import => "import",
        }
    }

    /// Parse a storage key.
    ///
    /// # Errors
    /// Returns the unrecognised text when it is not a known source.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "desktop" => Ok(Self::Desktop),
            "android_usage" => Ok(Self::AndroidUsage),
            "manual" => Ok(Self::Manual),
            "import" => Ok(Self::Import),
            other => Err(other.to_owned()),
        }
    }
}

/// One contiguous stretch of play time.
///
/// Sessions are append-only for sync: they are never overwritten by a merge,
/// only corrected by explicit user edits (which bump `updated_at`) or by
/// splitting into new rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaySession {
    /// Row identity and sync metadata.
    pub meta: SyncMeta,
    /// Owning game.
    pub game_id: EntityId,
    /// Chapter attributed by the active adapter, when detection succeeded.
    pub chapter_id: Option<EntityId>,
    /// Route the user timed directly, when the session targets a route instead of a
    /// chapter.
    pub route_id: Option<EntityId>,
    /// Inclusive start, UTC ms.
    pub started_at: Timestamp,
    /// Exclusive end, UTC ms.
    pub ended_at: Timestamp,
    /// Active time in ms, the number the library and stats headline. Not
    /// necessarily `ended_at - started_at`: idle stretches and paused time are
    /// excluded, and the remainder is recorded in `afk_ms`.
    pub counted_ms: i64,
    /// Time the user was away during this session, in ms. Kept separately so
    /// idle time is never silently discarded (see `crate::afk`).
    pub afk_ms: i64,
    /// What produced this session.
    pub source: SessionSource,
}

impl PlaySession {
    /// Build a session, validating the time range.
    ///
    /// # Errors
    /// Fails when the session ends before it starts or the counted time is negative.
    pub fn new(
        device: &DeviceId,
        now: Timestamp,
        game_id: EntityId,
        started_at: Timestamp,
        ended_at: Timestamp,
        counted_ms: i64,
        source: SessionSource,
    ) -> Result<Self, ModelError> {
        if ended_at < started_at {
            return Err(ModelError::SessionEndsBeforeStart);
        }
        if counted_ms < 0 {
            return Err(ModelError::NegativePlaytime(counted_ms));
        }
        Ok(Self {
            meta: SyncMeta::new(device, now),
            game_id,
            chapter_id: None,
            route_id: None,
            started_at,
            ended_at,
            counted_ms,
            afk_ms: 0,
            source,
        })
    }

    /// Record idle time for this session.
    ///
    /// # Errors
    /// Fails when `afk_ms` is negative.
    pub fn with_afk(mut self, afk_ms: i64) -> Result<Self, ModelError> {
        if afk_ms < 0 {
            return Err(ModelError::NegativePlaytime(afk_ms));
        }
        self.afk_ms = afk_ms;
        Ok(self)
    }

    /// Total time attributed to this session, active plus away.
    #[must_use]
    pub const fn elapsed_ms(&self) -> i64 {
        self.counted_ms.saturating_add(self.afk_ms)
    }

    /// Wall-clock span in ms.
    #[must_use]
    pub const fn wallclock_ms(&self) -> i64 {
        self.ended_at.saturating_diff_ms(self.started_at)
    }
}

/// One ending the user reached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ending {
    /// Row identity and sync metadata.
    pub meta: SyncMeta,
    /// Owning game.
    pub game_id: EntityId,
    /// Ending name, e.g. "True End".
    pub name: String,
    /// Optional route label, e.g. "Mio".
    pub route: Option<String>,
    /// The route this ending belongs to, when the user linked it to one.
    ///
    /// `route` above stays as the free-text label for round-tripping and search;
    /// this is what the combined routes view groups on, so an ending and the
    /// chapters of the same route finally meet.
    pub route_id: Option<EntityId>,
    /// Whether the user has reached it.
    pub achieved: bool,
    /// When it was reached.
    pub achieved_at: Option<Timestamp>,
}

impl Ending {
    /// Build a new ending record.
    ///
    /// # Errors
    /// Fails when the name is blank or over-long.
    pub fn new(
        device: &DeviceId,
        now: Timestamp,
        game_id: EntityId,
        name: impl Into<String>,
    ) -> Result<Self, ModelError> {
        let name = name.into();
        let len = name.trim().chars().count();
        if len == 0 {
            return Err(ModelError::EmptyTitle);
        }
        if len > 200 {
            return Err(ModelError::TextTooLong {
                field: "ending name",
                len,
                max: 200,
            });
        }
        Ok(Self {
            meta: SyncMeta::new(device, now),
            game_id,
            name: name.trim().to_owned(),
            route: None,
            route_id: None,
            achieved: false,
            achieved_at: None,
        })
    }
}

/// Where a [`Route`] row came from.
///
/// Backfill creates rows from both sides without linking them, and the user needs
/// to see which is which before merging two halves of one story.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteOrigin {
    /// Derived from an engine-reported chapter route.
    Engine,
    /// Derived from a free-text label on an ending.
    Ending,
    /// Created by the user directly.
    Manual,
}

impl RouteOrigin {
    /// The string stored in the `routes.origin` column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Engine => "engine",
            Self::Ending => "ending",
            Self::Manual => "manual",
        }
    }

    /// Parse a stored value, defaulting to `Manual` for anything unrecognised.
    ///
    /// Defaulting rather than failing is deliberate: `origin` is presentation
    /// metadata, and a row written by a newer build should still list correctly on
    /// an older one instead of disappearing from the route view.
    #[must_use]
    pub fn from_stored(value: &str) -> Self {
        match value {
            "engine" => Self::Engine,
            "ending" => Self::Ending,
            _ => Self::Manual,
        }
    }
}

/// A route: the unit a chapter and an ending both belong to.
///
/// Before this existed a route was modelled twice -- an engine key on `chapters`
/// and a free-text label on `endings` -- so one story was grouped two different
/// ways with nothing tying the halves together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    /// Row identity and sync metadata.
    pub meta: SyncMeta,
    /// Owning game.
    pub game_id: EntityId,
    /// What to show, e.g. "Shiroha".
    pub name: String,
    /// Presentation order; `None` sorts last.
    pub sort_order: Option<i32>,
    /// Which side this row was derived from.
    pub origin: RouteOrigin,
    /// The surviving route when the user merged this one into another.
    ///
    /// Merging writes a pointer instead of deleting: folder sync merges by id, so a
    /// delete could be resurrected by a device that had not seen it yet, while a
    /// pointer travels with the row and both devices converge.
    pub merged_into: Option<EntityId>,
}

impl Route {
    /// Build a new route.
    ///
    /// # Errors
    /// Fails when the name is blank or over-long.
    pub fn new(
        device: &DeviceId,
        now: Timestamp,
        game_id: EntityId,
        name: impl Into<String>,
    ) -> Result<Self, ModelError> {
        let name = name.into();
        let len = name.trim().chars().count();
        if len == 0 {
            return Err(ModelError::EmptyTitle);
        }
        if len > 200 {
            return Err(ModelError::TextTooLong {
                field: "route name",
                len,
                max: 200,
            });
        }
        Ok(Self {
            meta: SyncMeta::new(device, now),
            game_id,
            name: name.trim().to_owned(),
            sort_order: None,
            origin: RouteOrigin::Manual,
            merged_into: None,
        })
    }
}

/// App-level preferences that are local, not synced (device-specific).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalPrefs {
    /// Light or dark.
    pub theme: ThemeMode,
    /// Interface language tag.
    pub language: String,
    /// Art visibility policy.
    pub nsfw: NsfwMode,
    /// Swap glass for solid surfaces.
    pub reduce_effects: bool,
}

impl Default for LocalPrefs {
    fn default() -> Self {
        Self {
            theme: ThemeMode::default(),
            language: "en".to_owned(),
            nsfw: NsfwMode::default(),
            reduce_effects: false,
        }
    }
}

/// Theme selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    /// Always dark.
    #[default]
    Dark,
    /// Always light.
    Light,
    /// Follow the OS.
    System,
}

impl ThemeMode {
    /// Stable storage key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
        }
    }

    /// Parse a storage key.
    ///
    /// # Errors
    /// Returns the unrecognised text when it is not a known mode.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "dark" => Ok(Self::Dark),
            "light" => Ok(Self::Light),
            "system" => Ok(Self::System),
            other => Err(other.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> DeviceId {
        DeviceId::parse("test-device").unwrap()
    }

    fn now() -> Timestamp {
        Timestamp::from_millis(1_700_000_000_000)
    }

    #[test]
    fn new_rows_carry_full_sync_metadata() {
        let game = Game::new(&device(), now(), "Symphonic Rain", GameStatus::Playing).unwrap();
        assert_eq!(game.meta.updated_at, now());
        assert_eq!(game.meta.device_id_device.device().as_str(), "test-device");
        assert!(!game.meta.deleted);
    }

    #[test]
    fn empty_title_rejected() {
        let err = Game::new(&device(), now(), "   ", GameStatus::Playing).unwrap_err();
        assert_eq!(err, ModelError::EmptyTitle);
    }

    #[test]
    fn over_long_title_rejected_at_char_boundary() {
        let title = "あ".repeat(Game::MAX_TITLE_CHARS + 1);
        let err = Game::new(&device(), now(), title, GameStatus::Playing).unwrap_err();
        assert!(matches!(
            err,
            ModelError::TextTooLong { field: "title", .. }
        ));
    }

    #[test]
    fn japanese_title_at_limit_is_accepted() {
        let title = "あ".repeat(Game::MAX_TITLE_CHARS);
        assert!(Game::new(&device(), now(), title, GameStatus::Playing).is_ok());
    }

    #[test]
    fn rating_must_be_one_to_ten() {
        let mut game = Game::new(&device(), now(), "A", GameStatus::Playing).unwrap();
        assert!(game.set_rating(Some(0)).is_err());
        assert!(game.set_rating(Some(9)).is_err());
        assert!(game.set_rating(Some(101)).is_err());
        assert!(game.set_rating(Some(10)).is_ok(), "1.0 is the floor");
        assert!(game.set_rating(Some(75)).is_ok(), "7.5 must be storable");
        assert!(game.set_rating(Some(100)).is_ok());
        assert!(game.set_rating(None).is_ok());
    }

    #[test]
    fn explicit_detection_respects_mode() {
        let mut game = Game::new(&device(), now(), "A", GameStatus::Playing).unwrap();
        game.cover_sexual = Some(3);
        assert!(game.is_explicit(NsfwMode::Blur));
        assert!(game.is_explicit(NsfwMode::Hide));
        assert!(!game.is_explicit(NsfwMode::Show));

        // Level 1 is concealed. This assertion used to demand the opposite
        // (`assert!(!is_explicit)` for a 1/1 cover), which pinned the flooring bug as
        // intent: the library's threshold was `>= 2` while the screenshot gallery's was
        // `>= 1`, so the same scene was shown on a card and blurred in the gallery.
        game.cover_sexual = Some(1);
        game.cover_violence = Some(1);
        assert!(game.is_explicit(NsfwMode::Blur));

        game.cover_violence = Some(2);
        assert!(game.is_explicit(NsfwMode::Blur));

        // The only safe level is 0, and an unrated row is not concealed in-app: a
        // cached game predating these columns must still render its art.
        game.cover_sexual = Some(0);
        game.cover_violence = Some(0);
        assert!(!game.is_explicit(NsfwMode::Blur));
        game.cover_sexual = None;
        game.cover_violence = None;
        assert!(!game.is_explicit(NsfwMode::Blur));
    }

    #[test]
    fn a_cover_and_a_screenshot_at_the_same_level_are_concealed_together() {
        // The rule this whole change exists to enforce: one threshold owns both. A cover
        // is rated on `cover_sexual`/`cover_violence`, a screenshot on its own votes
        // rounded by `dto::flag_level`; both then go through this comparison.
        for level in 0..=2u8 {
            let at_level = is_explicit_at_level(Some(level));
            assert_eq!(at_level, level >= EXPLICIT_FLAG_LEVEL);
        }
        assert!(is_explicit_at_level(Some(1)));
        assert!(is_explicit_at_level(Some(2)));
        assert!(!is_explicit_at_level(Some(0)));
        assert!(!is_explicit_at_level(None));
    }

    #[test]
    fn the_explicit_level_is_one() {
        // Pinned so a future regression has to argue with a test rather than quietly
        // raise the bar back to a unanimous vote, which is what made the filter inert.
        assert_eq!(EXPLICIT_FLAG_LEVEL, 1);
    }

    #[test]
    fn nsfw_default_is_blur() {
        assert_eq!(NsfwMode::default(), NsfwMode::Blur);
    }

    #[test]
    fn playtime_includes_manual_adjustment() {
        let mut game = Game::new(&device(), now(), "A", GameStatus::Playing).unwrap();
        game.manual_playtime_ms = 3_600_000;
        assert_eq!(game.total_playtime_ms(1_800_000), 5_400_000);
        game.manual_playtime_ms = -10_000;
        assert_eq!(game.total_playtime_ms(5_000), 0);
    }

    #[test]
    fn session_rejects_inverted_range() {
        let id = EntityId::new();
        let err = PlaySession::new(
            &device(),
            now(),
            id,
            Timestamp::from_millis(2_000),
            Timestamp::from_millis(1_000),
            1_000,
            SessionSource::Desktop,
        )
        .unwrap_err();
        assert_eq!(err, ModelError::SessionEndsBeforeStart);
    }

    #[test]
    fn session_rejects_negative_counted_time() {
        let id = EntityId::new();
        let err = PlaySession::new(
            &device(),
            now(),
            id,
            Timestamp::from_millis(1_000),
            Timestamp::from_millis(2_000),
            -1,
            SessionSource::Desktop,
        )
        .unwrap_err();
        assert_eq!(err, ModelError::NegativePlaytime(-1));
    }

    #[test]
    fn afk_time_is_recorded_separately_from_active_time() {
        let id = EntityId::new();
        let session = PlaySession::new(
            &device(),
            now(),
            id,
            Timestamp::from_millis(0),
            Timestamp::from_millis(100_000),
            40_000,
            SessionSource::Desktop,
        )
        .unwrap()
        .with_afk(60_000)
        .unwrap();
        assert_eq!(session.counted_ms, 40_000);
        assert_eq!(session.afk_ms, 60_000);
        assert_eq!(session.elapsed_ms(), 100_000);
    }

    #[test]
    fn negative_afk_is_rejected() {
        let id = EntityId::new();
        let session = PlaySession::new(
            &device(),
            now(),
            id,
            Timestamp::from_millis(0),
            Timestamp::from_millis(1_000),
            1_000,
            SessionSource::Desktop,
        )
        .unwrap();
        assert_eq!(
            session.with_afk(-1).unwrap_err(),
            ModelError::NegativePlaytime(-1)
        );
    }

    #[test]
    fn zero_length_session_is_allowed() {
        let id = EntityId::new();
        let session = PlaySession::new(
            &device(),
            now(),
            id,
            Timestamp::from_millis(1_000),
            Timestamp::from_millis(1_000),
            0,
            SessionSource::Manual,
        )
        .unwrap();
        assert_eq!(session.wallclock_ms(), 0);
    }

    #[test]
    fn counted_time_can_be_less_than_wallclock() {
        // Idle time is excluded, so this is a legitimate shape.
        let id = EntityId::new();
        let session = PlaySession::new(
            &device(),
            now(),
            id,
            Timestamp::from_millis(0),
            Timestamp::from_millis(60_000),
            30_000,
            SessionSource::Desktop,
        )
        .unwrap();
        assert_eq!(session.counted_ms, 30_000);
        assert_eq!(session.wallclock_ms(), 60_000);
    }

    #[test]
    fn status_round_trips_through_storage_keys() {
        for status in GameStatus::ALL {
            assert_eq!(GameStatus::parse(status.as_str()), Ok(status));
        }
        assert!(GameStatus::parse("nonsense").is_err());
    }

    #[test]
    fn session_source_round_trips() {
        for source in [
            SessionSource::Desktop,
            SessionSource::AndroidUsage,
            SessionSource::Manual,
            SessionSource::Import,
        ] {
            assert_eq!(SessionSource::parse(source.as_str()), Ok(source));
        }
    }

    #[test]
    fn theme_round_trips() {
        for mode in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::System] {
            assert_eq!(ThemeMode::parse(mode.as_str()), Ok(mode));
        }
    }

    #[test]
    fn tombstone_keeps_identity_and_marks_deleted() {
        let meta = SyncMeta::new(&device(), now());
        let dead = meta.tombstone(Timestamp::from_millis(9_999));
        assert_eq!(dead.id, meta.id);
        assert!(dead.deleted);
        assert_eq!(dead.updated_at, Timestamp::from_millis(9_999));
    }

    #[test]
    fn touch_updates_timestamp_only() {
        let meta = SyncMeta::new(&device(), now());
        let touched = meta.touch(Timestamp::from_millis(5));
        assert_eq!(touched.id, meta.id);
        assert_eq!(touched.updated_at, Timestamp::from_millis(5));
        assert!(!touched.deleted);
    }

    #[test]
    fn chapter_rejects_blank_name() {
        let game_id = EntityId::new();
        let err = Chapter::new(&device(), now(), game_id, "  ", 0).unwrap_err();
        assert_eq!(err, ModelError::EmptyTitle);
    }

    #[test]
    fn ending_rejects_blank_name() {
        let game_id = EntityId::new();
        assert!(Ending::new(&device(), now(), game_id, "").is_err());
    }
}
