//! Allowlisted read-only tools. Never accepts SQL, paths, URLs or credentials from a model.
use super::Source;
use crate::vndb::dto::VnQueryResult;
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use std::fmt::Write;
use vn_core::chat::{ChatOptions, Spoilers, ToolArgs, public_text};

pub struct ToolContext<'a> {
    pub pool: &'a SqlitePool,
    pub vndb: &'a crate::vndb::VndbClient,
    pub client: &'a reqwest::Client,
    pub options: &'a ChatOptions,
    /// Live text-hook ring, for `recent_game_text`. The ring only exists on
    /// this device; there is no game attribution beyond what the player knows.
    pub hook: &'a std::sync::Arc<crate::tracking::hook::HookHost>,
}

impl std::fmt::Debug for ToolContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext").finish_non_exhaustive()
    }
}

pub async fn run(
    ctx: &ToolContext<'_>,
    name: &str,
    raw: &str,
    sources: &mut Vec<Source>,
) -> Result<Value, String> {
    let args = ToolArgs::parse(name, raw, ctx.options)?;
    if let Some(id) = &ctx.options.game_id {
        let live: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM games WHERE id=? AND deleted=0")
            .bind(id)
            .fetch_one(ctx.pool)
            .await
            .map_err(|_| "db_error")?;
        if live.0 != 1 {
            return Err("context_game_missing".into());
        }
    }
    let data = match name {
        "search_library" => search_library(ctx, &args).await?,
        "get_game" => game(ctx, args.game_id.as_deref().ok_or("game_id_required")?).await?,
        "library_stats" => stats(ctx).await?,
        "play_history" => history(ctx, &args).await?,
        "taste_profile" => taste(ctx).await?,
        "recent_game_text" => recent_text(ctx).await?,
        "script_digest" => script_digest(ctx).await?,
        // External evidence carries its own URL, so it is cited by URL, not by a
        // library citation id that would point at the user's notes instead.
        "search_vndb" => return vndb(ctx, &args.query, sources).await,
        "search_web" => return web(ctx, &args.query, sources).await,
        _ => return Err("tool_not_allowed".into()),
    };
    let id = args.game_id.as_ref().or(ctx.options.game_id.as_ref());
    let source = add_source(sources, name, None, id.cloned());
    Ok(json!({"source": source, "data":data}))
}

fn add_source(
    sources: &mut Vec<Source>,
    title: &str,
    url: Option<String>,
    game_id: Option<String>,
) -> String {
    let id = format!("S{}", sources.len() + 1);
    sources.push(Source {
        id: id.clone(),
        title: title.chars().take(240).collect(),
        url,
        game_id,
    });
    id
}

/// One shared row source for the card-shaped tools. Totals are aggregated in
/// the same statement, so a page never pays for a per-game subquery.
const GAMES_FROM: &str = "FROM games g     LEFT JOIN (SELECT p.game_id,SUM(p.counted_ms) AS active,SUM(p.afk_ms) AS afk     FROM play_sessions p JOIN games parent ON parent.id=p.game_id AND parent.deleted=0     WHERE p.deleted=0 GROUP BY p.game_id) s ON s.game_id=g.id     LEFT JOIN vndb_details d ON d.vn_id=g.vndb_id WHERE g.deleted=0";

const GAMES: &str = "SELECT g.id,g.title,g.title_original,g.vndb_id,g.status,g.rating,g.manual_playtime_ms,     COALESCE(s.active,0)+g.manual_playtime_ms AS total_ms,COALESCE(s.afk,0) AS afk_ms,d.tags_json ";

async fn game_rows(ctx: &ToolContext<'_>) -> Result<Vec<sqlx::sqlite::SqliteRow>, String> {
    let sql = format!("{GAMES}{GAMES_FROM} AND (? IS NULL OR g.id=?)");
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(&ctx.options.game_id)
        .bind(&ctx.options.game_id)
        .fetch_all(ctx.pool)
        .await
        .map_err(|_| "db_error".into())
}

fn safe_tags(raw: Option<&str>, all: bool) -> Vec<Value> {
    let tags: Vec<Value> = raw
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or_default();
    tags.into_iter()
        .filter(|tag| all || tag["spoiler"].as_f64() == Some(0.0))
        .filter_map(|tag| {
            tag["name"]
                .as_str()
                .map(|name| json!({"name":name,"rating":tag["rating"]}))
        })
        .take(40)
        .collect()
}

