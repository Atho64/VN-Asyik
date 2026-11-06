//! Local memory storage. Mutations run under `CompanionRuntime`'s database gate.
pub mod jobs;
mod service;
pub use service::TurnMemory;
#[cfg(test)]
mod tests;

use super::store::Session;
use crate::ai::conversation::store::now;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use vn_core::companion::{
    Knowledge,
    memory::{
        Class, Context, Evidence, MAX_EVIDENCE, MAX_EVIDENCE_TOTAL, MAX_INPUT, MAX_ITEMS, MAX_JOBS,
        MAX_MARKERS, MAX_OUTPUT, MAX_PINS, Memory, POLICY_VERSION, PROMPT_VERSION, RECAP_LIFETIME,
        Scope, WEEK, body_valid,
    },
};

pub(super) fn json<T: Serialize + ?Sized>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|_| "invalid_memory".into())
}
fn tag<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| "invalid_memory".into())
}
fn parse<T: DeserializeOwned>(value: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(value.into()))
        .map_err(|_| "invalid_memory".into())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub revision: i64,
    pub retrieval_enabled: bool,
    pub auto_summary: bool,
    pub gap: Option<String>,
}
pub async fn settings(pool: &SqlitePool) -> Result<Settings, String> {
    let row = sqlx::query("SELECT * FROM companion_memory_state WHERE id=1")
        .fetch_one(pool)
        .await
        .map_err(|_| "db_error")?;
    Ok(Settings {
        revision: row.get("revision"),
        retrieval_enabled: row.get("retrieval_enabled"),
        auto_summary: row.get("auto_summary"),
        gap: row.get("gap"),
    })
}

fn decode(row: &sqlx::sqlite::SqliteRow) -> Result<Memory, String> {
    Ok(Memory {
        id: row.get("id"),
        version: row.get("version"),
        class: parse(row.get("class"))?,
        scope: parse(row.get("scope"))?,
        companion_id: row.get("companion_id"),
        game_id: row.get("game_id"),
        playthrough_id: row.get("playthrough_id"),
        knowledge: parse(row.get("knowledge"))?,
        body: row.get("body"),
        evidence: parse(row.get("evidence"))?,
        pinned: row.get("pinned"),
        origin_session: row.get("origin_session"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        expires_at: row.get("expires_at"),
    })
}

pub async fn live(pool: &SqlitePool) -> Result<Vec<Memory>, String> {
    sqlx::query("SELECT * FROM companion_memories WHERE deleted=0 AND approved=1 AND (pinned=1 OR expires_at IS NULL OR expires_at>?) ORDER BY updated_at DESC,id LIMIT 1000")
        .bind(now()).fetch_all(pool).await.map_err(|_| "db_error")?.iter().map(decode).collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub kind: String,
    pub origin_session: Option<String>,
    pub message_id: Option<String>,
    pub knowledge: Knowledge,
    pub evidence: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub context: Option<Context>,
    pub settings: Settings,
    pub items: Vec<Memory>,
    pub jobs: Vec<jobs::JobView>,
    pub replay_excluded: bool,
    pub item_count: i64,
    pub pin_count: i64,
    pub marker_count: i64,
}

pub async fn view(pool: &SqlitePool, session: Option<&str>) -> Result<View, String> {
    let context = if let Some(id) = session {
        super::store::list(pool)
            .await?
            .into_iter()
            .find(|s| s.id == id)
            .map(|s| Context::new(&s.options, &s.playthrough_id))
    } else {
        None
    };
    let (item_count, pin_count): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),COALESCE(SUM(pinned),0) FROM companion_memories WHERE deleted=0",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "db_error")?;
    let (marker_count,): (i64,) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM companion_memories)+(SELECT COUNT(*) FROM companion_memory_jobs)")
        .fetch_one(pool).await.map_err(|_| "db_error")?;
    let (replay_excluded,): (bool,) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM companion_replay_barriers b JOIN companion_sessions s ON s.thread_id=b.thread_id WHERE s.id=?)")
        .bind(session).fetch_one(pool).await.map_err(|_| "db_error")?;
    Ok(View {
        context,
        settings: settings(pool).await?,
        items: live(pool).await?,
        jobs: jobs::list(pool).await?,
        replay_excluded,
        item_count,
        pin_count,
        marker_count,
    })
}

