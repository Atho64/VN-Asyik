//! Explicit per-game Android attribution from the foreground usage event log.
//!
//! Play selects the library game; `UsageStatsManager` supplies the package's
//! foreground intervals. A shared runner cannot identify its current ROM.
//! Query replay starts at Play so an activity spanning polls keeps its context;
//! the persisted cursor clips already credited time. Session rows and the cursor
//! commit together. Denied/missing usage data never becomes a timer estimate.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};
use tokio::sync::Mutex;
use vn_core::ids::{DeviceId, EntityId};
use vn_core::usage::{UsageEvent, intervals_for};

use crate::db::DbError;
use crate::platform::android::{
    AndroidUsage, AndroidUsageError, RawUsageEvent, SELF_PACKAGE_MARKER,
};
use crate::tracking::mapping::{self, GameExecutable};

const ACTIVE_SELECTION_KEY: &str = "android.activeSelection";
const LAST_IMPORTED_KEY: &str = "android.lastImportedAtMs";
// Android retains usage events for only a few days (SDK UsageStatsManager docs).
const REPLAY_WINDOW_MS: i64 = 3 * 24 * 60 * 60 * 1_000;

/// Serialises launch, foreground return, and manual stop so concurrent IPC calls
/// cannot commit the same active session twice.
static SESSION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Persisted state for the game the user launched from VN Asyik.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSelection {
    pub game_id: String,
    pub package: String,
    #[serde(default)]
    pub runner_kind: Option<String>,
    pub started_at_ms: i64,
    pub cursor_ms: i64,
    pub counted_ms: i64,
    /// Explicit Stop is remembered even if usage permission/data is unavailable.
    #[serde(default)]
    pub stopped_at_ms: Option<i64>,
}

/// Summary returned to the UI when an Android session starts, updates, or reconciles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidSessionView {
    pub active: bool,
    pub game_id: Option<String>,
    pub game_title: Option<String>,
    pub package: Option<String>,
    pub runner_kind: Option<String>,
    pub started_at_ms: Option<i64>,
    pub counted_ms: i64,
    /// Persisted foreground playtime; never elapsed wall-clock time.
    pub elapsed_ms: i64,
    pub last_imported_at_ms: Option<i64>,
    pub reason: Option<String>,
}

/// Read the active Android selection without ending it.
pub async fn status(pool: &SqlitePool) -> Result<AndroidSessionView, DbError> {
    let selection = load_selection(pool).await?;
    let last_imported_at_ms = read_i64(pool, LAST_IMPORTED_KEY).await?;
    let Some(selection) = selection else {
        return Ok(AndroidSessionView {
            last_imported_at_ms,
            ..AndroidSessionView::default()
        });
    };

    let title = match EntityId::parse(&selection.game_id) {
        Ok(gid) => mapping::game_title(pool, gid).await?,
        Err(_) => None,
    };
    Ok(AndroidSessionView {
        active: selection.stopped_at_ms.is_none(),
        game_id: Some(selection.game_id),
        game_title: title,
        package: Some(selection.package),
        runner_kind: selection.runner_kind,
        started_at_ms: Some(selection.started_at_ms),
        counted_ms: selection.counted_ms,
        elapsed_ms: selection.counted_ms,
        last_imported_at_ms,
        reason: None,
    })
}

/// Active target queried by Discord presence loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAndroidTarget {
    pub game_id: EntityId,
    pub package: String,
    pub active_ms: i64,
    pub start_epoch: i64,
}

/// How stale the last counted foreground interval may be and still count as
/// "the game is on screen": one poll plus reconciliation slack. A quick
/// lock-screen glance survives it; a browser session or an overnight lock does
/// not, and the card dies on the next poll after that.
const PRESENCE_FOREGROUND_TOLERANCE_MS: i64 = 90_000;