fn card(row: &sqlx::sqlite::SqliteRow, all: bool) -> Value {
    json!({"game_id":row.get::<String,_>("id"),"title":row.get::<String,_>("title"),"original_title":row.get::<Option<String>,_>("title_original"),
        "vndb_id":row.get::<Option<String>,_>("vndb_id"),"status":row.get::<String,_>("status"),
        "rating":row.get::<Option<i64>,_>("rating").and_then(|n|u8::try_from(n).ok()).map(|n| f64::from(n) / 10.0),
        "playtime_ms":row.get::<i64,_>("total_ms"),"afk_ms":row.get::<i64,_>("afk_ms"),
        "tags":safe_tags(row.get("tags_json"),all)})
}

/// Page size for `search_library`.
const PAGE: i64 = 20;

/// Find owned games by title, original title, VNDB id or tag name.
///
/// Filtering, ordering, paging and the tie count all run in `SQLite`. The previous
/// version loaded every live game, built a JSON card for each (parsing its tag
/// blob), then filtered and sorted those values in Rust: on a large library that
/// decoded thousands of tag arrays to answer a question about twenty titles.
async fn search_library(ctx: &ToolContext<'_>, args: &ToolArgs) -> Result<Value, String> {
    let query = args.query.trim().to_lowercase();
    let offset = i64::from(args.offset.unwrap_or(0));
    // sqlx binds in call order and SQLite reads `?N` by number, so the numbering
    // has to stay dense: a skipped `?2` makes an integer scope collide with a NULL
    // literal and every row fails as a type mismatch. Scope is always `?1`, the
    // optional filters take the next numbers, and paging stays inline integers so
    // it cannot shift them.
    let mut where_clause = String::from(" AND (?1 IS NULL OR g.id=?1)");
    let mut next = 2;
    if args.status.is_some() {
        let _ = write!(where_clause, " AND g.status=?{next}");
        next += 1;
    }
    let fuzzy = !query.is_empty();
    if fuzzy {
        let _ = write!(
            where_clause,
            " AND (INSTR(LOWER(g.title),?{next})>0              OR INSTR(LOWER(COALESCE(g.title_original,'')),?{next})>0              OR INSTR(LOWER(COALESCE(g.vndb_id,'')),?{next})>0              OR INSTR(LOWER(COALESCE(d.tags_json,'')),?{next})>0)"
        );
    }
    let order = match args.sort.as_deref() {
        Some("rating") => " ORDER BY g.rating IS NULL, g.rating DESC, g.title",
        Some("playtime") => " ORDER BY COALESCE(s.active,0)+g.manual_playtime_ms DESC, g.title",
        _ => " ORDER BY g.title",
    };
    let sql = format!("{GAMES}{GAMES_FROM}{where_clause}{order} LIMIT {PAGE} OFFSET {offset}");
    let count_sql = format!("SELECT COUNT(*) {GAMES_FROM}{where_clause}");

    let mut page_query = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(&ctx.options.game_id);
    let mut count_query =
        sqlx::query_as::<_, (i64,)>(sqlx::AssertSqlSafe(count_sql)).bind(&ctx.options.game_id);
    if let Some(status) = &args.status {
        page_query = page_query.bind(status);
        count_query = count_query.bind(status);
    }
    if fuzzy {
        page_query = page_query.bind(&query);
        count_query = count_query.bind(&query);
    }
    let rows = page_query
        .fetch_all(ctx.pool)
        .await
        .map_err(|_| "db_error")?;
    let total = count_query
        .fetch_one(ctx.pool)
        .await
        .map_err(|_| "db_error")?
        .0;
    let all = ctx.options.spoilers == Spoilers::All;
    let sorting = args.sort.as_deref();
    // A sort value shared by many games is not a ranking. Reporting the tie size
    // stops the model presenting seven equal ratings as a meaningful top-seven
    // list, and lets it say "seven are tied" instead. Only for value sorts.
    let tie_size = match sorting {
        // Ratings are decivotes, so integers preserve equality exactly.
        Some("rating") => leading_tie(rows.iter().map(|row| row.get::<Option<i64>, _>("rating"))),
        Some("playtime") => leading_tie(rows.iter().map(|row| Some(row.get::<i64, _>("total_ms")))),
        _ => 0,
    };
    Ok(json!({
        "total": total,
        "offset": offset,
        "has_more": total > offset + PAGE,
        "ranked_by": sorting,
        // Null unless the sort key produces ties at the top.
        "tied_at_leading_value": if tie_size > 1 { json!(tie_size) } else { Value::Null },
        "games": rows.iter().map(|row| card(row, all)).collect::<Vec<_>>(),
    }))
}