pub async fn sources(pool: &SqlitePool, id: &str, version: i64) -> Result<Vec<Source>, String> {
    sqlx::query("SELECT s.* FROM companion_memory_sources s JOIN companion_memories m ON m.id=s.memory_id AND m.version=s.memory_version AND m.deleted=0 WHERE m.id=? AND m.version=? AND s.revoked=0 LIMIT 8")
        .bind(id).bind(version).fetch_all(pool).await.map_err(|_| "db_error")?.iter().map(|r| Ok(Source {
            kind:r.get("kind"),origin_session:r.get("origin_session"),message_id:r.get("message_id"),
            knowledge:parse(r.get("knowledge"))?,evidence:r.get("evidence") })).collect()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Save {
    pub expected_companion_id: Option<String>,
    pub id: Option<String>,
    pub expected_version: Option<i64>,
    pub scope: Scope,
    pub class: Class,
    pub body: String,
    pub pinned: bool,
    pub session_id: Option<String>,
    pub message_id: Option<String>,
    pub request_id: String,
    pub expected_revision: i64,
}

pub(crate) async fn check_revision(
    tx: &mut Transaction<'_, Sqlite>,
    revision: i64,
) -> Result<(), String> {
    let (actual,): (i64,) =
        sqlx::query_as("SELECT revision FROM companion_memory_state WHERE id=1")
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| "db_error")?;
    if actual != revision {
        return Err("memory_stale".into());
    }
    Ok(())
}

/// Reserve room without eviction. Counting all rows also bounds revocation metadata.
pub(super) async fn capacity(tx: &mut Transaction<'_, Sqlite>, pinned: bool) -> Result<(), String> {
    let (live,pins,markers): (i64,i64,i64) = sqlx::query_as("SELECT COALESCE(SUM(deleted=0),0),COALESCE(SUM(pinned),0),(SELECT COUNT(*) FROM companion_memories)+(SELECT COUNT(*) FROM companion_memory_jobs) FROM companion_memories")
        .fetch_one(&mut **tx).await.map_err(|_| "db_error")?;
    if live >= MAX_ITEMS || (pinned && pins >= MAX_PINS) || markers >= MAX_MARKERS {
        return Err("memory_capacity".into());
    }
    Ok(())
}

pub(super) async fn bump(tx: &mut Transaction<'_, Sqlite>) -> Result<i64, String> {
    sqlx::query("UPDATE companion_memory_state SET revision=revision+1 WHERE id=1")
        .execute(&mut **tx)
        .await
        .map_err(|_| "db_error")?;
    let (revision,): (i64,) =
        sqlx::query_as("SELECT revision FROM companion_memory_state WHERE id=1")
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| "db_error")?;
    Ok(revision)
}

/// Conservative invalidation: all old companion exchanges and cached memos are
/// excluded. Text remains visible only in the independent local transcript.
pub(super) async fn invalidate(
    tx: &mut Transaction<'_, Sqlite>,
    revision: i64,
) -> Result<(), String> {
    sqlx::query("INSERT INTO companion_replay_barriers(thread_id,excluded_through,revision) SELECT s.thread_id,COALESCE(MAX(m.position),-1),? FROM companion_sessions s LEFT JOIN chat_messages m ON m.thread_id=s.thread_id GROUP BY s.thread_id ON CONFLICT(thread_id) DO UPDATE SET excluded_through=MAX(excluded_through,excluded.excluded_through),revision=excluded.revision")
        .bind(revision).execute(&mut **tx).await.map_err(|_| "db_error")?;
    sqlx::query("UPDATE companion_memory_sources SET evidence='',revoked=1 WHERE memory_id IN (SELECT id FROM companion_memories WHERE class='summary' AND deleted=0)")
        .execute(&mut **tx).await.map_err(|_| "db_error")?;
    sqlx::query("UPDATE companion_memories SET body='',deleted=1,pinned=0,version=version+1,revoked_revision=?,updated_at=? WHERE class='summary' AND deleted=0")
        .bind(revision).bind(now()).execute(&mut **tx).await.map_err(|_| "db_error")?;
    cancel_jobs(tx).await
}

pub(super) async fn cancel_jobs(tx: &mut Transaction<'_, Sqlite>) -> Result<(), String> {
    sqlx::query("UPDATE companion_memory_jobs SET state='cancelled',input='',output='',prompt='',updated_at=?,error=NULL WHERE state IN ('pending','running','paused','review')")
        .bind(now()).execute(&mut **tx).await.map_err(|_| "db_error")?;
    Ok(())
}

