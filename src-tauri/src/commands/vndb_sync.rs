//! VNDB account and list-sync commands.
//!
//! The split of responsibility matters here:
//!
//! * `vndb_connect` verifies the token and stores it in the OS credential store;
//!   the token never reaches the database or the frontend.
//! * `vndb_pull` fetches the remote list and stores it as sync state, producing a
//!   list of comparisons the UI can show. It does **not** rewrite local games by
//!   itself: every difference becomes something the user can see.
//! * `vndb_apply_sync` takes the decisions (or the policy) and performs the two
//!   directions: local updates and/or a queued push.
//! * `vndb_push_pending` replays the queue, so an offline session recovers.

use serde::{Deserialize, Serialize};
use tauri::State;
use vn_core::clock::Timestamp;

use super::{CommandError, CommandResult};
use crate::db::queries;
use crate::secrets::{self, SecretError, VndbToken};
use crate::state::AppState;
use crate::vndb::VndbError;
use crate::vndb::engine::{self, FieldDecision, LocalUpdate, SyncPlan};
use crate::vndb::mapping::{self, Conflict, ConflictPolicy, LabelMapping, LocalState, RemoteState};
use crate::vndb::sync::{self, Account, LogKind, QueueEntryView, SyncState};
use crate::vndb::ulist::{ULIST_FIELDS, UlistQuery};

fn now() -> CommandResult<Timestamp> {
    Timestamp::now().map_err(Into::into)
}

fn token() -> CommandResult<VndbToken> {
    match secrets::load_vndb_token() {
        Ok(token) => Ok(token),
        Err(SecretError::NotFound) => Err(CommandError::new(
            "not_connected",
            "connect a VNDB account first",
        )),
        Err(SecretError::Unsupported) => Err(CommandError::new(
            "unsupported",
            "secure storage is not available on this platform yet",
        )),
        Err(err) => Err(CommandError::new("secret", err.to_string())),
    }
}

fn map_vndb(err: &VndbError) -> CommandError {
    CommandError::from(err)
}

/// Connection state as shown in Settings.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionState {
    /// Whether this platform can store a token at all.
    pub storage_supported: bool,
    /// The connected account, when there is one.
    pub account: Option<Account>,
    /// Pending push count.
    pub pending: i64,
}

/// Read the current connection state.
#[tauri::command]
pub async fn vndb_status(state: State<'_, AppState>) -> CommandResult<ConnectionState> {
    let pool = state.db().pool();
    Ok(ConnectionState {
        storage_supported: secrets::is_supported(),
        account: sync::load_account(pool).await?,
        pending: sync::queue_len(pool).await?,
    })
}

/// Verify a token with `GET /authinfo` and store it on success.
///
/// The token is checked before it is stored, so a typo does not leave a broken
/// connection behind. Nothing about the token is logged.
#[tauri::command]
pub async fn vndb_connect(state: State<'_, AppState>, token: String) -> CommandResult<Account> {
    if !secrets::is_supported() {
        return Err(CommandError::new(
            "unsupported",
            "secure storage is not available on this platform yet",
        ));
    }
    let parsed = VndbToken::parse(&token).map_err(|reason| CommandError::new("invalid", reason))?;

    let info = state
        .vndb()
        .authinfo(parsed.expose())
        .await
        .map_err(|err| map_vndb(&err))?;

    secrets::store_vndb_token(&parsed)
        .map_err(|err| CommandError::new("secret", err.to_string()))?;

    let timestamp = now()?;
    let account = Account {
        user_id: info.id.clone(),
        username: info.username.clone(),
        permissions: info.permissions.clone(),
        connected_at: timestamp.as_millis(),
        last_pull_at: None,
        last_push_at: None,
    };
    sync::save_account(state.db().pool(), &account).await?;
    sync::log(
        state.db().pool(),
        LogKind::Connect,
        None,
        &format!("connected as {}", info.username),
        timestamp,
    )
    .await?;

    Ok(account)
}

/// Forget the token and all sync bookkeeping.
#[tauri::command]
pub async fn vndb_disconnect(state: State<'_, AppState>) -> CommandResult<()> {
    match secrets::clear_vndb_token() {
        Ok(()) | Err(SecretError::NotFound) => {}
        Err(err) => return Err(CommandError::new("secret", err.to_string())),
    }
    sync::clear_account(state.db().pool()).await?;
    sync::log(
        state.db().pool(),
        LogKind::Disconnect,
        None,
        "disconnected",
        now()?,
    )
    .await?;
    Ok(())
}