/// How many of `values` share the first non-null value, in order. Zero when the
/// page is empty or the first row has no value to tie on.
fn leading_tie(values: impl Iterator<Item = Option<i64>>) -> usize {
    let mut values = values;
    let Some(Some(leading)) = values.next() else {
        return 0;
    };
    values.filter(|value| *value == Some(leading)).count() + 1
}

async fn game(ctx: &ToolContext<'_>, id: &str) -> Result<Value, String> {
    let rows = game_rows(ctx).await?;
    let row = rows
        .iter()
        .find(|row| row.get::<String, _>("id") == id)
        .ok_or("game_missing")?;
    let mut output = card(row, ctx.options.spoilers == Spoilers::All);
    let detail = sqlx::query("SELECT g.notes,d.description,d.fetched_at FROM games g LEFT JOIN vndb_details d ON d.vn_id=g.vndb_id WHERE g.id=? AND g.deleted=0")
        .bind(id).fetch_one(ctx.pool).await.map_err(|_| "db_error")?;
    let desc: Option<String> = detail.get("description");
    output["synopsis"] = json!(desc.map(|text| if ctx.options.spoilers == Spoilers::All {
        text.chars().take(8000).collect()
    } else {
        public_text(&text)
    }));
    output["metadata_fetched_at"] = json!(detail.get::<Option<i64>, _>("fetched_at"));
    output["story_coverage"] =
        json!("Cached VNDB synopsis only; not a full script or verified route summary.");
    if ctx.options.spoilers == Spoilers::All {
        output["user_notes"] = json!(
            detail
                .get::<Option<String>, _>("notes")
                .map(|s| s.chars().take(8000).collect::<String>())
        );
        let chapters = sqlx::query("SELECT c.name,c.reached FROM chapters c JOIN games g ON g.id=c.game_id AND g.deleted=0 WHERE c.game_id=? AND c.deleted=0 ORDER BY c.sort_index LIMIT 100")
            .bind(id).fetch_all(ctx.pool).await.map_err(|_| "db_error")?;
        output["chapters"] = json!(chapters.iter().map(|r| json!({"name":r.get::<String,_>("name"),"reached":r.get::<bool,_>("reached")})).collect::<Vec<_>>());
        let endings = sqlx::query("SELECT e.name,e.route,e.achieved FROM endings e JOIN games g ON g.id=e.game_id AND g.deleted=0 WHERE e.game_id=? AND e.deleted=0 LIMIT 100")
            .bind(id).fetch_all(ctx.pool).await.map_err(|_| "db_error")?;
        output["endings"] = json!(endings.iter().map(|r| json!({"name":r.get::<String,_>("name"),"route":r.get::<Option<String>,_>("route"),"achieved":r.get::<bool,_>("achieved")})).collect::<Vec<_>>());
    } else {
        output["withheld"] = json!(
            "Private notes and chapter/ending names withheld because they have no reliable spoiler boundary. Ask the user to paste a permitted passage."
        );
    }
    Ok(output)
}

async fn stats(ctx: &ToolContext<'_>) -> Result<Value, String> {
    let rows = game_rows(ctx).await?;
    let mut statuses = std::collections::BTreeMap::<String, usize>::new();
    let mut active = 0_i64;
    let mut afk = 0_i64;
    let mut manual = 0_i64;
    for row in &rows {
        *statuses.entry(row.get("status")).or_default() += 1;
        active = active.saturating_add(row.get("total_ms"));
        afk = afk.saturating_add(row.get("afk_ms"));
        manual = manual.saturating_add(row.get("manual_playtime_ms"));
    }
    Ok(
        json!({"games":rows.len(),"by_status":statuses,"playtime_ms":active,"afk_ms":afk,"manual_adjustment_ms":manual}),
    )
}

