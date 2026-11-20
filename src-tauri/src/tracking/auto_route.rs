//! Device-local memory for the last route or chapter timed with the manual timer.
//!
//! Stores a global toggle and a per-game target in `sync_state` (device-local
//! configuration, never exported by folder sync or backups). Closing the game or
//! shutting down keeps the target so the next launch resumes timing it; an explicit
//! Stop from the user clears it.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use vn_core::ids::EntityId;

/// Global toggle in `sync_state` (`"true"` or `"false"`; absent defaults to `true`).
const AUTO_RESUME_KEY: &str = "tracking.autoResumeRoute";
const GAME_KEY_PREFIX: &str = "tracking.lastRoute.";
const MAX_MERGE_HOPS: usize = 8;

/// Persisted target stored per game in `sync_state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutoResumeTarget {
    Route { route_id: String },
    Chapter { chapter_id: String },
}

/// Live, validated target ready to start on game launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedAutoTarget {
    Route { route_id: EntityId },
    Chapter { chapter_id: EntityId },
}

/// Resolved target paired with its user-visible label for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribedAutoTarget {
    pub route_id: Option<EntityId>,
    pub chapter_id: Option<EntityId>,
    pub name: String,
}

fn game_key(game_id: EntityId) -> String {
    format!("{GAME_KEY_PREFIX}{game_id}")
}

/// Extract an auto-resume target from a running or finishing [`crate::tracking::manual::ManualRun`].
#[must_use]
pub fn target_from_manual(run: &crate::tracking::manual::ManualRun) -> Option<AutoResumeTarget> {
    if let Some(chapter_id) = run.chapter_id {
        Some(AutoResumeTarget::Chapter {
            chapter_id: chapter_id.to_string(),
        })
    } else {
        run.route_id.map(|route_id| AutoResumeTarget::Route {
            route_id: route_id.to_string(),
        })
    }
}

/// Read whether auto-resuming the last route/chapter timer on game launch is enabled.
/// Defaults to `true` unless explicitly stored as `"false"`.
pub async fn load_enabled(pool: &SqlitePool) -> bool {
    let found: Result<Option<(String,)>, _> =
        sqlx::query_as("SELECT value FROM sync_state WHERE key = ?")
            .bind(AUTO_RESUME_KEY)
            .fetch_optional(pool)
            .await;
    found
        .ok()
        .flatten()
        .is_none_or(|(value,)| value.trim() != "false")
}

/// Store the global auto-resume toggle in `sync_state`.
pub async fn save_enabled(pool: &SqlitePool, enabled: bool) {
    let value = if enabled { "true" } else { "false" };
    if let Err(err) = sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(AUTO_RESUME_KEY)
    .bind(value)
    .execute(pool)
    .await
    {
        eprintln!("failed to save auto-resume route toggle: {err}");
    }
}

/// Persist the last timed route or chapter for `game_id`.
pub async fn remember(pool: &SqlitePool, game_id: EntityId, target: &AutoResumeTarget) {
    let Ok(payload) = serde_json::to_string(target) else {
        return;
    };
    if let Err(err) = sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(game_key(game_id))
    .bind(payload)
    .execute(pool)
    .await
    {
        eprintln!("failed to remember auto-resume target: {err}");
    }
}

/// Forget any remembered target for `game_id`.
pub async fn clear(pool: &SqlitePool, game_id: EntityId) {
    if let Err(err) = sqlx::query("DELETE FROM sync_state WHERE key = ?")
        .bind(game_key(game_id))
        .execute(pool)
        .await
    {
        eprintln!("failed to clear auto-resume target: {err}");
    }
}

