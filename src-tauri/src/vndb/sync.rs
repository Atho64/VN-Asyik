//! Sync bookkeeping: the connected account, per-VN sync state, the push queue,
//! the label mapping and the change log.
//!
//! Nothing in this module knows about HTTP; it only persists decisions the pure
//! mapping layer makes, so a failed push never loses the intent to retry.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use vn_core::clock::Timestamp;

use crate::db::DbError;
use crate::vndb::mapping::{ConflictPolicy, LabelMapping};
use crate::vndb::ulist::UlistPatch;

/// The connected VNDB account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// User id, `u123`.
    pub user_id: String,
    /// Username.
    pub username: String,
    /// Permissions granted to the token.
    pub permissions: Vec<String>,
    /// When the account was connected.
    pub connected_at: i64,
    /// Last successful pull, UTC ms.
    pub last_pull_at: Option<i64>,
    /// Last successful push, UTC ms.
    pub last_push_at: Option<i64>,
}

impl Account {
    /// Whether the token can read the list, including private labels.
    #[must_use]
    pub fn can_read(&self) -> bool {
        self.permissions.iter().any(|p| p == "listread")
    }

    /// Whether the token can modify the list.
    #[must_use]
    pub fn can_write(&self) -> bool {
        self.permissions.iter().any(|p| p == "listwrite")
    }
}

/// What VNDB last reported for one VN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncState {
    /// VN id.
    pub vn_id: String,
    /// Local game this VN is attached to, when imported.
    pub game_id: Option<String>,
    /// Remote `lastmod`, UTC ms.
    pub remote_lastmod: i64,
    /// Remote vote in decivotes.
    pub remote_vote: Option<u32>,
    /// Remote notes.
    pub remote_notes: Option<String>,
    /// Remote start date, `YYYY-MM-DD`.
    pub remote_started: Option<String>,
    /// Remote finish date, `YYYY-MM-DD`.
    pub remote_finished: Option<String>,
    /// Remote label ids.
    pub remote_labels: Vec<i64>,
    /// When this row was written, UTC ms.
    pub pulled_at: i64,
}

/// A queued push for one VN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedPush {
    /// VN id.
    pub vn_id: String,
    /// Local game.
    pub game_id: String,
    /// The patch describing the desired end state.
    pub patch: UlistPatch,
    /// When it was queued, UTC ms.
    pub created_at: i64,
    /// How many times pushing has been attempted.
    pub attempts: i64,
    /// Last failure, for the UI.
    pub last_error: Option<String>,
}

/// Kinds written to the sync log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    /// A pull completed.
    Pull,
    /// A push completed.
    Push,
    /// A conflict was found.
    Conflict,
    /// Something failed.
    Error,
    /// The account was connected.
    Connect,
    /// The account was disconnected.
    Disconnect,
}

impl LogKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pull => "pull",
            Self::Push => "push",
            Self::Conflict => "conflict",
            Self::Error => "error",
            Self::Connect => "connect",
            Self::Disconnect => "disconnect",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "pull" => Some(Self::Pull),
            "push" => Some(Self::Push),
            "conflict" => Some(Self::Conflict),
            "error" => Some(Self::Error),
            "connect" => Some(Self::Connect),
            "disconnect" => Some(Self::Disconnect),
            _ => None,
        }
    }
}

/// One log line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Row id.
    pub id: i64,
    /// When it happened, UTC ms.
    pub at: i64,
    /// What kind of event.
    pub kind: String,
    /// The VN concerned, when relevant.
    pub vn_id: Option<String>,
    /// Human-readable message.
    pub message: String,
}