/// Query the Android game whose card Discord should show, if any.
///
/// The open selection alone is not evidence the game is running: `UsageStats`
/// cannot see process death, so a selection survives the user going home or
/// locking the phone. The end of the last counted interval is the freshest
/// foreground proof — replay closes it at every pause, app switch and screen-off
/// — so the card lives only while that proof is recent. Elapsed is counted time,
/// never `now - started_at`: a surviving card must not advertise hours nobody
/// played.
pub async fn active_target(pool: &SqlitePool, now_ms: i64) -> Option<ActiveAndroidTarget> {
    let selection = load_selection(pool).await.ok().flatten()?;
    if selection.stopped_at_ms.is_some() {
        return None;
    }
    let game_id = EntityId::parse(&selection.game_id).ok()?;
    // Strictly after `started_at_ms`: a slice of this selection always ends
    // later than it began, while a previous selection's last session may end
    // exactly at the boundary and must not count as fresh foreground.
    let last_foreground: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(ended_at) FROM play_sessions \
         WHERE game_id = ? AND source = 'android_usage' AND ended_at > ?",
    )
    .bind(&selection.game_id)
    .bind(selection.started_at_ms)
    .fetch_one(pool)
    .await
    .ok()
    .flatten();
    let Some(last_foreground) = last_foreground else {
        // No counted interval yet: the game has not reached the foreground
        // (still launching) or its row is gone and nothing may be published.
        return None;
    };
    if now_ms - last_foreground > PRESENCE_FOREGROUND_TOLERANCE_MS {
        return None;
    }
    // `counted_ms` plus the uncommitted tail of the open interval. Between
    // reconciles the tail is at most one poll, so a card held through the
    // tolerance window may tick over non-play seconds by no more than that.
    let active_ms = selection.counted_ms + (now_ms - last_foreground).max(0);
    let start_epoch = (now_ms - active_ms) / 1000;
    Some(ActiveAndroidTarget {
        game_id,
        package: selection.package,
        active_ms,
        start_epoch,
    })
}

/// Require Usage Access, persist the explicit selection, then open its package.
pub async fn launch(
    pool: &SqlitePool,
    device: &DeviceId,
    target: &GameExecutable,
    now_ms: i64,
) -> Result<(), String> {
    let _guard = SESSION_LOCK.lock().await;
    let package = target
        .android_package()
        .ok_or_else(|| "mapping is not an Android package".to_owned())?
        .to_owned();
    let access = AndroidUsage::access()
        .await
        .map_err(|err| err.to_string())?;
    if !access.granted {
        return Err(AndroidUsageError::NotGranted.to_string());
    }

    // If a previous game session was still open, finish and commit it first so
    // switching games never merges two games' playtime together.
    if load_selection(pool)
        .await
        .map_err(|err| err.to_string())?
        .is_some()
    {
        reconcile_locked(pool, device, now_ms, true).await?;
    }

    let selection = ActiveSelection {
        game_id: target.game_id.clone(),
        package,
        runner_kind: Some(target.runner_kind.clone()),
        started_at_ms: now_ms,
        cursor_ms: now_ms,
        counted_ms: 0,
        stopped_at_ms: None,
    };
    save_selection(pool, &selection)
        .await
        .map_err(|err| err.to_string())?;
    // Persist before launching: process death after startActivity must still
    // leave a replayable selection. Failed launches accrue no time.
    if let Err(err) = AndroidUsage::launch_package(&selection.package).await {
        clear_selection(pool)
            .await
            .map_err(|db_err| db_err.to_string())?;
        return Err(err.to_string());
    }
    Ok(())
}

/// Replay foreground events; a recorded return or reboot ends the selection.
pub async fn reconcile(
    pool: &SqlitePool,
    device: &DeviceId,
    until_ms: i64,
) -> Result<AndroidSessionView, String> {
    let _guard = SESSION_LOCK.lock().await;
    reconcile_locked(pool, device, until_ms, false).await
}

/// Explicitly stop after importing the package's last foreground intervals.
pub async fn stop(
    pool: &SqlitePool,
    device: &DeviceId,
    until_ms: i64,
) -> Result<AndroidSessionView, String> {
    let _guard = SESSION_LOCK.lock().await;
    reconcile_locked(pool, device, until_ms, true).await
}

async fn reconcile_locked(
    pool: &SqlitePool,
    device: &DeviceId,
    until_ms: i64,
    force: bool,
) -> Result<AndroidSessionView, String> {
    let Some(mut selection) = load_selection(pool).await.map_err(|err| err.to_string())? else {
        return status(pool).await.map_err(|err| err.to_string());
    };
    if force && selection.stopped_at_ms.is_none() {
        selection.stopped_at_ms = Some(until_ms);
        save_selection(pool, &selection)
            .await
            .map_err(|err| err.to_string())?;
    }
    let until_ms = selection
        .stopped_at_ms
        .map_or(until_ms, |stop| stop.min(until_ms));
    let force = force || selection.stopped_at_ms.is_some();
    if until_ms <= selection.cursor_ms {
        if force {
            clear_selection(pool).await.map_err(|err| err.to_string())?;
        }
        return status(pool).await.map_err(|err| err.to_string());
    }
    if until_ms.saturating_sub(selection.started_at_ms) > REPLAY_WINDOW_MS {
        // Once return/reboot events may have expired, an old selection cannot
        // safely identify a later launch of the same shared package.
        clear_selection(pool).await.map_err(|err| err.to_string())?;
        let mut view = status(pool).await.map_err(|err| err.to_string())?;
        view.reason =
            Some("Android usage history expired; no unverified time was added".to_owned());
        return Ok(view);
    }
    let packages = vec![selection.package.clone()];
    let result = AndroidUsage::query(&packages, selection.started_at_ms, until_ms).await;
    apply_query_result(pool, device, selection, until_ms, force, result).await
}