/// Follow up to 8 `routes.merged_into` hops to find the live surviving route and its name.
pub async fn resolve_route_survivor(
    pool: &SqlitePool,
    game_id: EntityId,
    route_id: &str,
) -> Option<(EntityId, String)> {
    let mut current = EntityId::parse(route_id).ok()?.to_string();
    let game = game_id.to_string();
    for _ in 0..MAX_MERGE_HOPS {
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT name, merged_into FROM routes WHERE id = ? AND game_id = ? AND deleted = 0",
        )
        .bind(&current)
        .bind(&game)
        .fetch_optional(pool)
        .await
        .ok()?;
        let (name, merged_into) = row?;
        match merged_into {
            None => {
                let id = EntityId::parse(&current).ok()?;
                return Some((id, name));
            }
            Some(next) => current = next,
        }
    }
    None
}

/// Resolve and describe the remembered target for `game_id`, cleaning up stale keys.
pub async fn describe_for_game(
    pool: &SqlitePool,
    game_id: EntityId,
) -> Option<DescribedAutoTarget> {
    let raw: String = sqlx::query_scalar("SELECT value FROM sync_state WHERE key = ?")
        .bind(game_key(game_id))
        .fetch_optional(pool)
        .await
        .ok()??;
    let Ok(target) = serde_json::from_str::<AutoResumeTarget>(&raw) else {
        clear(pool, game_id).await;
        return None;
    };

    match target {
        AutoResumeTarget::Route { route_id } => {
            if let Some((survivor, name)) = resolve_route_survivor(pool, game_id, &route_id).await {
                Some(DescribedAutoTarget {
                    route_id: Some(survivor),
                    chapter_id: None,
                    name,
                })
            } else {
                clear(pool, game_id).await;
                None
            }
        }
        AutoResumeTarget::Chapter { chapter_id } => {
            let Ok(cid) = EntityId::parse(&chapter_id) else {
                clear(pool, game_id).await;
                return None;
            };
            let name: Option<String> = sqlx::query_scalar(
                "SELECT COALESCE(NULLIF(display_name, ''), name) \
                 FROM chapters WHERE id = ? AND game_id = ? AND deleted = 0",
            )
            .bind(cid.to_string())
            .bind(game_id.to_string())
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
            if let Some(name) = name {
                Some(DescribedAutoTarget {
                    route_id: None,
                    chapter_id: Some(cid),
                    name,
                })
            } else {
                clear(pool, game_id).await;
                None
            }
        }
    }
}