/// Fill in the token from the OS store for a session, without exposing it.
///
/// Returns the masked form so the UI can confirm which token is in use.
#[tauri::command]
pub async fn vndb_token_hint() -> CommandResult<Option<String>> {
    let token = tauri::async_runtime::spawn_blocking(secrets::load_vndb_token)
        .await
        .map_err(|_| CommandError::new("secret", "secure storage task failed"))?;
    match token {
        Ok(token) => Ok(Some(token.masked())),
        Err(SecretError::NotFound | SecretError::Unsupported) => Ok(None),
        Err(err) => Err(CommandError::new("secret", err.to_string())),
    }
}

/// Label mapping and policy, as edited in Settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSettings {
    /// Mapping from local status to label ids.
    pub mapping: LabelMapping,
    /// Default conflict policy.
    pub policy: ConflictPolicy,
}

/// Read the label mapping and policy.
#[tauri::command]
pub async fn vndb_sync_settings(state: State<'_, AppState>) -> CommandResult<SyncSettings> {
    let (mapping, policy) = sync::load_settings(state.db().pool()).await?;
    Ok(SyncSettings { mapping, policy })
}

/// Save the label mapping and policy.
///
/// Rejects a mapping that sends two statuses to the same label, because the round
/// trip would be lossy and silently wrong.
#[tauri::command]
pub async fn vndb_save_sync_settings(
    state: State<'_, AppState>,
    mapping: LabelMapping,
    policy: ConflictPolicy,
) -> CommandResult<SyncSettings> {
    if let Some((first, second)) = mapping.collides() {
        return Err(CommandError::new(
            "collision",
            format!(
                "{} and {} would both map to the same VNDB label",
                first.as_str(),
                second.as_str()
            ),
        ));
    }
    sync::save_settings(state.db().pool(), &mapping, policy, now()?).await?;
    Ok(SyncSettings { mapping, policy })
}

/// One VN as it appears on both sides, for the sync screen.
#[derive(Debug, Clone, Serialize)]
pub struct SyncRow {
    /// VN id.
    pub vn_id: String,
    /// Local game, when the VN has been imported.
    pub game_id: Option<String>,
    /// Title for display.
    pub title: String,
    /// Conflicts found for this VN.
    pub conflicts: Vec<Conflict>,
}

/// Result of a pull.
#[derive(Debug, Clone, Serialize)]
pub struct PullResult {
    /// Entries read from VNDB.
    pub entries: usize,
    /// Entries that exist on VNDB but not locally, with the state the user
    /// already has, so importing them does not create conflicts.
    pub not_imported: Vec<crate::commands::vndb::MissingEntry>,
    /// Entries with at least one conflict.
    pub conflicts: Vec<SyncRow>,
    /// Entries that needed nothing.
    pub in_sync: usize,
}

/// Hard cap on pages, so a misbehaving server cannot loop forever.
const MAX_PAGES: u32 = 40;

/// Everything one pull needs, gathered once.
struct PullContext {
    mapping: LabelMapping,
    policy: ConflictPolicy,
    user_id: String,
}

/// Pull the remote list and compare it with the local library.
///
/// Nothing is overwritten here: the result is a report the UI turns into a
/// review screen.
#[tauri::command]
pub async fn vndb_pull(state: State<'_, AppState>) -> CommandResult<PullResult> {
    let token = token()?;
    let pool = state.db().pool();
    let (mapping, policy) = sync::load_settings(pool).await?;
    let account = sync::load_account(pool)
        .await?
        .ok_or_else(|| CommandError::new("not_connected", "connect a VNDB account first"))?;
    let ctx = PullContext {
        mapping,
        policy,
        user_id: account.user_id.clone(),
    };

    let result = compare_remote_list(&state, &ctx, token.expose()).await?;
    queue_policy_pushes(pool, &ctx, &result).await?;

    let timestamp = now()?;
    sync::touch_account(pool, Some(timestamp), None).await?;
    sync::log(
        pool,
        LogKind::Pull,
        None,
        &format!(
            "pulled {} entries, {} conflicts",
            result.entries,
            result.conflicts.len()
        ),
        timestamp,
    )
    .await?;
    sync::trim_log(pool, 500).await?;

    Ok(result)
}