/// Shared by the native bridge and regression tests so failures exercise the
/// actual persistence path, including missing permission and empty history.
async fn apply_query_result(
    pool: &SqlitePool,
    device: &DeviceId,
    mut selection: ActiveSelection,
    until_ms: i64,
    force: bool,
    result: Result<Vec<RawUsageEvent>, AndroidUsageError>,
) -> Result<AndroidSessionView, String> {
    let raw = result.map_err(|err| err.to_string())?;
    let mut events: Vec<UsageEvent> = raw
        .iter()
        .filter(|event| event.at_ms >= selection.started_at_ms && event.at_ms < until_ms)
        .map(RawUsageEvent::to_usage_event)
        .collect();
    events.sort_by_key(|event| event.at_ms);
    // Only a device boundary ends the selection.
    //
    // Our own `Resumed` used to count as one, on the theory that returning to VN
    // Asyik means the game is no longer in front. That is true, but it is not the
    // end of the *session*: the user launches a game, alt-tabs to check something,
    // and comes back to keep reading. Treating the return as an endpoint ended the
    // session on every trip away, so the next stretch never accrued -- while the
    // time spent in the other app was counted, because the game's interval stays
    // open until a different package resumes.
    //
    // The interval replay already handles the away stretch correctly: another
    // package resuming closes the game's interval. What was missing is stopping
    // *time* at that point, which is what the resumed-away cutoff below does.
    let boundary = events
        .iter()
        .find(|event| event.kind == vn_core::usage::UsageEventKind::DeviceBoundary);

    // The moment our own app took the foreground, if it did. Recording stops
    // here, because whatever runs after is the user in VN Asyik, not reading.
    //
    // Only a return *since the last commit* counts. The replay window always
    // starts at `started_at_ms`, so the previous round's return event is still in
    // it; taking that one capped the cutoff at a moment the cursor had already
    // passed, and every later stretch of the same session was silently dropped.
    let left_to_self = events.iter().find(|event| {
        event.kind == vn_core::usage::UsageEventKind::Resumed
            && event.package.as_deref() == Some(SELF_PACKAGE_MARKER)
            && event.at_ms > selection.cursor_ms
    });
    // Include the boundary event in the exclusive window: DEVICE_* must discard
    // an open activity rather than close it at an invented shutdown time.
    let cutoff = boundary.map_or(until_ms, |event| event.at_ms.saturating_add(1));
    let cutoff = left_to_self.map_or(cutoff, |event| cutoff.min(event.at_ms));
    let cutoff = selection
        .stopped_at_ms
        .map_or(cutoff, |stop| stop.min(cutoff));
    let keep_active = !force && boundary.is_none() && selection.stopped_at_ms.is_none();
    let title = match EntityId::parse(&selection.game_id) {
        Ok(gid) => mapping::game_title(pool, gid)
            .await
            .map_err(|err| err.to_string())?,
        Err(_) => None,
    };

    commit_events(pool, device, &mut selection, &events, cutoff, keep_active)
        .await
        .map_err(|err| err.to_string())?;
    Ok(AndroidSessionView {
        active: keep_active,
        game_id: Some(selection.game_id),
        game_title: title,
        package: Some(selection.package),
        runner_kind: selection.runner_kind,
        started_at_ms: Some(selection.started_at_ms),
        counted_ms: selection.counted_ms,
        elapsed_ms: selection.counted_ms,
        last_imported_at_ms: Some(cutoff),
        reason: None,
    })
}

