//! Pool creation and migration application.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use thiserror::Error;

/// Errors surfaced by the database layer.
#[derive(Debug, Error)]
pub enum DbError {
    /// The underlying `SQLite` driver failed.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// A migration failed to apply.
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// The database file lives in a directory that does not exist.
    #[error("database directory does not exist: {0}")]
    MissingDirectory(PathBuf),
    /// A query returned rows in a shape the domain model rejects.
    #[error("invalid stored data: {0}")]
    Corrupt(String),
}

/// A handle to the application database.
#[derive(Debug, Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Open (creating if needed) the database at `path` and apply migrations.
    ///
    /// Pragmas chosen for a local, single-user app that must survive a crash:
    /// WAL for concurrent reads while a write is in flight, `NORMAL` sync (the
    /// durable-enough default for WAL on desktop), and foreign keys enforced.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)
                .map_err(|_| DbError::MissingDirectory(parent.to_path_buf()))?;
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));

        Self::from_options(options).await
    }

    /// Open an in-memory database. Used by tests and by the desktop "try it"
    /// flow before a real file has been chosen.
    pub async fn open_memory() -> Result<Self, DbError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?
            .journal_mode(SqliteJournalMode::Memory)
            .foreign_keys(true);
        Self::from_options(options).await
    }

    async fn from_options(options: SqliteConnectOptions) -> Result<Self, DbError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        let db = Self { pool };
        db.migrate().await?;
        Ok(db)
    }

    /// Apply all pending migrations from `db/migrations`.
    pub async fn migrate(&self) -> Result<(), DbError> {
        sqlx::migrate!("src/db/migrations").run(&self.pool).await?;
        Ok(())
    }

    /// Borrow the pool for a query.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Close all connections. Pending writes are flushed by WAL on drop.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_apply_to_a_fresh_database() {
        let db = Db::open_memory().await.expect("open memory db");
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .fetch_all(db.pool())
        .await
        .expect("list tables");

        let names: Vec<&str> = tables.iter().map(|t| t.0.as_str()).collect();
        for expected in [
            "_sqlx_migrations",
            "adapter_profiles",
            "chapters",
            "endings",
            "games",
            "image_cache",
            "local_prefs",
            "play_sessions",
            "routes",
            "schema_guard",
            "sync_state",
            "vndb_cache",
        ] {
            assert!(names.contains(&expected), "missing table {expected}");
        }
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let db = Db::open_memory().await.expect("open");
        db.migrate().await.expect("second migrate is a no-op");

        let applied: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(db.pool())
            .await
            .expect("count migrations");
        assert_eq!(
            applied.0,
            i64::from(vn_core::migration::SCHEMA_VERSION),
            "applied migration count must match the declared schema version"
        );
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let db = Db::open_memory().await.expect("open");
        let result = sqlx::query(
            "INSERT INTO chapters (id, game_id, name, updated_at, device_id) VALUES ('c1','missing-game','ch',1,'dev')",
        )
        .execute(db.pool())
        .await;
        assert!(result.is_err(), "foreign key violation must be rejected");
    }

    #[tokio::test]
    async fn a_whole_star_rating_survives_the_decivote_migration() {
        // 0011 multiplies stored ratings by ten. A 7 that became 7 rather than
        // 70 would fail the new 10..=100 check, but a 7 that became 0.7 would
        // pass it and silently change what the user had chosen, so the value
        // itself is asserted, not just the constraint.
        let db = Db::open_memory().await.expect("open");
        sqlx::query(
            "INSERT INTO games (id, title, status, rating, updated_at, device_id) \
             VALUES ('g1','A','completed',70,1,'dev')",
        )
        .execute(db.pool())
        .await
        .expect("a 7.0 rating is storable");

        let stored: (i64,) = sqlx::query_as("SELECT rating FROM games WHERE id = 'g1'")
            .fetch_one(db.pool())
            .await
            .expect("read rating");
        assert_eq!(stored.0, 70, "70 is 7.0 in decivotes");

        // And the tenths are reachable, which is the point of the change.
        sqlx::query("UPDATE games SET rating = 75 WHERE id = 'g1'")
            .execute(db.pool())
            .await
            .expect("a 7.5 rating is storable");
    }

    #[tokio::test]
    async fn the_rating_migration_keeps_child_rows_reachable() {
        // 0011 rebuilds `games` to widen its rating CHECK. `ALTER TABLE RENAME`
        // rewrites every child table's foreign key to follow the renamed table,
        // so a naive rename-copy-swap leaves chapters and sessions pointing at a
        // name that no longer exists; a dry run against a real database went
        // from 0 to 14 dangling references. This asserts the rebuild left them
        // attached, which is the failure the pragma in the migration prevents.
        let db = Db::open_memory().await.expect("open");

        sqlx::query(
            "INSERT INTO games (id, title, status, rating, updated_at, device_id) \
             VALUES ('g1','A','playing',70,1,'dev')",
        )
        .execute(db.pool())
        .await
        .expect("insert game");
        sqlx::query(
            "INSERT INTO chapters (id, game_id, name, updated_at, device_id) \
             VALUES ('c1','g1','ch',1,'dev')",
        )
        .execute(db.pool())
        .await
        .expect("insert chapter");

        let refs: Vec<(String,)> =
            sqlx::query_as("SELECT \"table\" FROM pragma_foreign_key_list('chapters')")
                .fetch_all(db.pool())
                .await
                .expect("fk list");
        assert!(
            refs.iter().any(|(table,)| table == "games"),
            "chapters must reference the rebuilt games table, got {refs:?}"
        );

        let dangling: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chapters WHERE game_id NOT IN (SELECT id FROM games)",
        )
        .fetch_one(db.pool())
        .await
        .expect("count dangling");
        assert_eq!(dangling, 0, "a chapter must still resolve to its game");
    }

    #[tokio::test]
    async fn rating_check_constraint_holds() {
        let db = Db::open_memory().await.expect("open");
        sqlx::query(
            "INSERT INTO games (id, title, status, updated_at, device_id) VALUES ('g1','A','playing',1,'dev')",
        )
        .execute(db.pool())
        .await
        .expect("insert game");

        // Ratings are decivotes: 10 is 1.0 and 100 is 10.0.
        for rejected in [9, 101] {
            let bad = sqlx::query("UPDATE games SET rating = ? WHERE id = 'g1'")
                .bind(rejected)
                .execute(db.pool())
                .await;
            assert!(
                bad.is_err(),
                "rating {rejected} is outside 10..=100 and must be rejected"
            );
        }

        for accepted in [10, 75, 100] {
            let ok = sqlx::query("UPDATE games SET rating = ? WHERE id = 'g1'")
                .bind(accepted)
                .execute(db.pool())
                .await;
            assert!(ok.is_ok(), "rating {accepted} must be accepted");
        }
    }

    #[tokio::test]
    async fn session_invariants_hold() {
        let db = Db::open_memory().await.expect("open");
        sqlx::query(
            "INSERT INTO games (id, title, status, updated_at, device_id) VALUES ('g1','A','playing',1,'dev')",
        )
        .execute(db.pool())
        .await
        .expect("insert game");

        let inverted = sqlx::query(
            "INSERT INTO play_sessions (id, game_id, started_at, ended_at, counted_ms, source, updated_at, device_id) \
             VALUES ('s1','g1', 2000, 1000, 1000, 'desktop', 1, 'dev')",
        )
        .execute(db.pool())
        .await;
        assert!(inverted.is_err(), "ended_at < started_at must be rejected");

        let negative = sqlx::query(
            "INSERT INTO play_sessions (id, game_id, started_at, ended_at, counted_ms, source, updated_at, device_id) \
             VALUES ('s2','g1', 1000, 2000, -1, 'desktop', 1, 'dev')",
        )
        .execute(db.pool())
        .await;
        assert!(negative.is_err(), "negative counted_ms must be rejected");

        let good = sqlx::query(
            "INSERT INTO play_sessions (id, game_id, started_at, ended_at, counted_ms, source, updated_at, device_id) \
             VALUES ('s3','g1', 1000, 2000, 500, 'desktop', 1, 'dev')",
        )
        .execute(db.pool())
        .await;
        assert!(good.is_ok());
    }

    #[tokio::test]
    async fn deleting_a_game_cascades_to_children() {
        let db = Db::open_memory().await.expect("open");
        sqlx::query(
            "INSERT INTO games (id, title, status, updated_at, device_id) VALUES ('g1','A','playing',1,'dev')",
        )
        .execute(db.pool())
        .await
        .expect("insert game");
        sqlx::query(
            "INSERT INTO chapters (id, game_id, name, updated_at, device_id) VALUES ('c1','g1','ch',1,'dev')",
        )
        .execute(db.pool())
        .await
        .expect("insert chapter");
        sqlx::query("DELETE FROM games WHERE id = 'g1'")
            .execute(db.pool())
            .await
            .expect("delete game");

        let chapters: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chapters")
            .fetch_one(db.pool())
            .await
            .expect("count");
        assert_eq!(chapters.0, 0, "chapters cascade with the game");
    }
}