/// Walk every page of the remote list and classify each entry.
async fn compare_remote_list(
    state: &State<'_, AppState>,
    ctx: &PullContext,
    token: &str,
) -> CommandResult<PullResult> {
    let pool = state.db().pool();
    let mut page = 1u32;
    let mut result = PullResult {
        entries: 0,
        not_imported: Vec::new(),
        conflicts: Vec::new(),
        in_sync: 0,
    };

    loop {
        let query = UlistQuery::page(&ctx.user_id, ULIST_FIELDS, page);
        let response = state
            .vndb()
            .ulist_page(token, &query)
            .await
            .map_err(|err| map_vndb(&err))?;

        for entry in &response.results {
            result.entries += 1;
            classify_entry(pool, ctx, entry, &mut result).await?;
        }

        if !response.more || page >= MAX_PAGES {
            break;
        }
        page += 1;
    }

    Ok(result)
}

/// Record one entry as sync state and bucket it into the pull result.
async fn classify_entry(
    pool: &sqlx::SqlitePool,
    ctx: &PullContext,
    entry: &crate::vndb::ulist::UlistEntry,
    result: &mut PullResult,
) -> CommandResult<()> {
    let remote = RemoteState::from_entry(entry, &ctx.mapping);
    let Some(game) = queries::get_game_by_vndb_id(pool, &entry.id).await? else {
        // Carry the user's own list state along: creating the game from VNDB
        // metadata alone would leave it as an unrated wishlist row, and the next
        // pull would report every field as a conflict.
        result
            .not_imported
            .push(crate::commands::vndb::MissingEntry {
                vn_id: entry.id.clone(),
                status: remote.status.map(|status| {
                    serde_json::to_value(status)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_owned))
                        .unwrap_or_default()
                }),
                rating: remote.rating,
                notes: remote.notes.clone(),
                started_at: remote.started_at,
                finished_at: remote.finished_at,
            });
        return Ok(());
    };

    let local = local_state_of(&game);
    let plan = engine::plan(
        &local,
        &remote,
        &entry.label_ids(),
        &ctx.mapping,
        ctx.policy,
        &[],
    );

    sync::save_state(
        pool,
        &SyncState {
            vn_id: entry.id.clone(),
            game_id: Some(game.meta.id.to_string()),
            remote_lastmod: remote.lastmod_ms,
            remote_vote: entry.vote,
            remote_notes: entry.notes.clone(),
            remote_started: entry.started.clone(),
            remote_finished: entry.finished.clone(),
            remote_labels: entry.label_ids(),
            pulled_at: now()?.as_millis(),
        },
    )
    .await?;

    if plan.conflicts.is_empty() {
        result.in_sync += 1;
    } else {
        result.conflicts.push(SyncRow {
            vn_id: entry.id.clone(),
            game_id: Some(game.meta.id.to_string()),
            title: game.title,
            conflicts: plan.conflicts,
        });
    }
    Ok(())
}

/// Queue pushes that the policy already decided, so they are never lost.
async fn queue_policy_pushes(
    pool: &sqlx::SqlitePool,
    ctx: &PullContext,
    result: &PullResult,
) -> CommandResult<()> {
    let timestamp = now()?;
    for row in &result.conflicts {
        let Some(state_row) = sync::load_state(pool, &row.vn_id).await? else {
            continue;
        };
        let Some(game) = queries::get_game_by_vndb_id(pool, &row.vn_id).await? else {
            continue;
        };
        let local = local_state_of(&game);
        let remote = remote_state_of(&state_row, &ctx.mapping);
        let plan = engine::plan(
            &local,
            &remote,
            &state_row.remote_labels,
            &ctx.mapping,
            ctx.policy,
            &[],
        );
        if plan.needs_push() {
            sync::queue_push(
                pool,
                &row.vn_id,
                &game.meta.id.to_string(),
                &plan.push,
                timestamp,
            )
            .await?;
        }
    }
    Ok(())
}

/// Build the local comparison side from a stored game.
fn local_state_of(game: &vn_core::model::Game) -> LocalState {
    LocalState {
        status: Some(game.status),
        rating: game.rating,
        notes: game.notes.clone(),
        started_at: game.started_at.map(Timestamp::as_millis),
        finished_at: game.finished_at.map(Timestamp::as_millis),
        updated_at_ms: game.meta.updated_at.as_millis(),
    }
}