/// Apply foreground events for the active selection in one transaction.
///
/// Cursor changes become visible only after the session rows commit successfully.
pub async fn commit_events(
    pool: &SqlitePool,
    device: &DeviceId,
    selection: &mut ActiveSelection,
    events: &[UsageEvent],
    until_ms: i64,
    keep_active: bool,
) -> Result<i64, DbError> {
    if until_ms <= selection.cursor_ms {
        if !keep_active {
            clear_selection(pool).await?;
        }
        return Ok(0);
    }

    let packages = vec![selection.package.clone()];
    let intervals = intervals_for(&packages, events, until_ms);
    let mut tx = pool.begin().await?;
    let mut added_ms = 0_i64;

    if game_is_live(&mut tx, &selection.game_id).await? {
        for interval in intervals {
            let started_at = interval.start_ms.max(selection.cursor_ms);
            let ended_at = interval.end_ms.min(until_ms);
            let counted_ms = ended_at - started_at;
            // A glance is not a session. `UsageStats` reports every foreground
            // switch, so without this floor the list fills with `0:00` rows for
            // games that were never played (see `MIN_SESSION_MS`).
            if !vn_core::usage::is_session_worth_recording(counted_ms) {
                continue;
            }
            insert_android_session(
                &mut tx,
                device,
                &selection.game_id,
                started_at,
                ended_at,
                counted_ms,
                until_ms,
            )
            .await?;
            added_ms = added_ms.saturating_add(counted_ms);
        }
    }

    let mut committed = selection.clone();
    committed.cursor_ms = until_ms;
    committed.counted_ms = committed.counted_ms.saturating_add(added_ms);
    write_i64_tx(&mut tx, LAST_IMPORTED_KEY, until_ms).await?;
    if keep_active {
        save_selection_tx(&mut tx, &committed).await?;
    } else {
        clear_selection_tx(&mut tx).await?;
    }
    tx.commit().await?;
    *selection = committed;
    Ok(added_ms)
}

async fn insert_android_session(
    tx: &mut Transaction<'_, Sqlite>,
    device: &DeviceId,
    game_id: &str,
    started_at: i64,
    ended_at: i64,
    counted_ms: i64,
    now_ms: i64,
) -> Result<(), DbError> {
    let session_id = EntityId::new().to_string();
    sqlx::query(
        "INSERT INTO play_sessions \
         (id, game_id, chapter_id, started_at, ended_at, counted_ms, afk_ms, source, updated_at, device_id, deleted) \
         VALUES (?, ?, NULL, ?, ?, ?, 0, 'android_usage', ?, ?, 0)",
    )
    .bind(session_id)
    .bind(game_id)
    .bind(started_at)
    .bind(ended_at)
    .bind(counted_ms)
    .bind(now_ms)
    .bind(device.as_str())
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "UPDATE games SET started_at = COALESCE(started_at, ?), \
         updated_at = MAX(updated_at, ?), device_id = ? \
         WHERE id = ? AND deleted = 0",
    )
    .bind(started_at)
    .bind(now_ms)
    .bind(device.as_str())
    .bind(game_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

async fn game_is_live(tx: &mut Transaction<'_, Sqlite>, game_id: &str) -> Result<bool, DbError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM games WHERE id = ? AND deleted = 0")
        .bind(game_id)
        .fetch_one(&mut **tx)
        .await?;
    Ok(count > 0)
}

async fn load_selection(pool: &SqlitePool) -> Result<Option<ActiveSelection>, DbError> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM sync_state WHERE key = ?")
        .bind(ACTIVE_SELECTION_KEY)
        .fetch_optional(pool)
        .await?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|err| DbError::Corrupt(format!("corrupt Android active selection: {err}")))
}

async fn save_selection(pool: &SqlitePool, selection: &ActiveSelection) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    save_selection_tx(&mut tx, selection).await?;
    tx.commit().await?;
    Ok(())
}