pub(super) async fn insert(
    tx: &mut Transaction<'_, Sqlite>,
    m: &Memory,
    key: &str,
) -> Result<(), String> {
    if !m.valid() {
        return Err("invalid_memory".into());
    }
    capacity(tx, m.pinned).await?;
    sqlx::query("INSERT INTO companion_memories(id,version,class,scope,companion_id,game_id,playthrough_id,knowledge,body,evidence,pinned,origin_session,idempotency_key,created_at,updated_at,expires_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(&m.id).bind(m.version).bind(tag(&m.class)?).bind(tag(&m.scope)?).bind(&m.companion_id)
        .bind(&m.game_id).bind(&m.playthrough_id).bind(tag(&m.knowledge)?).bind(&m.body).bind(tag(&m.evidence)?)
        .bind(m.pinned).bind(&m.origin_session).bind(key).bind(m.created_at).bind(m.updated_at).bind(m.expires_at)
        .execute(&mut **tx).await.map_err(|_| "db_error")?;
    Ok(())
}

pub(super) async fn add_source(
    tx: &mut Transaction<'_, Sqlite>,
    m: &Memory,
    source: &Source,
    thread: Option<&str>,
    range: Option<(i64, i64)>,
) -> Result<(), String> {
    if source.evidence.len() > MAX_EVIDENCE {
        return Err("memory_evidence_limit".into());
    }
    let (count,total): (i64,i64) = sqlx::query_as("SELECT COALESCE(SUM(s.memory_id=?),0),COALESCE(SUM(length(CAST(s.evidence AS BLOB))),0) FROM companion_memory_sources s JOIN companion_memories m ON m.id=s.memory_id AND m.deleted=0 WHERE s.revoked=0")
        .bind(&m.id).fetch_one(&mut **tx).await.map_err(|_| "db_error")?;
    if count >= 8
        || total + i64::try_from(source.evidence.len()).unwrap_or(i64::MAX) > MAX_EVIDENCE_TOTAL
    {
        return Err("memory_capacity".into());
    }
    sqlx::query("INSERT INTO companion_memory_sources(id,memory_id,memory_version,kind,origin_session,thread_id,message_id,from_position,through_position,knowledge,evidence) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
        .bind(uuid::Uuid::new_v4().to_string()).bind(&m.id).bind(m.version).bind(&source.kind)
        .bind(&source.origin_session).bind(thread).bind(&source.message_id).bind(range.map(|r| r.0)).bind(range.map(|r| r.1))
        .bind(tag(&source.knowledge)?).bind(&source.evidence).execute(&mut **tx).await.map_err(|_| "db_error")?;
    Ok(())
}

pub(super) async fn save(
    pool: &SqlitePool,
    request: &Save,
    session: Option<&Session>,
) -> Result<Memory, String> {
    uuid::Uuid::parse_str(&request.request_id).map_err(|_| "invalid_memory")?;
    if !body_valid(&request.body) {
        return Err("invalid_memory".into());
    }
    if request.id.is_none() && request.class == Class::Summary {
        return Err("invalid_memory".into());
    }
    let mut tx = pool.begin().await.map_err(|_| "db_error")?;
    // An acknowledged request can be replayed, but a revoked idempotency marker
    // can never be used to restore content.
    if request.id.is_none() {
        if let Some(row) =
            sqlx::query("SELECT * FROM companion_memories WHERE idempotency_key=? AND deleted=0")
                .bind(&request.request_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| "db_error")?
        {
            let saved = decode(&row)?;
            if saved.body != request.body.trim()
                || saved.scope != request.scope
                || saved.class != request.class
                || saved.pinned != request.pinned
                || (saved.scope == Scope::Companion
                    && saved.companion_id != request.expected_companion_id)
                || (saved.scope == Scope::Reading && saved.origin_session != request.session_id)
            {
                return Err("memory_stale".into());
            }
            return Ok(saved);
        }
        let (used,): (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM companion_memories WHERE idempotency_key=?)",
        )
        .bind(&request.request_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "db_error")?;
        if used {
            return Err("memory_stale".into());
        }
    }
    check_revision(&mut tx, request.expected_revision).await?;
    let mut m = new_memory(request, session)?;
    let mut source = Source {
        kind: "reader".into(),
        origin_session: m.origin_session.clone(),
        message_id: None,
        knowledge: m.knowledge,
        evidence: m.body.clone(),
    };
    if let Some(message) = &request.message_id {
        if request.scope != Scope::Reading || request.id.is_some() {
            return Err("invalid_memory".into());
        }
        let s = session.ok_or("companion_missing")?;
        let row = sqlx::query("SELECT m.* FROM chat_messages m JOIN companion_sessions s ON s.thread_id=m.thread_id LEFT JOIN companion_replay_barriers b ON b.thread_id=m.thread_id WHERE m.id=? AND s.id=? AND m.status='complete' AND m.position>COALESCE(b.excluded_through,-1)")
            .bind(message).bind(&s.id).fetch_optional(&mut *tx).await.map_err(|_| "db_error")?.ok_or("memory_source_unavailable")?;
        source.kind = row.get("role");
        source.message_id = Some(message.clone());
        source.evidence = row.get("content");
        m.evidence = if source.kind == "assistant" {
            Evidence::Inference
        } else {
            Evidence::Passage
        };
    }
    if request.id.is_some() {
        let (updated, evidence) = edit_memory(&mut tx, request, m).await?;
        m = updated;
        if let Some(evidence) = evidence {
            source = evidence;
        } else {
            tx.commit().await.map_err(|_| "db_error")?;
            return Ok(m);
        }
    } else {
        insert(&mut tx, &m, &request.request_id).await?;
        bump(&mut tx).await?;
        cancel_jobs(&mut tx).await?;
    }
    add_source(
        &mut tx,
        &m,
        &source,
        session
            .filter(|_| m.scope == Scope::Reading)
            .map(|s| s.thread_id.as_str()),
        None,
    )
    .await?;
    tx.commit().await.map_err(|_| "db_error")?;
    Ok(m)
}

fn new_memory(request: &Save, session: Option<&Session>) -> Result<Memory, String> {
    let context = session.map(|s| Context::new(&s.options, &s.playthrough_id));
    let mut m = Memory {
        id: uuid::Uuid::new_v4().to_string(),
        version: 1,
        class: request.class,
        scope: request.scope,
        companion_id: None,
        game_id: None,
        playthrough_id: None,
        knowledge: Knowledge::Metadata,
        body: request.body.trim().into(),
        evidence: Evidence::Reader,
        pinned: request.pinned,
        origin_session: None,
        created_at: now(),
        updated_at: now(),
        expires_at: None,
    };
    match request.scope {
        _ if request.id.is_some() => (),
        Scope::Shared => (),
        Scope::Companion => {
            if request.expected_companion_id.as_deref()
                != context.as_ref().map(|c| c.companion_id.as_str())
            {
                return Err("memory_stale".into());
            }
            m.companion_id = Some(
                context
                    .as_ref()
                    .ok_or("companion_missing")?
                    .companion_id
                    .clone(),
            );
        }
        Scope::Reading => {
            let c = context.as_ref().ok_or("companion_missing")?;
            m.game_id = Some(c.game_id.clone());
            m.playthrough_id = Some(c.playthrough_id.clone());
            m.knowledge = c.knowledge;
            m.origin_session = session.map(|s| s.id.clone());
        }
    }
    Ok(m)
}

async fn edit_memory(
    tx: &mut Transaction<'_, Sqlite>,
    request: &Save,
    mut m: Memory,
) -> Result<(Memory, Option<Source>), String> {
    let id = request.id.as_deref().ok_or("invalid_memory")?;
    let row =
        sqlx::query("SELECT * FROM companion_memories WHERE id=? AND deleted=0 AND version=?")
            .bind(id)
            .bind(request.expected_version)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| "db_error")?
            .ok_or("memory_stale")?;
    let old = decode(&row)?;
    let corrected = m.body != old.body;
    // An edit changes text/pin, never widens its original provenance/scope.
    m = Memory {
        body: m.body,
        pinned: m.pinned,
        version: old.version + 1,
        updated_at: now(),
        ..old
    };
    if !m.valid() {
        return Err("invalid_memory".into());
    }
    let (pins,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM companion_memories WHERE deleted=0 AND pinned=1 AND id<>?",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| "db_error")?;
    if m.pinned && pins >= MAX_PINS {
        return Err("memory_capacity".into());
    }
    let revision = bump(tx).await?;
    if corrected {
        invalidate(tx, revision).await?;
    } else {
        cancel_jobs(tx).await?;
    }
    // Keep the corrected recap itself; dependent recaps were invalidated above.
    if corrected {
        sqlx::query("DELETE FROM companion_memory_sources WHERE memory_id=?")
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(|_| "db_error")?;
    } else {
        sqlx::query(
            "UPDATE companion_memory_sources SET memory_version=? WHERE memory_id=? AND revoked=0",
        )
        .bind(m.version)
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|_| "db_error")?;
    }
    sqlx::query("UPDATE companion_memories SET body=?,version=?,pinned=?,updated_at=?,deleted=0,revoked_revision=NULL WHERE id=?")
            .bind(&m.body).bind(m.version).bind(m.pinned).bind(m.updated_at).bind(id).execute(&mut **tx).await.map_err(|_| "db_error")?;
    let source = Source {
        kind: "reader".into(),
        origin_session: m.origin_session.clone(),
        message_id: None,
        knowledge: m.knowledge,
        evidence: m.body.clone(),
    };
    if !corrected {
        return Ok((m, None));
    }

    Ok((m, Some(source)))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Forget {
    Item { id: String, version: i64 },
    Session { id: String },
    Reading { id: String },
    Companion { id: String },
    Everything,
}

pub(super) async fn forget_tx(
    tx: &mut Transaction<'_, Sqlite>,
    target: &Forget,
) -> Result<(), String> {
    let (field, value, version) = match target {
        Forget::Item { id, version } => ("id", Some(id.clone()), Some(*version)),
        Forget::Session { id } => ("origin_session", Some(id.clone()), None),
        Forget::Companion { id } => ("companion_id", Some(id.clone()), None),
        Forget::Reading { id } => {
            let (playthrough,): (String,) =
                sqlx::query_as("SELECT playthrough_id FROM companion_sessions WHERE id=?")
                    .bind(id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|_| "db_error")?
                    .ok_or("companion_missing")?;
            ("playthrough_id", Some(playthrough), None)
        }
        Forget::Everything => ("id", None, None),
    };
    if version.is_some() {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM companion_memories WHERE deleted=0 AND id=? AND version=?",
        )
        .bind(&value)
        .bind(version)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| "db_error")?;
        if count != 1 {
            return Err("memory_stale".into());
        }
    }
    let revision = bump(tx).await?;
    sqlx::query("UPDATE companion_memory_sources SET evidence='',revoked=1 WHERE memory_id IN (SELECT id FROM companion_memories WHERE deleted=0 AND (? IS NULL OR CASE ? WHEN 'id' THEN id WHEN 'origin_session' THEN origin_session WHEN 'companion_id' THEN companion_id WHEN 'playthrough_id' THEN playthrough_id END=?) AND (? IS NULL OR version=?))")
        .bind(&value).bind(field).bind(&value).bind(version).bind(version).execute(&mut **tx).await.map_err(|_| "db_error")?;
    sqlx::query("UPDATE companion_memories SET body='',deleted=1,pinned=0,version=version+1,revoked_revision=?,updated_at=? WHERE deleted=0 AND (? IS NULL OR CASE ? WHEN 'id' THEN id WHEN 'origin_session' THEN origin_session WHEN 'companion_id' THEN companion_id WHEN 'playthrough_id' THEN playthrough_id END=?) AND (? IS NULL OR version=?)")
        .bind(revision).bind(now()).bind(&value).bind(field).bind(&value).bind(version).bind(version).execute(&mut **tx).await.map_err(|_| "db_error")?;
    invalidate(tx, revision).await
}

pub async fn recover(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query("UPDATE companion_memory_jobs SET state='paused',error='interrupted',updated_at=? WHERE state IN ('pending','running')")
        .bind(now()).execute(pool).await.map_err(|_| "db_error")?;
    expire(pool).await
}

pub async fn expire(pool: &SqlitePool) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|_| "db_error")?;
    sqlx::query("UPDATE companion_memory_jobs SET state='expired',input='',output='',prompt='',error=NULL WHERE expires_at<=? AND state IN ('pending','running','paused','review')")
        .bind(now()).execute(&mut *tx).await.map_err(|_| "db_error")?;
    sqlx::query("UPDATE companion_memory_sources SET evidence='',revoked=1 WHERE memory_id IN (SELECT id FROM companion_memories WHERE deleted=0 AND pinned=0 AND expires_at<=?)")
        .bind(now()).execute(&mut *tx).await.map_err(|_| "db_error")?;
    sqlx::query("UPDATE companion_memories SET body='',deleted=1,pinned=0,version=version+1 WHERE deleted=0 AND pinned=0 AND expires_at<=?")
        .bind(now()).execute(&mut *tx).await.map_err(|_| "db_error")?;
    tx.commit().await.map_err(|_| "db_error".into())
}