// account
/// Read the connected account, if any.
pub async fn load_account(pool: &SqlitePool) -> Result<Option<Account>, DbError> {
    let row: Option<AccountRow> = sqlx::query_as(
        "SELECT user_id, username, permissions, connected_at, last_pull_at, last_push_at \
         FROM vndb_account WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Account::from))
}

/// Database row for the connected account.
#[derive(Debug, sqlx::FromRow)]
struct AccountRow {
    user_id: String,
    username: String,
    permissions: String,
    connected_at: i64,
    last_pull_at: Option<i64>,
    last_push_at: Option<i64>,
}

impl From<AccountRow> for Account {
    fn from(row: AccountRow) -> Self {
        Self {
            user_id: row.user_id,
            username: row.username,
            permissions: serde_json::from_str(&row.permissions).unwrap_or_default(),
            connected_at: row.connected_at,
            last_pull_at: row.last_pull_at,
            last_push_at: row.last_push_at,
        }
    }
}

/// Store (or replace) the connected account.
pub async fn save_account(pool: &SqlitePool, account: &Account) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO vndb_account (id, user_id, username, permissions, connected_at, last_pull_at, last_push_at) \
         VALUES (1, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET user_id = excluded.user_id, username = excluded.username, \
            permissions = excluded.permissions, connected_at = excluded.connected_at, \
            last_pull_at = excluded.last_pull_at, last_push_at = excluded.last_push_at",
    )
    .bind(&account.user_id)
    .bind(&account.username)
    .bind(serde_json::to_string(&account.permissions).unwrap_or_else(|_| "[]".to_owned()))
    .bind(account.connected_at)
    .bind(account.last_pull_at)
    .bind(account.last_push_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a successful pull or push timestamp.
pub async fn touch_account(
    pool: &SqlitePool,
    pulled_at: Option<Timestamp>,
    pushed_at: Option<Timestamp>,
) -> Result<(), DbError> {
    if let Some(at) = pulled_at {
        sqlx::query("UPDATE vndb_account SET last_pull_at = ? WHERE id = 1")
            .bind(at.as_millis())
            .execute(pool)
            .await?;
    }
    if let Some(at) = pushed_at {
        sqlx::query("UPDATE vndb_account SET last_push_at = ? WHERE id = 1")
            .bind(at.as_millis())
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Forget the account and everything derived from it.
///
/// The push queue and sync state go too: they describe a relationship that no
/// longer exists. Games and their playtime are untouched, because they are the
/// user's data, not VNDB's.
pub async fn clear_account(pool: &SqlitePool) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    for statement in [
        "DELETE FROM vndb_account",
        "DELETE FROM vndb_sync_state",
        "DELETE FROM vndb_push_queue",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

// sync state
/// Read the sync state for a VN.
pub async fn load_state(pool: &SqlitePool, vn_id: &str) -> Result<Option<SyncState>, DbError> {
    let row: Option<SyncStateRow> = sqlx::query_as(
        "SELECT vn_id, game_id, remote_lastmod, remote_vote, remote_notes, remote_started, \
                remote_finished, remote_labels, pulled_at \
         FROM vndb_sync_state WHERE vn_id = ?",
    )
    .bind(vn_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(SyncState::from))
}

/// Database row for one VN's sync state.
#[derive(Debug, sqlx::FromRow)]
struct SyncStateRow {
    vn_id: String,
    game_id: Option<String>,
    remote_lastmod: i64,
    remote_vote: Option<i64>,
    remote_notes: Option<String>,
    remote_started: Option<String>,
    remote_finished: Option<String>,
    remote_labels: String,
    pulled_at: i64,
}

impl From<SyncStateRow> for SyncState {
    fn from(row: SyncStateRow) -> Self {
        Self {
            vn_id: row.vn_id,
            game_id: row.game_id,
            remote_lastmod: row.remote_lastmod,
            remote_vote: row.remote_vote.and_then(|value| u32::try_from(value).ok()),
            remote_notes: row.remote_notes,
            remote_started: row.remote_started,
            remote_finished: row.remote_finished,
            remote_labels: serde_json::from_str(&row.remote_labels).unwrap_or_default(),
            pulled_at: row.pulled_at,
        }
    }
}

/// Write the sync state for a VN.
pub async fn save_state(pool: &SqlitePool, state: &SyncState) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO vndb_sync_state (vn_id, game_id, remote_lastmod, remote_vote, remote_notes, \
            remote_started, remote_finished, remote_labels, pulled_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(vn_id) DO UPDATE SET game_id = excluded.game_id, \
            remote_lastmod = excluded.remote_lastmod, remote_vote = excluded.remote_vote, \
            remote_notes = excluded.remote_notes, remote_started = excluded.remote_started, \
            remote_finished = excluded.remote_finished, remote_labels = excluded.remote_labels, \
            pulled_at = excluded.pulled_at",
    )
    .bind(&state.vn_id)
    .bind(&state.game_id)
    .bind(state.remote_lastmod)
    .bind(state.remote_vote.map(i64::from))
    .bind(&state.remote_notes)
    .bind(&state.remote_started)
    .bind(&state.remote_finished)
    .bind(serde_json::to_string(&state.remote_labels).unwrap_or_else(|_| "[]".to_owned()))
    .bind(state.pulled_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Every sync-state row, for a batch apply.
pub async fn all_states(pool: &SqlitePool) -> Result<Vec<SyncState>, DbError> {
    let rows: Vec<String> = sqlx::query_as("SELECT vn_id FROM vndb_sync_state")
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(vn_id,)| vn_id)
        .collect();

    let mut states = Vec::with_capacity(rows.len());
    for vn_id in rows {
        if let Some(state) = load_state(pool, &vn_id).await? {
            states.push(state);
        }
    }
    Ok(states)
}

/// A queued push as shown in the UI.
///
/// Deliberately omits the patch itself: the UI needs to know that something is
/// waiting and why it failed, not the wire payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntryView {
    /// VN id.
    pub vn_id: String,
    /// Local game.
    pub game_id: String,
    /// When queued, UTC ms.
    pub created_at: i64,
    /// Failed attempts so far.
    pub attempts: i64,
    /// Last failure message.
    pub last_error: Option<String>,
}

// settings
/// Read the label mapping and conflict policy.
pub async fn load_settings(pool: &SqlitePool) -> Result<(LabelMapping, ConflictPolicy), DbError> {
    let row: SyncSettingsRow = sqlx::query_as(
        "SELECT label_playing, label_completed, label_on_hold, label_dropped, label_wishlist, policy \
         FROM vndb_sync_settings WHERE id = 1",
    )
    .fetch_one(pool)
    .await?;

    let mapping = LabelMapping {
        playing: row.label_playing,
        completed: row.label_completed,
        on_hold: row.label_on_hold,
        dropped: row.label_dropped,
        wishlist: row.label_wishlist,
    };
    let policy = match row.policy.as_str() {
        "prefer_local" => ConflictPolicy::PreferLocal,
        "prefer_remote" => ConflictPolicy::PreferRemote,
        "newest" => ConflictPolicy::Newest,
        _ => ConflictPolicy::Ask,
    };
    Ok((mapping, policy))
}

/// Database row for the sync settings.
#[derive(Debug, sqlx::FromRow)]
struct SyncSettingsRow {
    label_playing: i64,
    label_completed: i64,
    label_on_hold: i64,
    label_dropped: i64,
    label_wishlist: i64,
    policy: String,
}

/// Persist the label mapping and conflict policy.
pub async fn save_settings(
    pool: &SqlitePool,
    mapping: &LabelMapping,
    policy: ConflictPolicy,
    now: Timestamp,
) -> Result<(), DbError> {
    let policy_text = match policy {
        ConflictPolicy::Ask => "ask",
        ConflictPolicy::PreferLocal => "prefer_local",
        ConflictPolicy::PreferRemote => "prefer_remote",
        ConflictPolicy::Newest => "newest",
    };
    sqlx::query(
        "UPDATE vndb_sync_settings SET label_playing = ?, label_completed = ?, \
            label_on_hold = ?, label_dropped = ?, label_wishlist = ?, policy = ?, \
            updated_at = ? WHERE id = 1",
    )
    .bind(mapping.playing)
    .bind(mapping.completed)
    .bind(mapping.on_hold)
    .bind(mapping.dropped)
    .bind(mapping.wishlist)
    .bind(policy_text)
    .bind(now.as_millis())
    .execute(pool)
    .await?;
    Ok(())
}

// queue
/// Queue (or replace) a push for a VN.
pub async fn queue_push(
    pool: &SqlitePool,
    vn_id: &str,
    game_id: &str,
    patch: &UlistPatch,
    now: Timestamp,
) -> Result<(), DbError> {
    let json = serde_json::to_string(patch)
        .map_err(|err| DbError::Corrupt(format!("patch could not be encoded: {err}")))?;
    sqlx::query(
        "INSERT INTO vndb_push_queue (vn_id, game_id, patch_json, created_at, attempts, last_error) \
         VALUES (?, ?, ?, ?, 0, NULL) \
         ON CONFLICT(vn_id) DO UPDATE SET game_id = excluded.game_id, \
            patch_json = excluded.patch_json, created_at = excluded.created_at, attempts = 0, \
            last_error = NULL",
    )
    .bind(vn_id)
    .bind(game_id)
    .bind(json)
    .bind(now.as_millis())
    .execute(pool)
    .await?;
    Ok(())
}

/// Every queued push, oldest first.
pub async fn list_queue(pool: &SqlitePool) -> Result<Vec<QueuedPush>, DbError> {
    let rows: Vec<(String, String, String, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT vn_id, game_id, patch_json, created_at, attempts, last_error \
         FROM vndb_push_queue ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(
            |(vn_id, game_id, patch_json, created_at, attempts, last_error)| {
                let patch: UlistPatch = serde_json::from_str(&patch_json).ok()?;
                Some(QueuedPush {
                    vn_id,
                    game_id,
                    patch,
                    created_at,
                    attempts,
                    last_error,
                })
            },
        )
        .collect())
}

/// Drop a queued push, after it succeeded.
pub async fn remove_push(pool: &SqlitePool, vn_id: &str) -> Result<(), DbError> {
    sqlx::query("DELETE FROM vndb_push_queue WHERE vn_id = ?")
        .bind(vn_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Record a failed push attempt so the UI can show it and backoff can grow.
pub async fn record_push_failure(
    pool: &SqlitePool,
    vn_id: &str,
    error: &str,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE vndb_push_queue SET attempts = attempts + 1, last_error = ? WHERE vn_id = ?",
    )
    .bind(error)
    .bind(vn_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// How many pushes are waiting.
pub async fn queue_len(pool: &SqlitePool) -> Result<i64, DbError> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vndb_push_queue")
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

// log
/// Append a log line.
pub async fn log(
    pool: &SqlitePool,
    kind: LogKind,
    vn_id: Option<&str>,
    message: &str,
    now: Timestamp,
) -> Result<(), DbError> {
    sqlx::query("INSERT INTO vndb_sync_log (at, kind, vn_id, message) VALUES (?, ?, ?, ?)")
        .bind(now.as_millis())
        .bind(kind.as_str())
        .bind(vn_id)
        .bind(message)
        .execute(pool)
        .await?;
    Ok(())
}

/// Read the most recent log lines, newest first.
pub async fn recent_log(pool: &SqlitePool, limit: i64) -> Result<Vec<LogEntry>, DbError> {
    let rows: Vec<(i64, i64, String, Option<String>, String)> = sqlx::query_as(
        "SELECT id, at, kind, vn_id, message FROM vndb_sync_log ORDER BY at DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|(id, at, kind, vn_id, message)| {
            Some(LogEntry {
                id,
                at,
                kind: LogKind::parse(&kind)?.as_str().to_owned(),
                vn_id,
                message,
            })
        })
        .collect())
}

/// Trim the log to its newest `keep` rows.
pub async fn trim_log(pool: &SqlitePool, keep: i64) -> Result<(), DbError> {
    sqlx::query(
        "DELETE FROM vndb_sync_log WHERE id NOT IN \
         (SELECT id FROM vndb_sync_log ORDER BY at DESC LIMIT ?)",
    )
    .bind(keep.clamp(50, 10_000))
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn now() -> Timestamp {
        Timestamp::from_millis(1_700_000_000_000)
    }

    async fn fixture() -> Db {
        Db::open_memory().await.expect("open")
    }

    #[tokio::test]
    async fn account_round_trips_with_permissions() {
        let db = fixture().await;
        assert!(load_account(db.pool()).await.expect("load").is_none());

        let account = Account {
            user_id: "u3".to_owned(),
            username: "ayo".to_owned(),
            permissions: vec!["listread".to_owned(), "listwrite".to_owned()],
            connected_at: now().as_millis(),
            last_pull_at: None,
            last_push_at: None,
        };
        save_account(db.pool(), &account).await.expect("save");

        let loaded = load_account(db.pool())
            .await
            .expect("load")
            .expect("present");
        assert_eq!(loaded, account);
        assert!(loaded.can_read());
        assert!(loaded.can_write());
    }

    #[tokio::test]
    async fn read_only_token_is_reported_as_such() {
        let db = fixture().await;
        let account = Account {
            user_id: "u3".to_owned(),
            username: "ayo".to_owned(),
            permissions: vec!["listread".to_owned()],
            connected_at: 1,
            last_pull_at: None,
            last_push_at: None,
        };
        save_account(db.pool(), &account).await.expect("save");
        let loaded = load_account(db.pool())
            .await
            .expect("load")
            .expect("present");
        assert!(loaded.can_read());
        assert!(!loaded.can_write());
    }

    #[tokio::test]
    async fn disconnecting_clears_sync_data_but_not_games() {
        let db = fixture().await;
        sqlx::query(
            "INSERT INTO games (id, title, status, updated_at, device_id) VALUES ('g1','A','playing',1,'dev')",
        )
        .execute(db.pool())
        .await
        .expect("game");

        let account = Account {
            user_id: "u3".to_owned(),
            username: "ayo".to_owned(),
            permissions: vec![],
            connected_at: 1,
            last_pull_at: None,
            last_push_at: None,
        };
        save_account(db.pool(), &account).await.expect("save");
        queue_push(db.pool(), "v17", "g1", &UlistPatch::default(), now())
            .await
            .expect("queue");

        clear_account(db.pool()).await.expect("clear");

        assert!(load_account(db.pool()).await.expect("load").is_none());
        assert_eq!(queue_len(db.pool()).await.expect("len"), 0);
        let games: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM games")
            .fetch_one(db.pool())
            .await
            .expect("count");
        assert_eq!(games.0, 1, "the user's own library must survive");
    }

    #[tokio::test]
    async fn sync_state_round_trips() {
        let db = fixture().await;
        let state = SyncState {
            vn_id: "v17".to_owned(),
            game_id: Some("g1".to_owned()),
            remote_lastmod: 1_600_000_000_000,
            remote_vote: Some(80),
            remote_notes: Some("notes".to_owned()),
            remote_started: Some("2020-09-01".to_owned()),
            remote_finished: None,
            remote_labels: vec![1, 7],
            pulled_at: now().as_millis(),
        };
        save_state(db.pool(), &state).await.expect("save");
        let loaded = load_state(db.pool(), "v17")
            .await
            .expect("load")
            .expect("present");
        assert_eq!(loaded, state);
    }

    #[tokio::test]
    async fn settings_default_to_the_documented_labels_and_ask_policy() {
        let db = fixture().await;
        let (mapping, policy) = load_settings(db.pool()).await.expect("load");
        assert_eq!(mapping, LabelMapping::default());
        assert_eq!(policy, ConflictPolicy::Ask);
    }

    #[tokio::test]
    async fn settings_persist_a_custom_mapping() {
        let db = fixture().await;
        let mapping = LabelMapping {
            playing: 12,
            wishlist: 13,
            ..LabelMapping::default()
        };
        save_settings(db.pool(), &mapping, ConflictPolicy::Newest, now())
            .await
            .expect("save");

        let (loaded, policy) = load_settings(db.pool()).await.expect("load");
        assert_eq!(loaded.playing, 12);
        assert_eq!(loaded.wishlist, 13);
        assert_eq!(policy, ConflictPolicy::Newest);
    }

    #[tokio::test]
    async fn queue_replaces_rather_than_appends_per_vn() {
        let db = fixture().await;
        let first = UlistPatch {
            vote: Some(Some(70)),
            ..UlistPatch::default()
        };
        queue_push(db.pool(), "v17", "g1", &first, now())
            .await
            .expect("q1");

        let second = UlistPatch {
            vote: Some(Some(90)),
            ..UlistPatch::default()
        };
        queue_push(db.pool(), "v17", "g1", &second, now())
            .await
            .expect("q2");

        let queue = list_queue(db.pool()).await.expect("list");
        assert_eq!(queue.len(), 1, "one row per VN");
        assert_eq!(queue[0].patch.vote, Some(Some(90)), "newest edit wins");
    }

    #[tokio::test]
    async fn failures_are_counted_and_cleared_on_replace() {
        let db = fixture().await;
        let patch = UlistPatch {
            vote: Some(Some(70)),
            ..UlistPatch::default()
        };
        queue_push(db.pool(), "v17", "g1", &patch, now())
            .await
            .expect("q");
        record_push_failure(db.pool(), "v17", "offline")
            .await
            .expect("fail");
        record_push_failure(db.pool(), "v17", "offline")
            .await
            .expect("fail");

        let queue = list_queue(db.pool()).await.expect("list");
        assert_eq!(queue[0].attempts, 2);
        assert_eq!(queue[0].last_error.as_deref(), Some("offline"));

        queue_push(db.pool(), "v17", "g1", &patch, now())
            .await
            .expect("q2");
        let queue = list_queue(db.pool()).await.expect("list");
        assert_eq!(queue[0].attempts, 0);
        assert!(queue[0].last_error.is_none());
    }

    #[tokio::test]
    async fn removing_a_push_empties_the_queue() {
        let db = fixture().await;
        queue_push(db.pool(), "v17", "g1", &UlistPatch::default(), now())
            .await
            .expect("q");
        assert_eq!(queue_len(db.pool()).await.expect("len"), 1);
        remove_push(db.pool(), "v17").await.expect("remove");
        assert_eq!(queue_len(db.pool()).await.expect("len"), 0);
    }

    #[tokio::test]
    async fn log_is_newest_first_and_trimmable() {
        let db = fixture().await;
        for index in 0..10 {
            log(
                db.pool(),
                LogKind::Pull,
                Some("v17"),
                &format!("line {index}"),
                Timestamp::from_millis(1_000 + index),
            )
            .await
            .expect("log");
        }
        let recent = recent_log(db.pool(), 3).await.expect("recent");
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].message, "line 9");

        trim_log(db.pool(), 50).await.expect("trim");
        let all = recent_log(db.pool(), 100).await.expect("all");
        assert_eq!(all.len(), 10, "trim keeps everything below the floor");
    }

    #[tokio::test]
    async fn log_limits_its_page_size() {
        let db = fixture().await;
        let entries = recent_log(db.pool(), 10_000).await.expect("recent");
        assert!(entries.is_empty());
    }
}