async fn save_selection_tx(
    tx: &mut Transaction<'_, Sqlite>,
    selection: &ActiveSelection,
) -> Result<(), DbError> {
    let raw = serde_json::to_string(selection)
        .map_err(|err| DbError::Corrupt(format!("cannot encode Android selection: {err}")))?;
    sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(ACTIVE_SELECTION_KEY)
    .bind(raw)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn clear_selection(pool: &SqlitePool) -> Result<(), DbError> {
    sqlx::query("DELETE FROM sync_state WHERE key = ?")
        .bind(ACTIVE_SELECTION_KEY)
        .execute(pool)
        .await?;
    Ok(())
}

async fn clear_selection_tx(tx: &mut Transaction<'_, Sqlite>) -> Result<(), DbError> {
    sqlx::query("DELETE FROM sync_state WHERE key = ?")
        .bind(ACTIVE_SELECTION_KEY)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn read_i64(pool: &SqlitePool, key: &str) -> Result<Option<i64>, DbError> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM sync_state WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(raw.and_then(|value| value.parse::<i64>().ok()))
}

async fn write_i64_tx(
    tx: &mut Transaction<'_, Sqlite>,
    key: &str,
    value: i64,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, queries};
    use vn_core::clock::Timestamp;
    use vn_core::model::{Game, GameStatus};
    use vn_core::usage::UsageEventKind;

    async fn seed_game(pool: &SqlitePool, device: &DeviceId, title: &str) -> EntityId {
        let now = Timestamp::from_millis(1_000);
        let game = Game::new(device, now, title, GameStatus::Playing).expect("valid game");
        let id = game.meta.id;
        queries::upsert_game(pool, &game).await.expect("seed game");
        id
    }

    fn selection(game_id: EntityId) -> ActiveSelection {
        ActiveSelection {
            game_id: game_id.to_string(),
            package: "org.example.runner".to_owned(),
            runner_kind: Some("custom".to_owned()),
            started_at_ms: 1_000,
            cursor_ms: 1_000,
            counted_ms: 0,
            stopped_at_ms: None,
        }
    }

    fn event(package: Option<&str>, kind: i32, at_ms: i64) -> RawUsageEvent {
        RawUsageEvent {
            package: package.map(str::to_owned),
            activity: None,
            kind,
            at_ms,
        }
    }

    async fn total(pool: &SqlitePool, game_id: EntityId) -> i64 {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(s.counted_ms), 0) FROM play_sessions s \
             JOIN games g ON g.id = s.game_id WHERE g.id = ? AND g.deleted = 0",
        )
        .bind(game_id.to_string())
        .fetch_one(pool)
        .await
        .expect("total")
    }

    #[tokio::test]
    async fn leaving_for_another_app_keeps_the_session_alive() {
        // Reported bug: start a game from VN Asyik, open another app, come back.
        // The time away was counted and the session ended on return -- the exact
        // inverse of what should happen.
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "Away").await;
        let selected = selection(game);

        // Reading for 90 s, then another app for 5 minutes, then VN Asyik itself
        // resumes at 400_000 -- which is what `tracking_android_session` replays
        // when the user comes back.
        let events = vec![
            event(Some("org.example.runner"), 1, 2_000),
            event(Some("com.android.chrome"), 1, 92_000),
            event(Some(SELF_PACKAGE_MARKER), 1, 400_000),
        ];
        let view = apply_query_result(db.pool(), &device, selected, 401_000, false, Ok(events))
            .await
            .expect("replay");

        // The session must still be open, so the next stretch keeps accruing.
        assert!(view.active, "coming back must not end the session");
        // Only the reading stretch is counted; the 5 minutes in Chrome are not.
        assert_eq!(view.counted_ms, 90_000, "away time must not be counted");
        assert_eq!(total(db.pool(), game).await, 90_000);
        assert!(
            load_selection(db.pool()).await.expect("load").is_some(),
            "the selection survives so reading can continue"
        );
    }

    #[tokio::test]
    async fn reading_after_returning_to_the_app_keeps_counting() {
        // The reported sequence one step further: come back to VN Asyik, then go
        // back to the game and read again. The cutoff has to be the return of
        // *this* round, not the first one in the replay window, or the persisted
        // cursor never moves past it and every later stretch is dropped.
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "Second stretch").await;
        let selected = selection(game);

        // Round 1: 90 s reading, back to VN Asyik at 100_000.
        let first = apply_query_result(
            db.pool(),
            &device,
            selected,
            101_000,
            false,
            Ok(vec![
                event(Some("org.example.runner"), 1, 2_000),
                event(Some("com.android.chrome"), 1, 92_000),
                event(Some(SELF_PACKAGE_MARKER), 1, 100_000),
            ]),
        )
        .await
        .expect("first replay");
        assert_eq!(first.counted_ms, 90_000);

        // Round 2: back to the game at 200_000, five minutes of reading, return at
        // 500_000. UsageStats replays the whole window from `started_at_ms`, so the
        // first round's own return event is still in the log.
        let selected = load_selection(db.pool())
            .await
            .expect("load")
            .expect("selection");
        let view = apply_query_result(
            db.pool(),
            &device,
            selected,
            501_000,
            false,
            Ok(vec![
                event(Some("org.example.runner"), 1, 2_000),
                event(Some("com.android.chrome"), 1, 92_000),
                event(Some(SELF_PACKAGE_MARKER), 1, 100_000),
                event(Some("org.example.runner"), 1, 200_000),
                event(Some(SELF_PACKAGE_MARKER), 1, 500_000),
            ]),
        )
        .await
        .expect("second replay");

        assert!(view.active, "the session continues across both stretches");
        assert_eq!(
            view.counted_ms,
            90_000 + 300_000,
            "the time read after coming back must be counted"
        );
        assert_eq!(total(db.pool(), game).await, 390_000);
    }

    #[tokio::test]
    async fn a_two_second_foreground_stretch_writes_no_session_row() {
        // The reported bug: a game flicked to for a moment left a `0:00` row,
        // and doing that repeatedly filled the session list with entries for
        // games that were never read. The interval is real; the row is not.
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "Glanced").await;
        let selected = selection(game);

        let events = vec![
            event(Some("org.example.runner"), 1, 2_000),
            event(None, 2, 4_000),
        ];
        apply_query_result(db.pool(), &device, selected, 5_000, true, Ok(events))
            .await
            .expect("replay");

        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM play_sessions")
            .fetch_one(db.pool())
            .await
            .expect("count");
        assert_eq!(rows, 0, "a 2 s glance must not create a session");
        assert_eq!(total(db.pool(), game).await, 0);
    }

    #[tokio::test]
    async fn a_minute_of_reading_is_still_recorded() {
        // The other side of the floor: the fix must not swallow real time.
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "Read").await;
        let selected = selection(game);

        let events = vec![
            event(Some("org.example.runner"), 1, 2_000),
            event(None, 2, 62_000),
        ];
        apply_query_result(db.pool(), &device, selected, 63_000, true, Ok(events))
            .await
            .expect("replay");

        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM play_sessions")
            .fetch_one(db.pool())
            .await
            .expect("count");
        assert_eq!(rows, 1, "60 s of reading is a session");
        // 2_000 -> 62_000 is sixty seconds of foreground, plus the row starts at
        // the selection cursor, so the exact figure is pinned rather than
        // approximated.
        assert_eq!(total(db.pool(), game).await, 61_000);
    }

    #[tokio::test]
    async fn usage_failures_and_empty_history_never_become_wall_clock_playtime() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "A").await;
        let selected = selection(game);
        save_selection(db.pool(), &selected)
            .await
            .expect("selection");
        for failure in [
            AndroidUsageError::NotGranted,
            AndroidUsageError::Unavailable("locked".to_owned()),
        ] {
            assert!(
                apply_query_result(
                    db.pool(),
                    &device,
                    selected.clone(),
                    3_601_000,
                    false,
                    Err(failure)
                )
                .await
                .is_err()
            );
            assert_eq!(
                load_selection(db.pool()).await.expect("load"),
                Some(selected.clone())
            );
            assert_eq!(total(db.pool(), game).await, 0);
            let status = status(db.pool()).await.expect("status");
            assert_eq!(status.elapsed_ms, 0, "status has no wall-clock fallback");
        }
        let empty = apply_query_result(db.pool(), &device, selected, 3_601_000, true, Ok(vec![]))
            .await
            .expect("empty query");
        assert!(!empty.active);
        assert_eq!(empty.counted_ms, 0);
        assert_eq!(total(db.pool(), game).await, 0);
    }

    #[tokio::test]
    async fn foreground_replay_excludes_other_apps_screen_off_and_later_runner_launches() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "Selected").await;
        let other = seed_game(db.pool(), &device, "Same runner").await;
        let selected = selection(game);
        save_selection(db.pool(), &selected)
            .await
            .expect("selection");
        let history = vec![
            event(Some("org.example.runner"), 1, 2_000),
            event(Some(""), 1, 12_000), // Anonymous browser switch, outgoing pause absent.
            event(Some("org.example.runner"), 1, 22_000),
            event(None, 16, 32_000),
            event(None, 15, 42_000),
            event(Some("org.example.runner"), 1, 43_000),
            event(Some(SELF_PACKAGE_MARKER), 1, 53_000),
            event(Some("org.example.runner"), 1, 63_000), // Not attributed after app return.
        ];
        let view = apply_query_result(db.pool(), &device, selected, 73_000, false, Ok(history))
            .await
            .expect("replay");
        // The session stays open: returning to VN Asyik records *no further time*
        // but does not end the run, so the next stretch continues the same
        // session. It used to end here, which meant a reader who checked
        // something mid-scene lost everything after the trip.
        assert!(
            view.active,
            "coming back to the app must not end the session"
        );
        assert_eq!(view.counted_ms, 30_000);
        assert_eq!(total(db.pool(), game).await, 30_000);
        assert_eq!(total(db.pool(), other).await, 0);
        assert!(load_selection(db.pool()).await.expect("load").is_some());
    }

    #[tokio::test]
    async fn restart_replay_discards_open_shutdown_interval_and_ends_selection() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "A").await;
        save_selection(db.pool(), &selection(game))
            .await
            .expect("selection");
        // Reload from SQLite to exercise restart persistence rather than an in-memory timer.
        let selected = load_selection(db.pool())
            .await
            .expect("load")
            .expect("selection");
        let view = apply_query_result(
            db.pool(),
            &device,
            selected,
            40_000,
            false,
            Ok(vec![
                event(Some("org.example.runner"), 1, 2_000),
                event(Some("org.example.runner"), 2, 12_000),
                event(Some("org.example.runner"), 1, 15_000),
                event(None, 26, 20_000),
                event(None, 27, 30_000),
                event(Some("org.example.runner"), 1, 31_000),
            ]),
        )
        .await
        .expect("restart replay");
        assert!(!view.active);
        assert_eq!(
            view.counted_ms, 10_000,
            "open pre-shutdown activity and later launch are uncredited"
        );
    }

    #[tokio::test]
    async fn polls_replay_open_activity_context_and_clip_persisted_cursor() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "A").await;
        let selected = selection(game);
        save_selection(db.pool(), &selected)
            .await
            .expect("selection");
        let first = apply_query_result(
            db.pool(),
            &device,
            selected,
            10_000,
            false,
            Ok(vec![event(Some("org.example.runner"), 1, 2_000)]),
        )
        .await
        .expect("first");
        assert_eq!(first.counted_ms, 8_000);
        let selected = load_selection(db.pool())
            .await
            .expect("load")
            .expect("selection");
        let next = apply_query_result(
            db.pool(),
            &device,
            selected,
            30_000,
            false,
            Ok(vec![
                event(Some("org.example.runner"), 1, 2_000),
                event(Some("org.example.runner"), 2, 22_000),
            ]),
        )
        .await
        .expect("second");
        assert_eq!(next.counted_ms, 20_000);
        assert_eq!(
            total(db.pool(), game).await,
            20_000,
            "already credited 8s must not replay"
        );
    }

    #[tokio::test]
    async fn failed_cursor_write_rolls_back_sessions_and_preserves_retry_position() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "A").await;
        let mut selected = selection(game);
        save_selection(db.pool(), &selected)
            .await
            .expect("selection");
        let original = selected.clone();
        sqlx::query("CREATE TRIGGER fail_cursor BEFORE UPDATE ON sync_state WHEN NEW.key = 'android.activeSelection' BEGIN SELECT RAISE(ABORT, 'disk write failure'); END")
            .execute(db.pool()).await.expect("trigger");
        let history = [
            event(Some("org.example.runner"), 1, 2_000),
            event(Some("org.example.runner"), 2, 12_000),
        ];
        let events: Vec<_> = history.iter().map(RawUsageEvent::to_usage_event).collect();
        assert!(
            commit_events(db.pool(), &device, &mut selected, &events, 20_000, true)
                .await
                .is_err()
        );
        assert_eq!(
            selected, original,
            "in-memory cursor must also wait for commit"
        );
        assert_eq!(
            load_selection(db.pool()).await.expect("load"),
            Some(original)
        );
        assert_eq!(
            total(db.pool(), game).await,
            0,
            "partial session insert rolls back"
        );
        sqlx::query("DROP TRIGGER fail_cursor")
            .execute(db.pool())
            .await
            .expect("drop trigger");
        assert_eq!(
            commit_events(db.pool(), &device, &mut selected, &events, 20_000, true)
                .await
                .expect("retry"),
            10_000
        );
        assert_eq!(total(db.pool(), game).await, 10_000);
    }

    #[tokio::test]
    async fn failed_stop_remembers_cutoff_without_crediting_future_runner_time() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "A").await;
        let mut selected = selection(game);
        selected.stopped_at_ms = Some(12_000);
        save_selection(db.pool(), &selected)
            .await
            .expect("pending stop");
        assert!(
            apply_query_result(
                db.pool(),
                &device,
                selected.clone(),
                12_000,
                true,
                Err(AndroidUsageError::NotGranted)
            )
            .await
            .is_err()
        );
        assert!(!status(db.pool()).await.expect("status").active);
        let view = apply_query_result(
            db.pool(),
            &device,
            selected,
            40_000,
            false,
            Ok(vec![
                event(Some("org.example.runner"), 1, 2_000),
                event(Some("org.example.runner"), 2, 12_000),
                event(Some("org.example.runner"), 1, 20_000),
            ]),
        )
        .await
        .expect("retry stop");
        assert!(!view.active);
        assert_eq!(total(db.pool(), game).await, 10_000);
    }

    #[tokio::test]
    async fn shared_package_attributes_only_to_selected_game_and_advances_cursor() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game_a = seed_game(db.pool(), &device, "Fate/stay night").await;
        let game_b = seed_game(db.pool(), &device, "Tsukihime").await;

        let mut selection = ActiveSelection {
            game_id: game_a.to_string(),
            package: "org.tvp.kirikiri2".to_owned(),
            runner_kind: Some("kirikiroid2".to_owned()),
            started_at_ms: 1_000,
            cursor_ms: 1_000,
            counted_ms: 0,
            stopped_at_ms: None,
        };
        save_selection(db.pool(), &selection)
            .await
            .expect("save selection");

        let events = vec![
            UsageEvent {
                package: Some("org.tvp.kirikiri2".to_owned()),
                activity: None,
                kind: UsageEventKind::Resumed,
                at_ms: 2_000,
            },
            UsageEvent {
                package: Some("org.tvp.kirikiri2".to_owned()),
                activity: None,
                kind: UsageEventKind::Paused,
                at_ms: 62_000,
            },
        ];

        let first = commit_events(db.pool(), &device, &mut selection, &events, 70_000, true)
            .await
            .expect("commit");
        let replay = commit_events(db.pool(), &device, &mut selection, &events, 70_000, false)
            .await
            .expect("replay");

        assert_eq!(first, 60_000);
        assert_eq!(replay, 0, "cursor must prevent double-counting");

        let b_total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(counted_ms), 0) FROM play_sessions WHERE game_id = ?",
        )
        .bind(game_b.to_string())
        .fetch_one(db.pool())
        .await
        .expect("sum b");
        assert_eq!(b_total, 0);
    }

    // ---- Discord presence follows the foreground, not the open selection ----

    #[tokio::test]
    async fn presence_dies_when_the_game_leaves_the_foreground() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "Foreground").await;
        let selected = selection(game);
        save_selection(db.pool(), &selected)
            .await
            .expect("selection");
        let history = vec![
            event(Some("org.example.runner"), 1, 2_000),
            event(None, 16, 30_000), // Screen off ends the interval.
        ];
        apply_query_result(
            db.pool(),
            &device,
            selected.clone(),
            40_000,
            false,
            Ok(history),
        )
        .await
        .expect("replay");

        // A screen that went off 10 s ago is still inside the tolerance window.
        let target = active_target(db.pool(), 40_000).await.expect("recent");
        assert_eq!(target.game_id, game);
        assert_eq!(target.active_ms, 38_000);
        assert_eq!(target.start_epoch, 2, "anchored at the first counted ms");

        // The same state an hour later is an overnight lock, not foreground.
        assert!(active_target(db.pool(), 200_000).await.is_none());

        let stopped = ActiveSelection {
            stopped_at_ms: Some(35_000),
            ..selected
        };
        save_selection(db.pool(), &stopped).await.expect("stop");
        assert!(active_target(db.pool(), 40_000).await.is_none());
    }

    #[tokio::test]
    async fn presence_needs_a_counted_interval_before_the_game_reaches_the_foreground() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "Launching").await;
        save_selection(db.pool(), &selection(game))
            .await
            .expect("selection");
        // The game was launched but UsageStats has not seen it resume yet.
        apply_query_result(
            db.pool(),
            &device,
            selection(game),
            10_000,
            false,
            Ok(vec![]),
        )
        .await
        .expect("empty replay");
        assert!(active_target(db.pool(), 10_000).await.is_none());
    }

    #[tokio::test]
    async fn presence_elapsed_is_counted_time_not_wall_clock() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "Counted").await;
        save_selection(db.pool(), &selection(game))
            .await
            .expect("selection");
        let history = vec![
            event(Some("org.example.runner"), 1, 2_000),
            event(Some(""), 1, 12_000), // Browser gap the card must not count.
            event(Some("org.example.runner"), 1, 42_000),
        ];
        apply_query_result(
            db.pool(),
            &device,
            selection(game),
            52_000,
            false,
            Ok(history),
        )
        .await
        .expect("replay");

        let target = active_target(db.pool(), 52_000).await.expect("foreground");
        assert_eq!(target.active_ms, 20_000, "two 10 s intervals, gap excluded");
        assert_eq!(
            target.start_epoch, 32,
            "now minus counted time, so the card shows 20 s at t=52 s"
        );
    }
}