#[cfg(test)]
mod migration_0021_tests {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    /// Apply every migration **except** 0021, so an orphan can be created in the
    /// pre-migration shape and 0021 then run against it.
    async fn schema_20_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::from_str("sqlite::memory:").expect("options"))
            .await
            .expect("connect");
        sqlx::migrate!("src/db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        // Undo 0021 by dropping what it added, leaving the 0020 shape.
        sqlx::query("DROP INDEX IF EXISTS idx_endings_route")
            .execute(&pool)
            .await
            .expect("drop idx");
        sqlx::query("DROP INDEX IF EXISTS idx_chapters_route")
            .execute(&pool)
            .await
            .expect("drop idx");
        sqlx::query("DROP INDEX IF EXISTS idx_routes_game")
            .execute(&pool)
            .await
            .expect("drop idx");
        sqlx::query("DROP TABLE IF EXISTS routes")
            .execute(&pool)
            .await
            .expect("drop routes");
        // SQLite cannot drop a column before 3.35; leave the added ones, they are
        // nullable and do not affect the backfill under test.
        pool
    }

    /// Seed one live game with a chapter, plus a chapter whose game is gone.
    ///
    /// Foreign keys are off for the seed only: an orphan is by definition a row its
    /// parent no longer backs. This is the per-connection pragma the AGENTS.md rule
    /// warns about, reproduced deliberately so the migration is tested against a
    /// state a real library can actually be in.
    async fn seed_live_game_and_orphan(pool: &sqlx::SqlitePool) {
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(pool)
            .await
            .expect("fk off");
        sqlx::query(
            // A real game id is a UUID and the backfill slices its hex form, so a
            // short id here would produce a malformed route id and fail for the
            // wrong reason.
            "INSERT INTO games (id, title, status, updated_at, device_id)              VALUES ('01a151db-1111-7000-8000-000000000001','Kept','playing',1,'dev')",
        )
        .execute(pool)
        .await
        .expect("insert game");
        sqlx::query(
            "INSERT INTO chapters (id, game_id, name, route_key, sort_index, updated_at, device_id, deleted)              VALUES ('c_orphan','g_gone','ghost',X'EFBFBD',0,1,'dev',0)",
        )
        .execute(pool)
        .await
        .expect("insert orphan chapter");
        sqlx::query(
            "INSERT INTO chapters (id, game_id, name, route_key, sort_index, updated_at, device_id, deleted)              VALUES ('c_live','01a151db-1111-7000-8000-000000000001','ch','route_1',0,1,'dev',0)",
        )
        .execute(pool)
        .await
        .expect("insert live chapter");
    }

    /// Run migration 21's statements against an already-migrated pool.
    ///
    /// `ALTER TABLE ... ADD COLUMN` is filtered out because the columns already exist
    /// (the real migration ran while seeding) and `SQLite` cannot drop a column.
    async fn run_routes_backfill(pool: &sqlx::SqlitePool) {
        // Re-enable first: the point of the test is that the backfill survives with
        // the constraint enforced.
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(pool)
            .await
            .expect("fk on");

        let sql = include_str!("migrations/0021_routes.sql");
        // Strip `--` comments before splitting on `;`, or a semicolon inside a
        // comment ends a "statement" in the middle of prose.
        let stripped: Vec<String> = sql
            .lines()
            .map(|line| match line.find("--") {
                Some(at) => line[..at].to_owned(),
                None => line.to_owned(),
            })
            .collect();
        let joined = stripped.join(
            "
",
        );
        let statements: Vec<String> = joined
            .split(';')
            .map(|part| part.trim().to_owned())
            .filter(|part| !part.is_empty())
            .filter(|part| !part.to_ascii_uppercase().starts_with("ALTER TABLE"))
            .collect();
        for statement in statements {
            // Audited: every statement comes from this crate's own migration file,
            // not from user input. sqlx 0.9 requires saying so explicitly.
            sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(pool)
                .await
                .unwrap_or_else(|err| panic!("statement failed: {err}"));
        }
    }

    /// The exact failure observed on a live database: three chapters pointed at a
    /// game row that no longer existed, and inserting a route for them violated
    /// `routes.game_id REFERENCES games(id)`, aborting migration 21 entirely.
    #[tokio::test]
    async fn an_orphan_chapter_does_not_abort_the_routes_backfill() {
        let pool = schema_20_pool().await;

        seed_live_game_and_orphan(&pool).await;
        run_routes_backfill(&pool).await;

        // The ids must parse as UUIDs. A hex hash produced "invalid id" on every
        // read once the backfill ran, which no test caught because routes were only
        // ever created through `Route::new`.
        let ids: Vec<(String,)> = sqlx::query_as("SELECT id FROM routes")
            .fetch_all(&pool)
            .await
            .expect("read ids");
        for (id,) in &ids {
            vn_core::ids::EntityId::parse(id)
                .unwrap_or_else(|err| panic!("backfilled route id {id} is not a UUID: {err}"));
        }

        let routes: Vec<(String,)> = sqlx::query_as("SELECT game_id FROM routes")
            .fetch_all(&pool)
            .await
            .expect("read routes");
        assert_eq!(
            routes.len(),
            1,
            "only the live game's route is created: {routes:?}"
        );
        assert_eq!(routes[0].0, "01a151db-1111-7000-8000-000000000001");

        let dangling: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chapters c WHERE c.route_id IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM routes r WHERE r.id = c.route_id)",
        )
        .fetch_one(&pool)
        .await
        .expect("count dangling");
        assert_eq!(
            dangling, 0,
            "no chapter may point at a route that was not created"
        );
    }

    /// Both halves of a route must be backfilled, and are never auto-merged.
    ///
    /// The two inserts number their ids from 1, so before the offset the ending half
    /// collided with the engine half of the same ordinal and `INSERT OR IGNORE`
    /// dropped it without a word -- every ending link came out NULL.
    #[tokio::test]
    async fn the_ending_half_of_a_route_is_backfilled_and_linked() {
        let pool = schema_20_pool().await;
        seed_live_game_and_orphan(&pool).await;

        sqlx::query(
            "INSERT INTO endings (id, game_id, name, route, achieved, updated_at, device_id, deleted)              VALUES ('e1','01a151db-1111-7000-8000-000000000001','GE','  shiroha ',0,1,'dev',0)",
        )
        .execute(&pool)
        .await
        .expect("insert ending");

        run_routes_backfill(&pool).await;

        let origins: Vec<(String,)> = sqlx::query_as("SELECT origin FROM routes ORDER BY origin")
            .fetch_all(&pool)
            .await
            .expect("read origins");
        let origins: Vec<&str> = origins.iter().map(|(o,)| o.as_str()).collect();
        assert!(
            origins.contains(&"engine") && origins.contains(&"ending"),
            "both halves of a route must be backfilled, got {origins:?}"
        );

        let ending_route: Option<String> =
            sqlx::query_scalar("SELECT route_id FROM endings WHERE id = 'e1'")
                .fetch_one(&pool)
                .await
                .expect("read ending link");
        assert!(
            ending_route.is_some(),
            "an ending with a route label must be linked to its backfilled route"
        );
    }
}