async fn history(ctx: &ToolContext<'_>, args: &ToolArgs) -> Result<Value, String> {
    let id = args.game_id.as_ref().or(ctx.options.game_id.as_ref());
    let totals: (i64, i64, i64) = sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(p.counted_ms),0),COALESCE(SUM(p.afk_ms),0) FROM play_sessions p JOIN games g ON g.id=p.game_id AND g.deleted=0 WHERE p.deleted=0 AND (? IS NULL OR p.game_id=?) AND p.started_at>=? AND p.started_at<?")
    .bind(id)
    .bind(id)
    .bind(args.from_ms.unwrap_or(0))
    .bind(args.to_ms.unwrap_or(i64::MAX))
    .fetch_one(ctx.pool)
    .await
    .map_err(|_| "db_error")?;
    let rows = sqlx::query("SELECT g.title,p.game_id,p.started_at,p.ended_at,p.counted_ms,p.afk_ms,p.source FROM play_sessions p JOIN games g ON g.id=p.game_id AND g.deleted=0 WHERE p.deleted=0 AND (? IS NULL OR p.game_id=?) AND p.started_at>=? AND p.started_at<? ORDER BY p.started_at DESC,p.id LIMIT 20 OFFSET ?")
        .bind(id).bind(id).bind(args.from_ms.unwrap_or(0)).bind(args.to_ms.unwrap_or(i64::MAX)).bind(args.offset.unwrap_or(0))
        .fetch_all(ctx.pool).await.map_err(|_| "db_error")?;
    Ok(
        json!({"total_sessions":totals.0,"counted_ms":totals.1,"afk_ms":totals.2,"manual_adjustments_included":false,"sessions":rows.iter().map(|r| json!({"title":r.get::<String,_>("title"),"game_id":r.get::<String,_>("game_id"),"started_at":r.get::<i64,_>("started_at"),"ended_at":r.get::<i64,_>("ended_at"),"counted_ms":r.get::<i64,_>("counted_ms"),"afk_ms":r.get::<i64,_>("afk_ms"),"source":r.get::<String,_>("source")})).collect::<Vec<_>>()}),
    )
}

async fn taste(ctx: &ToolContext<'_>) -> Result<Value, String> {
    let rows = game_rows(ctx).await?;
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for row in &rows {
        for tag in safe_tags(row.get("tags_json"), false) {
            if let Some(name) = tag["name"].as_str() {
                *counts.entry(name.to_owned()).or_default() += 1;
            }
        }
    }
    let mut themes: Vec<_> = counts.into_iter().collect();
    themes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(
        json!({"games":rows.len(),"themes":themes.into_iter().take(30).map(|(name,count)| json!({"name":name,"games":count})).collect::<Vec<_>>(),"limitation":"Tag frequency describes ownership, not necessarily enjoyment. Missing cached tags are unknown."}),
    )
}

/// Live text-hook capture. These lines are whatever the hooked engine just
/// displayed, so they are the freshest evidence the companion has — but they
/// carry no game attribution and no history beyond the ring buffer.
async fn recent_text(ctx: &ToolContext<'_>) -> Result<Value, String> {
    let lines = ctx.hook.lines().await;
    let recent: Vec<_> = lines
        .iter()
        .rev()
        .take(30)
        .rev()
        .map(|line| json!({"text": line.text, "thread": line.thread, "at_ms": line.at}))
        .collect();
    Ok(
        json!({"lines":recent,"coverage":"Live text-hook capture, oldest first, at most the last 30 lines. Present only while a hook is attached to a running game; not attributed to a specific library game, and lines may repeat engine chrome. This is what the player has just read."}),
    )
}

/// The compacted digest of the scoped game's imported script. The digest covers
/// the whole story from the first line, so it has no spoiler boundary at all —
/// the same rule that withholds private notes in `get_game`.
async fn script_digest(ctx: &ToolContext<'_>) -> Result<Value, String> {
    if ctx.options.spoilers != Spoilers::All {
        return Ok(
            json!({"withheld":"The imported script digest covers the whole story and has no reliable spoiler boundary. Ask the user to enable full spoilers, or ask them to paste a permitted passage."}),
        );
    }
    let id = ctx
        .options
        .game_id
        .as_deref()
        .ok_or("game_scope_required")?;
    match crate::script::digest_for_game(ctx.pool, id).await? {
        None => Ok(
            json!({"digest":Value::Null,"coverage":"No script has been imported for this game."}),
        ),
        Some(state) if state.digest.is_none() => Ok(
            json!({"digest":Value::Null,"done_chunks":state.done_chunks,"chunk_count":state.chunk_count,"coverage":"A script is imported but compaction has not finished, so there is no digest to cite yet."}),
        ),
        Some(state) => {
            let coverage = if state.outdated.is_some() {
                "Compacted digest of the newest imported script for this game, built by an older summarizer. Compaction is lossy; treat wording as unverified and say when the digest is silent."
            } else {
                "Compacted story index of the newest imported script for this game: characters, chronology, player choices and branches, and explicitly unresolved questions. It is lossy, not the script itself; treat exact wording as unverified and say when the index is silent."
            };
            Ok(json!({"digest":state.digest,"outdated":state.outdated,"coverage":coverage}))
        }
    }
}