/// Build the remote comparison side from stored sync state.
fn remote_state_of(state_row: &SyncState, mapping: &LabelMapping) -> RemoteState {
    RemoteState {
        status: mapping.status_from_labels(&state_row.remote_labels),
        rating: mapping::vote_to_rating(state_row.remote_vote),
        notes: state_row.remote_notes.clone(),
        started_at: mapping::date_to_ms(state_row.remote_started.as_deref()),
        finished_at: mapping::date_to_ms(state_row.remote_finished.as_deref()),
        lastmod_ms: state_row.remote_lastmod,
    }
}

/// Payload for applying sync decisions.
#[derive(Debug, Clone, Deserialize)]
pub struct ApplyInput {
    /// Per-field decisions from the conflict screen. Empty means "use the policy".
    #[serde(default)]
    pub decisions: Vec<VnDecisions>,
    /// When true, push every local row rather than only decided ones.
    #[serde(default)]
    pub push_all_local: bool,
}

/// Decisions for one VN.
#[derive(Debug, Clone, Deserialize)]
pub struct VnDecisions {
    /// VN id.
    pub vn_id: String,
    /// The field decisions.
    pub fields: Vec<FieldDecision>,
}

/// Outcome of applying decisions.
#[derive(Debug, Clone, Serialize)]
pub struct ApplyResult {
    /// Local games updated from VNDB.
    pub updated: usize,
    /// VN ids queued for push.
    pub queued: Vec<String>,
    /// Conflicts still undecided.
    pub remaining: usize,
}

/// Apply conflict decisions locally and queue the pushes.
#[tauri::command]
pub async fn vndb_apply_sync(
    state: State<'_, AppState>,
    input: ApplyInput,
) -> CommandResult<ApplyResult> {
    let pool = state.db().pool();
    let (mapping, policy) = sync::load_settings(pool).await?;
    let timestamp = now()?;
    let mut result = ApplyResult {
        updated: 0,
        queued: Vec::new(),
        remaining: 0,
    };

    let states = sync::all_states(pool).await?;

    for state_row in &states {
        let Some(game) = queries::get_game_by_vndb_id(pool, &state_row.vn_id).await? else {
            continue;
        };
        let local = local_state_of(&game);
        let remote = remote_state_of(state_row, &mapping);

        let decisions: Vec<FieldDecision> = input
            .decisions
            .iter()
            .find(|entry| entry.vn_id == state_row.vn_id)
            .map(|entry| entry.fields.clone())
            .unwrap_or_default();

        let plan: SyncPlan = engine::plan(
            &local,
            &remote,
            &state_row.remote_labels,
            &mapping,
            policy,
            &decisions,
        );

        if plan.needs_local_update() {
            apply_local_update(pool, &game, &plan.update, timestamp).await?;
            result.updated += 1;
        }

        let should_push = plan.needs_push()
            || (input.push_all_local
                && matches!(policy, ConflictPolicy::PreferLocal | ConflictPolicy::Newest)
                && plan.update.is_empty());

        if should_push {
            let patch = if plan.needs_push() {
                plan.push.clone()
            } else {
                engine::full_push(&local, &state_row.remote_labels, &mapping)
            };
            if !patch.is_empty() {
                sync::queue_push(
                    pool,
                    &state_row.vn_id,
                    &game.meta.id.to_string(),
                    &patch,
                    timestamp,
                )
                .await?;
                result.queued.push(state_row.vn_id.clone());
            }
        }

        result.remaining += plan.conflicts.len();
    }

    sync::log(
        pool,
        LogKind::Conflict,
        None,
        &format!(
            "applied decisions: {} updated, {} queued, {} remaining",
            result.updated,
            result.queued.len(),
            result.remaining
        ),
        timestamp,
    )
    .await?;

    Ok(result)
}