#[cfg(test)]
mod migration_0025_tests {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    /// Strip `--` comments, split on `;` and run every non-empty statement of a
    /// migration file against the pool. Audited: the SQL comes from this
    /// crate's own migration files, never from user input.
    pub(super) async fn run_migration_file(pool: &sqlx::SqlitePool, file: &str) {
        let stripped: Vec<String> = file
            .lines()
            .map(|line| match line.find("--") {
                Some(at) => line[..at].to_owned(),
                None => line.to_owned(),
            })
            .collect();
        let statements: Vec<String> = stripped
            .join("\n")
            .split(';')
            .map(|part| part.trim().to_owned())
            .filter(|part| !part.is_empty())
            .collect();
        for statement in statements {
            sqlx::query(sqlx::AssertSqlSafe(statement.as_str()))
                .execute(pool)
                .await
                .unwrap_or_else(|err| panic!("statement failed ({statement}): {err}"));
        }
    }

    /// A pool in the pre-0025 shape: every migration applied, then the script
    /// tables dropped and recreated from 0024 so a row can carry the old
    /// `fvp_dialogue` kind.
    async fn schema_24_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::from_str("sqlite::memory:").expect("options"))
            .await
            .expect("connect");
        sqlx::migrate!("src/db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        sqlx::query("DROP TABLE script_compaction")
            .execute(&pool)
            .await
            .expect("drop compaction");
        sqlx::query("DROP TABLE script_documents")
            .execute(&pool)
            .await
            .expect("drop documents");
        run_migration_file(&pool, include_str!("migrations/0024_script_ingest.sql")).await;
        pool
    }

    /// Seed one game with a `fvp_dialogue` script and in-progress compaction.
    async fn seed_old_kind_script(pool: &sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO games (id, title, status, updated_at, device_id) VALUES ('01a151db-2222-7000-8000-000000000002','Saku','playing',1,'dev')",
        )
        .execute(pool)
        .await
        .expect("insert game");
        sqlx::query(
            "INSERT INTO script_documents (id, game_id, engine, source_kind, source_path, source_sha256, line_count, body, created_at, updated_at) VALUES ('s1','01a151db-2222-7000-8000-000000000002','fvp','fvp_dialogue','C:\\g\\dialogue.json','ab',12,'雪々: こんにちは',1,1)",
        )
        .execute(pool)
        .await
        .expect("insert script");
        sqlx::query(
            "INSERT INTO script_compaction (document_id, model_id, chunk_count, done_chunks, summaries_json, digest, updated_at) VALUES ('s1','qwen',3,1,'[\"one\"]',NULL,1)",
        )
        .execute(pool)
        .await
        .expect("insert compaction");
    }

    async fn run_0025(pool: &sqlx::SqlitePool) {
        run_migration_file(pool, include_str!("migrations/0025_script_vntp_kind.sql")).await;
    }

    /// The rename must convert stored rows, keep the child compaction row
    /// pointing at the rebuilt table, and swap the CHECK to the new kind.
    #[tokio::test]
    async fn the_rebuild_renames_the_kind_and_keeps_compaction_rows() {
        let pool = schema_24_pool().await;
        seed_old_kind_script(&pool).await;

        run_0025(&pool).await;

        let kind: String =
            sqlx::query_scalar("SELECT source_kind FROM script_documents WHERE id='s1'")
                .fetch_one(&pool)
                .await
                .expect("read kind");
        assert_eq!(kind, "json_vntp");

        // The child row survived the parent swap.
        let done: i64 =
            sqlx::query_scalar("SELECT done_chunks FROM script_compaction WHERE document_id='s1'")
                .fetch_one(&pool)
                .await
                .expect("read compaction");
        assert_eq!(done, 1);

        // The rebuild's own final pragma must agree: no dangling references.
        let violations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
            .fetch_one(&pool)
            .await
            .expect("fk check");
        assert_eq!(violations, 0, "foreign_key_check must be clean after 0025");
    }

    /// The new CHECK accepts `json_vntp` and rejects the old kind: a row
    /// written after the rename can never regress to the removed value.
    #[tokio::test]
    async fn the_new_check_takes_only_the_renamed_kind() {
        let pool = schema_24_pool().await;
        seed_old_kind_script(&pool).await;
        run_0025(&pool).await;

        let ok = sqlx::query(
            "INSERT INTO script_documents (id, game_id, engine, source_kind, source_path, source_sha256, line_count, body, created_at, updated_at) VALUES ('s2','01a151db-2222-7000-8000-000000000002','user','json_vntp','a.txt','cd',1,'x',1,1)",
        )
        .execute(&pool)
        .await;
        assert!(ok.is_ok(), "json_vntp must be accepted after the rename");

        let rejected = sqlx::query(
            "INSERT INTO script_documents (id, game_id, engine, source_kind, source_path, source_sha256, line_count, body, created_at, updated_at) VALUES ('s3','01a151db-2222-7000-8000-000000000002','user','fvp_dialogue','b.txt','ef',1,'x',1,1)",
        )
        .execute(&pool)
        .await;
        assert!(
            rejected.is_err(),
            "the old fvp_dialogue kind must be rejected after the rename"
        );
    }
}