async fn vndb(
    ctx: &ToolContext<'_>,
    query: &str,
    sources: &mut Vec<Source>,
) -> Result<Value, String> {
    use crate::vndb::cache;
    let key = format!("chat:vn:{}", query.trim().to_lowercase());
    let now = vn_core::clock::Timestamp::now().map_err(|_| "clock_error")?;
    let cached = cache::get_fresh(ctx.pool, &key, now)
        .await
        .map_err(|_| "db_error")?;
    // A failed live call falls back to expired cache and is marked stale, so the
    // model can say the metadata is old instead of claiming VNDB is unreachable.
    let (result, stale): (VnQueryResult, bool) = if let Some(raw) = cached {
        (
            serde_json::from_str(&raw).map_err(|_| "invalid_cache")?,
            false,
        )
    } else if let Some(result) = vndb_live(ctx, &key, query, now).await {
        (result, false)
    } else {
        let raw = cache::get_any(ctx.pool, &key)
            .await
            .map_err(|_| "db_error")?
            .ok_or("vndb_unavailable")?;
        (
            serde_json::from_str(&raw).map_err(|_| "invalid_cache")?,
            true,
        )
    };
    let games: Vec<_>=result.results.iter().map(|vn| {
        let source=add_source(sources,vn.title.as_deref().unwrap_or(&vn.id),Some(format!("https://vndb.org/{}",vn.id)),None);
        let text=vn.description.as_deref().unwrap_or("");
        json!({"source":source,"id":vn.id,"title":vn.title,"synopsis":if ctx.options.spoilers==Spoilers::All {text.chars().take(8000).collect::<String>()} else { public_text(text) },"released":vn.released,"tags":vn.tags.iter().filter(|t| ctx.options.spoilers==Spoilers::All || t.spoiler==Some(0)).map(|t| &t.name).collect::<Vec<_>>()})
    }).collect();
    Ok(
        json!({"games":games,"stale_cache":stale,"more":result.more,"coverage":"VNDB metadata, not full story scripts"}),
    )
}

/// Live VNDB lookup that also refreshes the cache. Any failure is `None` so the
/// caller can fall back to expired cache.
async fn vndb_live(
    ctx: &ToolContext<'_>,
    key: &str,
    query: &str,
    now: vn_core::clock::Timestamp,
) -> Option<VnQueryResult> {
    use crate::vndb::{cache, dto};
    let result = ctx
        .vndb
        .search_vns(&dto::VnQuery::by_title(
            query,
            "id,title,alttitle,description,released,tags.name,tags.rating,tags.spoiler",
            1,
            5,
        ))
        .await
        .ok()?;
    let raw = serde_json::to_string(&result).ok()?;
    cache::put(ctx.pool, key, &raw, cache::SEARCH_TTL_MS, now)
        .await
        .ok()?;
    Some(result)
}

async fn web(
    ctx: &ToolContext<'_>,
    query: &str,
    sources: &mut Vec<Source>,
) -> Result<Value, String> {
    // Open-web snippets cannot be reliably assigned to a chapter boundary.
    if ctx.options.spoilers != Spoilers::All {
        return Err("web_requires_spoilers".into());
    }
    let key = tauri::async_runtime::spawn_blocking(crate::secrets::load_tavily_key)
        .await
        .map_err(|_| "key_unavailable")?
        .map_err(|_| "tavily_key_missing")?;
    let result =
        crate::ai::tavily::search(ctx.client, &key, query, 5, crate::ai::tavily::Depth::Basic)
            .await
            .map_err(|_| "web_unavailable")?;
    let hits: Vec<_> = result
        .results
        .into_iter()
        .filter(|r| {
            reqwest::Url::parse(&r.url).is_ok_and(|u| matches!(u.scheme(), "https" | "http"))
        })
        .map(|r| {
            let source = add_source(sources, &r.title, Some(r.url), None);
            json!({"source":source,"excerpt":r.content.chars().take(4000).collect::<String>()})
        })
        .collect();
    Ok(
        json!({"results":hits,"credits":result.credits,"coverage":"Public search excerpts; verify claims and separate facts from interpretation."}),
    )
}