/// Resolve the remembered target for `game_id`, following merged routes and pruning deleted rows.
pub async fn resolve_for_game(pool: &SqlitePool, game_id: EntityId) -> Option<ResolvedAutoTarget> {
    let described = describe_for_game(pool, game_id).await?;
    if let Some(chapter_id) = described.chapter_id {
        Some(ResolvedAutoTarget::Chapter { chapter_id })
    } else {
        described
            .route_id
            .map(|route_id| ResolvedAutoTarget::Route { route_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, queries};
    use crate::tracking::mapping;
    use vn_core::clock::Timestamp;
    use vn_core::ids::DeviceId;
    use vn_core::model::{Game, GameStatus, Route};

    async fn seed_game(pool: &SqlitePool, device: &DeviceId, title: &str) -> EntityId {
        let now = Timestamp::from_millis(1_000);
        let game = Game::new(device, now, title, GameStatus::Playing).expect("valid game");
        let id = game.meta.id;
        queries::upsert_game(pool, &game).await.expect("seed game");
        id
    }

    async fn seed_route(
        pool: &SqlitePool,
        device: &DeviceId,
        game_id: EntityId,
        name: &str,
        merged_into: Option<EntityId>,
    ) -> EntityId {
        let now = Timestamp::from_millis(1_000);
        let mut route = Route::new(device, now, game_id, name).expect("valid route");
        route.merged_into = merged_into;
        let id = route.meta.id;
        queries::upsert_route(pool, &route)
            .await
            .expect("seed route");
        id
    }

    #[tokio::test]
    async fn toggle_defaults_to_true_and_persists_changes() {
        let db = Db::open_memory().await.expect("db");
        assert!(load_enabled(db.pool()).await);

        save_enabled(db.pool(), false).await;
        assert!(!load_enabled(db.pool()).await);

        save_enabled(db.pool(), true).await;
        assert!(load_enabled(db.pool()).await);
    }

    #[tokio::test]
    async fn remember_and_clear_are_scoped_per_game_for_routes_and_chapters() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let g1 = seed_game(db.pool(), &device, "Game 1").await;
        let g2 = seed_game(db.pool(), &device, "Game 2").await;
        let r1 = seed_route(db.pool(), &device, g1, "Shiroha", None).await;

        let now = Timestamp::from_millis(1_000);
        let c2 = mapping::insert_manual_chapter(
            db.pool(),
            g2,
            "Prologue Edited",
            None,
            None,
            &device,
            now,
        )
        .await
        .expect("seed chapter");

        remember(
            db.pool(),
            g1,
            &AutoResumeTarget::Route {
                route_id: r1.to_string(),
            },
        )
        .await;
        remember(
            db.pool(),
            g2,
            &AutoResumeTarget::Chapter {
                chapter_id: c2.to_string(),
            },
        )
        .await;

        assert_eq!(
            resolve_for_game(db.pool(), g1).await,
            Some(ResolvedAutoTarget::Route { route_id: r1 })
        );
        let described_g2 = describe_for_game(db.pool(), g2).await.expect("g2");
        assert_eq!(described_g2.chapter_id, Some(c2));
        assert_eq!(described_g2.route_id, None);
        assert_eq!(described_g2.name, "Prologue Edited");

        clear(db.pool(), g1).await;
        assert_eq!(resolve_for_game(db.pool(), g1).await, None);
        assert_eq!(
            resolve_for_game(db.pool(), g2).await,
            Some(ResolvedAutoTarget::Chapter { chapter_id: c2 })
        );
    }

    #[tokio::test]
    async fn merged_route_resolves_to_survivor_and_cycles_clean_up() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "Summer Pockets").await;
        let survivor = seed_route(db.pool(), &device, game, "Shiroha True", None).await;
        let merged = seed_route(db.pool(), &device, game, "Shiroha Old", Some(survivor)).await;

        remember(
            db.pool(),
            game,
            &AutoResumeTarget::Route {
                route_id: merged.to_string(),
            },
        )
        .await;

        let described = describe_for_game(db.pool(), game)
            .await
            .expect("survivor resolved");
        assert_eq!(described.route_id, Some(survivor));
        assert_eq!(described.name, "Shiroha True");

        // A self-referential / cyclical merge chain terminates and clears the stale entry.
        sqlx::query("UPDATE routes SET merged_into = ? WHERE id = ?")
            .bind(merged.to_string())
            .bind(survivor.to_string())
            .execute(db.pool())
            .await
            .expect("create cycle");
        assert_eq!(resolve_for_game(db.pool(), game).await, None);
        let stored: Option<String> =
            sqlx::query_scalar("SELECT value FROM sync_state WHERE key = ?")
                .bind(game_key(game))
                .fetch_optional(db.pool())
                .await
                .expect("check cleaned");
        assert_eq!(stored, None);
    }

    #[tokio::test]
    async fn deleted_targets_return_none_and_prune_stale_state() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = seed_game(db.pool(), &device, "Deleted Targets").await;
        let route = seed_route(db.pool(), &device, game, "Temp Route", None).await;

        remember(
            db.pool(),
            game,
            &AutoResumeTarget::Route {
                route_id: route.to_string(),
            },
        )
        .await;
        sqlx::query("UPDATE routes SET deleted = 1 WHERE id = ?")
            .bind(route.to_string())
            .execute(db.pool())
            .await
            .expect("soft delete route");

        assert_eq!(resolve_for_game(db.pool(), game).await, None);
        let remaining: Option<String> =
            sqlx::query_scalar("SELECT value FROM sync_state WHERE key = ?")
                .bind(game_key(game))
                .fetch_optional(db.pool())
                .await
                .expect("check cleaned");
        assert_eq!(remaining, None);
    }
}