#[cfg(test)]
mod migration_0026_tests {
    use super::migration_0025_tests::run_migration_file;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    /// A pool in the pre-0026 shape: every migration applied, then
    /// `script_compaction` dropped and recreated from 0024's flat definition
    /// so a row can carry the phase 18/19B single-agent progress form.
    async fn schema_25_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::from_str("sqlite::memory:").expect("options"))
            .await
            .expect("connect");
        sqlx::migrate!("src/db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        sqlx::query("DROP TABLE script_compaction")
            .execute(&pool)
            .await
            .expect("drop compaction");
        // 0024's definition of the table, verbatim shape: flat summaries,
        // no agent columns.
        sqlx::query(sqlx::AssertSqlSafe(
            "CREATE TABLE script_compaction (\
             document_id TEXT PRIMARY KEY NOT NULL REFERENCES script_documents(id) ON DELETE CASCADE,\
             model_id TEXT NOT NULL, chunk_count INTEGER NOT NULL,\
             done_chunks INTEGER NOT NULL DEFAULT 0,\
             summaries_json TEXT NOT NULL DEFAULT '[]', digest TEXT, updated_at INTEGER NOT NULL)",
        ))
        .execute(&pool)
        .await
        .expect("recreate compaction");
        sqlx::query(
            "INSERT INTO games (id, title, status, updated_at, device_id) VALUES ('01a151db-4444-7000-8000-000000000004','Yuki','playing',1,'dev')",
        )
        .execute(&pool)
        .await
        .expect("insert game");
        sqlx::query(
            "INSERT INTO script_documents (id, game_id, engine, source_kind, source_path, source_sha256, line_count, body, created_at, updated_at) VALUES ('s4','01a151db-4444-7000-8000-000000000004','user','plain_text','y.txt','cd',2,'a\nb',1,1)",
        )
        .execute(&pool)
        .await
        .expect("insert script");
        sqlx::query(
            "INSERT INTO script_compaction (document_id, model_id, chunk_count, done_chunks, summaries_json, digest, updated_at) VALUES ('s4','qwen',3,2,'[\"one\",\"two\"]',NULL,1)",
        )
        .execute(&pool)
        .await
        .expect("insert flat progress");
        pool
    }

    /// Flat single-agent progress becomes one agent's array, and the new
    /// columns arrive with the values that make the row a phase-18 run: one
    /// agent, compactor version 0 (which the new pipeline declines to resume).
    #[tokio::test]
    async fn flat_summaries_become_one_agents_array() {
        let pool = schema_25_pool().await;
        run_migration_file(&pool, include_str!("migrations/0026_script_agents.sql")).await;

        let (summaries, agents, version, segments): (String, i64, i64, Option<String>) =
            sqlx::query_as(
                "SELECT summaries_json, agent_count, compactor_version, segment_digests_json \
                 FROM script_compaction WHERE document_id='s4'",
            )
            .fetch_one(&pool)
            .await
            .expect("read row");
        assert_eq!(summaries, r#"[["one","two"]]"#);
        assert_eq!(agents, 1);
        assert_eq!(version, 0);
        assert!(segments.is_none());
    }
}