/// Write the chosen remote values onto the local game.
async fn apply_local_update(
    pool: &sqlx::SqlitePool,
    game: &vn_core::model::Game,
    update: &LocalUpdate,
    timestamp: Timestamp,
) -> CommandResult<()> {
    let mut game = game.clone();
    if let Some(status) = update.status {
        game.status = status;
    }
    if let Some(rating) = update.rating {
        game.set_rating(rating)?;
    }
    if let Some(notes) = update.notes.clone() {
        game.set_notes(notes)?;
    }
    if let Some(started) = update.started_at {
        game.started_at = started.map(Timestamp::from_millis);
    }
    if let Some(finished) = update.finished_at {
        game.finished_at = finished.map(Timestamp::from_millis);
    }
    game.meta = game.meta.touch(timestamp);
    queries::upsert_game(pool, &game).await?;
    Ok(())
}

/// Progress of a queue replay.
#[derive(Debug, Clone, Serialize)]
pub struct PushResult {
    /// Pushes that succeeded.
    pub pushed: usize,
    /// Pushes that failed and stay queued.
    pub failed: usize,
    /// Pushes still waiting.
    pub remaining: i64,
}

/// Replay the push queue.
///
/// Entries are removed only after the API confirms success, so a failure leaves
/// the intent to retry intact and nothing is lost offline.
#[tauri::command]
pub async fn vndb_push_pending(state: State<'_, AppState>) -> CommandResult<PushResult> {
    let token = token()?;
    let pool = state.db().pool();
    let queue = sync::list_queue(pool).await?;
    let timestamp = now()?;
    let mut pushed = 0usize;
    let mut failed = 0usize;

    for entry in &queue {
        match state
            .vndb()
            .ulist_patch(token.expose(), &entry.vn_id, &entry.patch)
            .await
        {
            Ok(()) => {
                sync::remove_push(pool, &entry.vn_id).await?;
                sync::log(
                    pool,
                    LogKind::Push,
                    Some(&entry.vn_id),
                    "pushed local changes",
                    timestamp,
                )
                .await?;
                pushed += 1;
            }
            Err(err) => {
                // Auth failures will never succeed by retrying, so stop early and
                // report rather than burning the rate limit.
                sync::record_push_failure(pool, &entry.vn_id, &err.to_string()).await?;
                sync::log(
                    pool,
                    LogKind::Error,
                    Some(&entry.vn_id),
                    &err.to_string(),
                    timestamp,
                )
                .await?;
                failed += 1;
                if matches!(err, VndbError::Unauthorized) {
                    break;
                }
            }
        }
    }

    sync::touch_account(pool, None, Some(timestamp)).await?;

    Ok(PushResult {
        pushed,
        failed,
        remaining: sync::queue_len(pool).await?,
    })
}

/// The queue as shown to the user.
#[tauri::command]
pub async fn vndb_queue(state: State<'_, AppState>) -> CommandResult<Vec<QueueEntryView>> {
    let rows = sync::list_queue(state.db().pool()).await?;
    Ok(rows
        .into_iter()
        .map(|entry| QueueEntryView {
            vn_id: entry.vn_id,
            game_id: entry.game_id,
            created_at: entry.created_at,
            attempts: entry.attempts,
            last_error: entry.last_error,
        })
        .collect())
}

/// Recent sync log entries.
#[tauri::command]
pub async fn vndb_log(state: State<'_, AppState>) -> CommandResult<Vec<sync::LogEntry>> {
    Ok(sync::recent_log(state.db().pool(), 100).await?)
}

/// Available labels for the mapping UI.
#[tauri::command]
pub async fn vndb_labels(
    state: State<'_, AppState>,
) -> CommandResult<Vec<crate::vndb::ulist::UlistLabelInfo>> {
    let token = token()?;
    let account = sync::load_account(state.db().pool())
        .await?
        .ok_or_else(|| CommandError::new("not_connected", "connect a VNDB account first"))?;
    let response = state
        .vndb()
        .ulist_labels(token.expose(), &account.user_id)
        .await
        .map_err(|err| map_vndb(&err))?;
    Ok(response.labels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vndb::mapping::Resolution;
    use vn_core::model::GameStatus;

    #[test]
    fn a_decision_carries_a_stable_field_key() {
        let decision = FieldDecision {
            field: mapping::Field::Rating.key().to_owned(),
            resolution: Resolution::KeepRemote,
        };
        assert_eq!(decision.field, "rating");
    }

    #[test]
    fn game_status_parse_round_trips_through_the_api_strings() {
        for status in GameStatus::ALL {
            assert_eq!(GameStatus::parse(status.as_str()), Ok(status));
        }
        assert!(GameStatus::parse("teleported").is_err());
    }
}
