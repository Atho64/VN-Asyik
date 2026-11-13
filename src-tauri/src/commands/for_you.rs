//! "For you" commands: taste seeds from the library, candidates from VNDB.
//!
//! The query goes to the documented `POST /vn` endpoint with filters built
//! from the library's seeds. The filter shapes were verified against the live
//! API when this feature was built: `developer` and `staff` take nested id
//! queries, `tag` takes `[tagId, maxSpoiler, minRating]`, and the `staff`
//! filter matches voice actors too — which is what the Voices tab runs on.
//! Ranking is pure `vn_core::for_you` arithmetic over the returned entries;
//! nothing here invents a score the seeds cannot explain.
//!
//! Notes returned to the UI are stable codes (`seeds`, `source`), not
//! sentences: the frontend translates them, like the screenshot error codes.

use serde::{Deserialize, Serialize};
use tauri::State;
use vn_core::clock::Timestamp;
use vn_core::for_you::{self, Candidate, ForYouSeeds, Ranked, SeedSignal};
use vn_core::taste::{BOILERPLATE_BELOW, NamedEntity, StaffCredit, TagWeight};

use super::{CommandError, CommandResult};
use crate::state::AppState;
use crate::vndb::cache;
use crate::vndb::dto::{Vn, VnQuery};

/// How long a result page is reused before VNDB is asked again. Long enough
/// that flipping filters does not spend the rate limiter, short enough that a
/// library change (and therefore a seeds change) misses the cache anyway —
/// the cache key hashes the seeds.
const RESULT_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// How long the tag-breadth lookup is reused. A tag's catalogue breadth
/// drifts on the scale of months; a week keeps the cost at one request per
/// fresh seed set without pretending it never changes.
const BREADTH_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Bump when the ranking arithmetic or the seed semantics change, so cached
/// pages computed the old way cannot serve.
const RANKING_VERSION: &str = "rarity-v3";

/// Candidates per page. The field list stays under the "too much data" rule
/// at this size; verified live: a 50-result page with the full field list
/// came back 200 at ~590 KB.
const PAGE_RESULTS: u32 = 50;

/// Match `vn_core::taste::usable_tag` (non-spoiler tags >= 2.88) in the VNDB
/// predicate so remote pages are not padded with candidates local scoring rejects.
const TAG_MIN_RATING: f32 = BOILERPLATE_BELOW;
const TAG_MAX_SPOILER: u8 = 0;

/// Maximum non-saturated seed tags in one VNDB filter, and maximum seed tags
/// rendered in the profile strip.
const MAX_QUERY_TAGS: usize = 16;
const MAX_DISPLAY_TAGS: usize = 12;

/// Standout solo tag weight and rarity ceiling for Combined affinity queries.
const STANDOUT_TAG_WEIGHT: f32 = 3.75;
const RARE_TAG_BELOW: u64 = 3_000;

/// Top voice seeds checked for representation on each Taste page, with up to
/// four per-actor supplemental queries to prevent prolific VAs crowding others.
const VOICE_TOP_SEEDS: usize = 8;
const MAX_EXTRA_VOICE_QUERIES: usize = 4;
const EXTRA_VOICE_RESULTS: u32 = 15;

/// Minimal field list used when backfilling pre-0037 `staff_json`/`va_json`.
const CREDIT_BACKFILL_FIELDS: &str = "id,staff.eid,staff.id,staff.name,staff.original,staff.role,va.staff.id,va.staff.name,va.staff.original";

/// Which seed category drives the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForYouSource {
    /// Tags, developers and staff or-ed together, ranked by total overlap.
    Combined,
    /// Themes only.
    Tags,
    /// Developers only.
    Studios,
    /// Scenario writers and artists only.
    Staff,
    /// Voice actors only.
    Voices,
}

/// How to order results. `Taste` ranks locally from the seeds; the rest pass
/// through to VNDB's documented sort keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForYouSort {
    Taste,
    Rating,
    Votes,
    Released,
    Title,
}

/// One search request.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForYouRequest {
    /// Which seeds drive the query.
    pub source: ForYouSource,
    /// VNDB length bucket 1..5.
    pub length: Option<u8>,
    /// Bayesian rating floor, 10..100.
    pub rating_min: Option<u8>,
    /// Vote-count floor.
    pub votes_min: Option<u32>,
    /// Release date floor, `YYYY-MM-DD`.
    pub released_after: Option<String>,
    /// Language code, e.g. `en`.
    pub language: Option<String>,
    /// Order; `taste` when omitted.
    pub sort: Option<ForYouSort>,
    /// 1-based page.
    pub page: Option<u32>,
}

/// One seed signal as the panel shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalView {
    /// Stable VNDB id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Distinct games carrying it.
    pub count: usize,
}

/// The library's seeds, for the panel header and chips.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForYouProfileView {
    /// Distinct works that contributed at least one signal.
    pub considered: usize,
    /// Whether the library is large enough to query VNDB.
    pub ready: bool,
    pub tags: Vec<SignalView>,
    pub developers: Vec<SignalView>,
    pub staff: Vec<SignalView>,
    pub voices: Vec<SignalView>,
    /// Strongest seed games, titles only.
    pub top_games: Vec<String>,
}

/// One recommendation card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForYouCard {
    pub vn_id: String,
    pub title: String,
    pub alttitle: Option<String>,
    pub released: Option<String>,
    pub image_url: Option<String>,
    /// Cover sexual rating in hundredths, the same units `SearchHit` uses.
    pub sexual: Option<i64>,
    /// Cover violence rating in hundredths.
    pub violence: Option<i64>,
    /// Coarse length bucket 1..5.
    pub length: Option<u8>,
    pub length_minutes: Option<i64>,
    pub rating: Option<f32>,
    pub votecount: Option<u32>,
    /// 0 finished, 1 in development, 2 cancelled.
    pub devstatus: Option<u8>,
    /// Developer names, as VNDB lists them.
    pub developers: Vec<String>,
    /// Summed match weight from the ranking.
    pub score: f32,
    /// Seed game title for the "Because you liked X" line.
    pub because: Option<String>,
    /// Matched theme names.
    pub themes: Vec<String>,
    /// Matched studio and staff names.
    pub creators: Vec<String>,
    /// Matched voice actors.
    pub voices: Vec<String>,
}

/// One result page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForYouPage {
    pub items: Vec<ForYouCard>,
    /// Whether another page exists.
    pub more: bool,
    /// True when VNDB could not be reached and saved results were used.
    pub from_cache: bool,
    /// Stable code explaining an empty page: `seeds` (library too small) or
    /// `source` (that category has no seeds yet). Translated by the UI.
    pub note: Option<String>,
}

/// The library's seeds, as the panel header shows them.
///
/// # Errors
/// Database failures.
#[tauri::command]
pub async fn for_you_profile(state: State<'_, AppState>) -> CommandResult<ForYouProfileView> {
    let pool = state.db().pool();
    let games = crate::db::queries::for_you::seeded_games(pool).await?;
    let seeds = for_you::for_you_seeds(&games);
    Ok(view_from_seeds(&seeds))
}

/// Search VNDB for VNs matching the library's taste.
///
/// # Errors
/// Database failures, and VNDB failures when no cached page exists.
#[tauri::command]
pub async fn for_you_search(
    state: State<'_, AppState>,
    request: ForYouRequest,
) -> CommandResult<ForYouPage> {
    let pool = state.db().pool();
    let timestamp = Timestamp::now().map_err(|err| CommandError::new("clock", err.to_string()))?;
    backfill_missing_credits(pool, state.vndb()).await;
    let games = crate::db::queries::for_you::seeded_games(pool).await?;
    let seeds = for_you::for_you_seeds(&games);

    if !seeds.ready {
        return Ok(ForYouPage {
            items: Vec::new(),
            more: false,
            from_cache: false,
            note: Some("seeds".to_owned()),
        });
    }

    let page = request.page.unwrap_or(1).max(1);
    // The key carries the ranking version: when the arithmetic changes (it
    // did once already — rarity and scopes), pages cached under the old
    // arithmetic must not serve for another day.
    let cache_key = format!(
        "for_you:{}:{}:{}:{page}",
        RANKING_VERSION,
        seeds_hash(&seeds),
        request_key(&request)
    );
    if let Some(payload) = cache::get_fresh(pool, &cache_key, timestamp).await?
        && let Ok(cached) = serde_json::from_str::<ForYouPage>(&payload)
    {
        return Ok(cached);
    }

    // Tag breadth — how many VNs in the whole catalogue carry each seed tag —
    // is what separates a defining theme from catalogue plumbing. One batched
    // lookup per fresh page, itself cached for a week: a tag's breadth drifts
    // on the scale of months, not hours.
    let mut seeds = seeds;
    if let Ok(breadth) = tag_breadth(&state, &seed_tag_ids(&seeds)).await {
        for signal in &mut seeds.tags {
            signal.vn_count = breadth.get(&signal.id).copied();
        }
    }
    // A failed lookup degrades to the neutral middle assumption: rarity is an
    // improvement, not a prerequisite, and the page must not die for it.

    let Some(filters) = build_filter(request.source, &seeds, &request) else {
        return Ok(ForYouPage {
            items: Vec::new(),
            more: false,
            from_cache: false,
            note: Some("source".to_owned()),
        });
    };

    let is_taste = matches!(request.sort, None | Some(ForYouSort::Taste));
    let primary_sort = if request.source == ForYouSource::Voices && is_taste {
        "rating"
    } else {
        sort_key(request.sort)
    };
    let query = VnQuery {
        filters,
        fields: FIELDS.to_owned(),
        results: PAGE_RESULTS,
        page,
        sort: primary_sort.to_owned(),
        reverse: true,
    };
    let primary = match state.vndb().clone().search_vns(&query).await {
        Ok(result) => result,
        Err(err) => {
            // Offline fallback, the same contract as title search: a saved
            // page beats an empty screen, flagged so the UI can say so.
            if let Some(payload) = cache::get_any(pool, &cache_key).await?
                && let Ok(mut cached) = serde_json::from_str::<ForYouPage>(&payload)
            {
                cached.from_cache = true;
                return Ok(cached);
            }
            return Err(CommandError::from(&err));
        }
    };

    let owned = super::vndb::library_vndb_ids(pool).await?;
    let mut pool_vns: Vec<Vn> = Vec::with_capacity(primary.results.len());
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut more = primary.more;
    merge_unowned(&mut pool_vns, &mut seen_ids, &owned, primary.results);

    if is_taste {
        more |= supplement_taste_pool(
            state.vndb(),
            &request,
            &seeds,
            &owned,
            page,
            &mut pool_vns,
            &mut seen_ids,
        )
        .await;
    }

    let ranked = for_you::rank_candidates(
        &seeds,
        &candidates_from_vns(&pool_vns),
        scope(request.source),
    );
    let items = result_items(&pool_vns, &ranked, &owned, request.sort);
    let response = ForYouPage {
        items,
        more,
        from_cache: false,
        note: None,
    };
    if let Ok(serialised) = serde_json::to_string(&response) {
        cache::put(pool, &cache_key, &serialised, RESULT_TTL_MS, timestamp).await?;
    }
    Ok(response)
}

/// Fill missing `staff_json` / `va_json` on existing library details once per
/// un-migrated row so Staff and Voices seeds reflect pre-0037 games too.
async fn backfill_missing_credits(pool: &sqlx::SqlitePool, client: &crate::vndb::VndbClient) {
    let Ok(missing) = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT d.vn_id
         FROM games g
         JOIN vndb_details d ON d.vn_id = g.vndb_id
         WHERE g.deleted = 0 AND (d.staff_json IS NULL OR d.va_json IS NULL)",
    )
    .fetch_all(pool)
    .await
    else {
        return;
    };
    if missing.is_empty() {
        return;
    }

    for chunk in missing.chunks(PAGE_RESULTS as usize) {
        let filters = if chunk.len() == 1 {
            serde_json::json!(["id", "=", chunk[0]])
        } else {
            let mut or_list = vec![serde_json::Value::String("or".to_owned())];
            or_list.extend(chunk.iter().map(|id| serde_json::json!(["id", "=", id])));
            serde_json::Value::Array(or_list)
        };
        let query = VnQuery {
            filters,
            fields: CREDIT_BACKFILL_FIELDS.to_owned(),
            results: PAGE_RESULTS,
            page: 1,
            sort: "id".to_owned(),
            reverse: false,
        };
        let Ok(result) = client.clone().search_vns(&query).await else {
            // Degrade quietly offline; the library stays usable as-is.
            return;
        };
        let mut updated = std::collections::HashSet::with_capacity(result.results.len());
        for vn in result.results {
            let staff_json = super::vndb::encode_list(&vn.staff);
            let va_json = super::vndb::encode_list(&super::vndb::voice_credits(&vn));
            let _ =
                sqlx::query("UPDATE vndb_details SET staff_json = ?, va_json = ? WHERE vn_id = ?")
                    .bind(staff_json)
                    .bind(va_json)
                    .bind(&vn.id)
                    .execute(pool)
                    .await;
            updated.insert(vn.id);
        }
        // Mark any requested VN id omitted by VNDB so subsequent opens do not retry it forever.
        for id in chunk {
            if !updated.contains(id) {
                let _ = sqlx::query(
                    "UPDATE vndb_details
                     SET staff_json = COALESCE(staff_json, '[]'),
                         va_json = COALESCE(va_json, '[]')
                     WHERE vn_id = ?",
                )
                .bind(id)
                .execute(pool)
                .await;
            }
        }
    }
}

fn merge_unowned(
    pool_vns: &mut Vec<Vn>,
    seen_ids: &mut std::collections::HashSet<String>,
    owned: &std::collections::HashSet<String>,
    incoming: Vec<Vn>,
) {
    for vn in incoming {
        if !owned.contains(&vn.id) && seen_ids.insert(vn.id.clone()) {
            pool_vns.push(vn);
        }
    }
}

async fn supplement_taste_pool(
    client: &crate::vndb::VndbClient,
    request: &ForYouRequest,
    seeds: &ForYouSeeds,
    owned: &std::collections::HashSet<String>,
    page: u32,
    pool_vns: &mut Vec<Vn>,
    seen_ids: &mut std::collections::HashSet<String>,
) -> bool {
    let mut more = false;
    match request.source {
        ForYouSource::Combined => {
            if let Some(filters) = affinity_filter(seeds, request) {
                let query = VnQuery {
                    filters,
                    fields: FIELDS.to_owned(),
                    results: PAGE_RESULTS,
                    page,
                    sort: "rating".to_owned(),
                    reverse: true,
                };
                if let Ok(extra) = client.clone().search_vns(&query).await {
                    more |= extra.more;
                    merge_unowned(pool_vns, seen_ids, owned, extra.results);
                }
            }
        }
        ForYouSource::Voices => {
            for actor_id in underrepresented_voice_ids(&seeds.voices, pool_vns) {
                let filters = refine(
                    serde_json::json!(["staff", "=", ["id", "=", actor_id]]),
                    request,
                );
                let query = VnQuery {
                    filters,
                    fields: FIELDS.to_owned(),
                    results: EXTRA_VOICE_RESULTS,
                    page,
                    sort: "rating".to_owned(),
                    reverse: true,
                };
                if let Ok(extra) = client.clone().search_vns(&query).await {
                    more |= extra.more;
                    merge_unowned(pool_vns, seen_ids, owned, extra.results);
                }
            }
        }
        ForYouSource::Tags | ForYouSource::Studios | ForYouSource::Staff => {}
    }
    more
}

fn underrepresented_voice_ids(voices: &[SeedSignal], unowned_vns: &[Vn]) -> Vec<String> {
    let watched: Vec<&str> = voices
        .iter()
        .take(VOICE_TOP_SEEDS)
        .map(|s| s.id.as_str())
        .collect();
    if watched.is_empty() {
        return Vec::new();
    }
    let mut hits: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for vn in unowned_vns {
        let mut matched_in_vn = std::collections::HashSet::new();
        for entry in &vn.va {
            if let Some(id) = entry.staff.as_ref().and_then(|s| s.id.as_deref())
                && watched.contains(&id)
                && matched_in_vn.insert(id)
            {
                *hits.entry(id).or_insert(0) += 1;
            }
        }
    }
    watched
        .into_iter()
        .filter(|id| hits.get(id).copied().unwrap_or(0) < 2)
        .take(MAX_EXTRA_VOICE_QUERIES)
        .map(str::to_owned)
        .collect()
}

/// The field list for one result page: everything a card renders, and the
/// signals ranking reads (tags, staff, voice actors), plus the description
/// for the excerpt. Kept lean — see `PAGE_RESULTS`.
const FIELDS: &str = "title,alttitle,released,image.url,image.sexual,image.violence,length,length_minutes,rating,votecount,devstatus,description,tags.id,tags.name,tags.rating,tags.spoiler,developers.id,developers.name,staff.id,staff.name,staff.role,va.staff.id,va.staff.name";

/// Join candidate metadata with its evidence, then apply the requested order.
/// VNDB supplies a bounded page in vote order for Taste; merely joining the
/// locally ranked entries back to that page would discard the ranking again.
fn result_items(
    vns: &[Vn],
    ranked: &[Ranked],
    owned: &std::collections::HashSet<String>,
    sort: Option<ForYouSort>,
) -> Vec<ForYouCard> {
    if matches!(sort, None | Some(ForYouSort::Taste)) {
        let by_id: std::collections::HashMap<&str, &Vn> =
            vns.iter().map(|vn| (vn.id.as_str(), vn)).collect();
        return ranked
            .iter()
            .filter(|rank| !owned.contains(&rank.id))
            .filter_map(|rank| by_id.get(rank.id.as_str()).map(|vn| card_from_vn(vn, rank)))
            .take(PAGE_RESULTS as usize)
            .collect();
    }

    let by_id: std::collections::HashMap<&str, &Ranked> =
        ranked.iter().map(|rank| (rank.id.as_str(), rank)).collect();
    vns.iter()
        .filter(|vn| !owned.contains(&vn.id))
        .filter_map(|vn| by_id.get(vn.id.as_str()).map(|rank| card_from_vn(vn, rank)))
        .collect()
}

/// Map the sort request onto VNDB's documented sort keys.
///
/// `taste` sorts by votes remotely: the pre-order only has to be stable so
/// pagination is correct, the ranking happens here.
fn sort_key(sort: Option<ForYouSort>) -> &'static str {
    match sort {
        Some(ForYouSort::Rating) => "rating",
        Some(ForYouSort::Released) => "released",
        Some(ForYouSort::Title) => "title",
        // Votes and taste share the remote pre-order: the remote sort only
        // has to be stable so pagination is correct — taste is applied here,
        // on top of whatever VNDB sent.
        Some(ForYouSort::Votes | ForYouSort::Taste) | None => "votecount",
    }
}

/// Which seed categories may score, per tab: the Staff tab keeps VNs its
/// cards can explain, not VNs where a seed person happened to voice a
/// character.
fn scope(source: ForYouSource) -> for_you::RankScope {
    match source {
        ForYouSource::Combined => for_you::RankScope::Combined,
        ForYouSource::Tags => for_you::RankScope::Tags,
        ForYouSource::Studios => for_you::RankScope::Studios,
        ForYouSource::Staff => for_you::RankScope::Staff,
        ForYouSource::Voices => for_you::RankScope::Voices,
    }
}

/// The seed tag ids whose breadth the ranking needs.
fn seed_tag_ids(seeds: &ForYouSeeds) -> Vec<String> {
    seeds.tags.iter().map(|signal| signal.id.clone()).collect()
}

/// Look up how many VNs carry each seed tag, cached a week.
///
/// # Errors
/// Cache or VNDB failures; the caller degrades to neutral weights instead of
/// failing the page.
async fn tag_breadth(
    state: &State<'_, AppState>,
    tag_ids: &[String],
) -> Result<std::collections::HashMap<String, u64>, String> {
    if tag_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let pool = state.db().pool();
    let timestamp = Timestamp::now().map_err(|err| err.to_string())?;
    let mut ids: Vec<String> = tag_ids.to_vec();
    ids.sort();
    let cache_key = format!("tag_breadth:{}", short_hash(&ids.join(",")));
    if let Some(payload) = cache::get_fresh(pool, &cache_key, timestamp)
        .await
        .map_err(|err| err.to_string())?
        && let Ok(cached) = serde_json::from_str::<Vec<crate::vndb::dto::TagCount>>(&payload)
    {
        return Ok(breadth_map(cached));
    }

    let counts = state
        .vndb()
        .clone()
        .tag_counts(&ids)
        .await
        .map_err(|err| err.to_string())?;
    if let Ok(serialised) = serde_json::to_string(&counts) {
        let _ = cache::put(pool, &cache_key, &serialised, BREADTH_TTL_MS, timestamp).await;
    }
    Ok(breadth_map(counts))
}

fn breadth_map(counts: Vec<crate::vndb::dto::TagCount>) -> std::collections::HashMap<String, u64> {
    counts
        .into_iter()
        .filter_map(|tag| Some((tag.id?, tag.vn_count?)))
        .collect()
}

fn push_tag_predicates(
    signals: &mut Vec<serde_json::Value>,
    tags: &[SeedSignal],
    allow_saturated_fallback: bool,
) {
    let before = signals.len();
    signals.extend(
        tags.iter()
            .filter(|seed| for_you::filter_worthy(seed))
            .take(MAX_QUERY_TAGS)
            .map(|seed| {
                serde_json::json!(["tag", "=", [seed.id, TAG_MAX_SPOILER, TAG_MIN_RATING]])
            }),
    );
    if signals.len() == before && allow_saturated_fallback {
        signals.extend(tags.iter().take(MAX_DISPLAY_TAGS).map(|seed| {
            serde_json::json!(["tag", "=", [seed.id, TAG_MAX_SPOILER, TAG_MIN_RATING]])
        }));
    }
}

fn push_nested_predicates(signals: &mut Vec<serde_json::Value>, items: &[SeedSignal], field: &str) {
    signals.extend(
        items
            .iter()
            .map(|seed| serde_json::json!([field, "=", ["id", "=", seed.id]])),
    );
}

fn or_refined(
    mut signals: Vec<serde_json::Value>,
    request: &ForYouRequest,
) -> Option<serde_json::Value> {
    if signals.is_empty() {
        return None;
    }
    if signals.len() == 1 {
        // `and`/`or` take 2+ predicates; a lone seed is its own filter.
        return Some(refine(signals.pop().expect("one signal"), request));
    }
    // `["or", p1, p2, …]` is an array whose first element is the literal
    // string "or".
    let mut combined = vec![serde_json::Value::String("or".to_owned())];
    combined.extend(signals);
    Some(refine(serde_json::Value::Array(combined), request))
}

fn affinity_filter(seeds: &ForYouSeeds, request: &ForYouRequest) -> Option<serde_json::Value> {
    let mut signals: Vec<serde_json::Value> = Vec::new();
    push_nested_predicates(&mut signals, &seeds.developers, "developer");
    push_nested_predicates(&mut signals, &seeds.staff, "staff");
    signals.extend(
        seeds
            .tags
            .iter()
            .filter(|seed| {
                for_you::filter_worthy(seed)
                    && seed.weight >= STANDOUT_TAG_WEIGHT
                    && seed.vn_count.is_some_and(|count| count < RARE_TAG_BELOW)
            })
            .take(8)
            .map(|seed| {
                serde_json::json!(["tag", "=", [seed.id, TAG_MAX_SPOILER, TAG_MIN_RATING]])
            }),
    );
    or_refined(signals, request)
}

/// Build the documented filter JSON for one sub-tab.
///
/// `None` when that category has no seeds — a query with an empty `or` is not
/// "everything", it is a bug, so the caller reports it instead of asking.
fn build_filter(
    source: ForYouSource,
    seeds: &ForYouSeeds,
    request: &ForYouRequest,
) -> Option<serde_json::Value> {
    let mut signals: Vec<serde_json::Value> = Vec::new();
    match source {
        // Voices stay out of the combined federation: an actor shared by two
        // games is a much weaker pull than a theme two games carry, and the
        // Voices tab exists for exactly that signal.
        ForYouSource::Combined => {
            push_tag_predicates(&mut signals, &seeds.tags, false);
            push_nested_predicates(&mut signals, &seeds.developers, "developer");
            push_nested_predicates(&mut signals, &seeds.staff, "staff");
        }
        ForYouSource::Tags => push_tag_predicates(&mut signals, &seeds.tags, true),
        ForYouSource::Studios => {
            push_nested_predicates(&mut signals, &seeds.developers, "developer");
        }
        ForYouSource::Staff => push_nested_predicates(&mut signals, &seeds.staff, "staff"),
        ForYouSource::Voices => push_nested_predicates(&mut signals, &seeds.voices, "staff"),
    }
    or_refined(signals, request)
}

/// Wrap a signal filter with the optional narrowing predicates.
fn refine(signal: serde_json::Value, request: &ForYouRequest) -> serde_json::Value {
    let mut extra: Vec<serde_json::Value> = Vec::new();
    if let Some(rating) = request.rating_min {
        extra.push(serde_json::json!(["rating", ">=", rating]));
    }
    if let Some(votes) = request.votes_min {
        extra.push(serde_json::json!(["votecount", ">=", votes]));
    }
    if let Some(length) = request.length {
        extra.push(serde_json::json!(["length", "=", length]));
    }
    if let Some(language) = &request.language {
        extra.push(serde_json::json!(["lang", "=", language]));
    }
    if let Some(after) = &request.released_after {
        extra.push(serde_json::json!(["released", ">=", after]));
    }
    if extra.is_empty() {
        return signal;
    }
    // `["and", signal, …narrowing predicates]` — the first element is the
    // literal "and", and the signal stays one operand rather than being
    // flattened into the narrowing predicates.
    let mut predicates = vec![serde_json::Value::String("and".to_owned()), signal];
    predicates.extend(extra);
    serde_json::Value::Array(predicates)
}

/// Hash the seeds, so a library change misses the cache instead of serving
/// recommendations built from the old taste.
fn seeds_hash(seeds: &ForYouSeeds) -> String {
    let canonical = serde_json::to_string(seeds).unwrap_or_default();
    short_hash(&canonical)
}

/// Hash the request shape, so two pages of one query share a prefix.
fn request_key(request: &ForYouRequest) -> String {
    let canonical = format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        request.source,
        request.length,
        request.rating_min,
        request.votes_min,
        request.released_after,
        request.language,
        request.sort,
    );
    short_hash(&canonical)
}

fn short_hash(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Map `POST /vn` results into ranking candidates.
fn candidates_from_vns(vns: &[Vn]) -> Vec<Candidate> {
    vns.iter()
        .map(|vn| Candidate {
            id: vn.id.clone(),
            title: vn.title.clone().unwrap_or_else(|| vn.id.clone()),
            rating: vn.rating,
            votecount: vn.votecount.unwrap_or(0),
            tags: vn
                .tags
                .iter()
                .map(|tag| TagWeight {
                    id: tag.id.clone(),
                    name: tag.name.clone().unwrap_or_default(),
                    rating: tag.rating.unwrap_or(0.0),
                    spoiler: tag.spoiler.unwrap_or(0),
                })
                .collect(),
            developers: vn
                .developers
                .iter()
                .filter_map(|producer| {
                    Some(NamedEntity {
                        id: producer.id.clone()?,
                        name: producer.name.clone().unwrap_or_default(),
                    })
                })
                .collect(),
            staff: vn
                .staff
                .iter()
                .filter_map(|member| {
                    Some(StaffCredit {
                        id: member.id.clone()?,
                        name: member.name.clone().unwrap_or_default(),
                        role: member.role.clone().unwrap_or_default(),
                    })
                })
                .collect(),
            voices: vn
                .va
                .iter()
                .filter_map(|entry| entry.staff.as_ref())
                .filter_map(|actor| {
                    Some(NamedEntity {
                        id: actor.id.clone()?,
                        name: actor.name.clone().unwrap_or_default(),
                    })
                })
                .collect(),
        })
        .collect()
}

/// Join one VN with its ranking into a card.
fn card_from_vn(vn: &Vn, rank: &Ranked) -> ForYouCard {
    let image = vn.image.as_ref();
    ForYouCard {
        vn_id: vn.id.clone(),
        title: vn.title.clone().unwrap_or_else(|| vn.id.clone()),
        alttitle: vn.original_title(),
        released: vn.released.clone(),
        image_url: image.and_then(|img| img.url.clone()),
        sexual: image.and_then(crate::vndb::dto::Image::sexual_hundredths),
        violence: image.and_then(crate::vndb::dto::Image::violence_hundredths),
        length: vn.length,
        length_minutes: vn.length_minutes,
        rating: vn.rating,
        votecount: vn.votecount,
        devstatus: vn.devstatus,
        developers: vn
            .developers
            .iter()
            .filter_map(|producer| producer.name.clone())
            .collect(),
        score: rank.score,
        because: rank.because.clone(),
        themes: rank.themes.clone(),
        creators: rank.creators.clone(),
        voices: rank.voices.clone(),
    }
}

fn view_from_seeds(seeds: &ForYouSeeds) -> ForYouProfileView {
    let signals = |list: &[SeedSignal], limit: usize| {
        list.iter()
            .take(limit)
            .map(|signal| SignalView {
                id: signal.id.clone(),
                name: signal.name.clone(),
                count: signal.count,
            })
            .collect()
    };
    ForYouProfileView {
        considered: seeds.considered,
        ready: seeds.ready,
        tags: signals(&seeds.tags, MAX_DISPLAY_TAGS),
        developers: signals(&seeds.developers, seeds.developers.len()),
        staff: signals(&seeds.staff, seeds.staff.len()),
        voices: signals(&seeds.voices, seeds.voices.len()),
        top_games: seeds
            .games
            .iter()
            .take(5)
            .map(|game| game.title.clone())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vn_core::for_you::SeedSignal;

    fn seeds() -> ForYouSeeds {
        ForYouSeeds {
            considered: 3,
            ready: true,
            games: Vec::new(),
            tags: vec![
                SeedSignal {
                    id: "g505".to_owned(),
                    name: "Cherry Blossom".to_owned(),
                    weight: 3.0,
                    count: 3,
                    vn_count: None,
                },
                SeedSignal {
                    id: "g902".to_owned(),
                    name: "Island".to_owned(),
                    weight: 2.0,
                    count: 2,
                    vn_count: None,
                },
            ],
            developers: vec![SeedSignal {
                id: "p8".to_owned(),
                name: "KID".to_owned(),
                weight: 2.0,
                count: 2,
                vn_count: None,
            }],
            staff: vec![SeedSignal {
                id: "s545".to_owned(),
                name: "Nakazawa Takumi".to_owned(),
                weight: 2.0,
                count: 2,
                vn_count: None,
            }],
            voices: vec![SeedSignal {
                id: "s2001".to_owned(),
                name: "Suzumura Kenichi".to_owned(),
                weight: 2.0,
                count: 2,
                vn_count: None,
            }],
        }
    }

    fn request(source: ForYouSource) -> ForYouRequest {
        ForYouRequest {
            source,
            length: None,
            rating_min: None,
            votes_min: None,
            released_after: None,
            language: None,
            sort: None,
            page: None,
        }
    }

    #[test]
    fn tags_query_by_id_with_spoiler_and_rating_bounds() {
        let filter = build_filter(ForYouSource::Tags, &seeds(), &request(ForYouSource::Tags))
            .expect("seeds exist");
        let serde_json::Value::Array(predicates) = &filter else {
            panic!("or array, got {filter}");
        };
        assert_eq!(predicates.len(), 3, "or + two tag seeds, got {filter}");
        assert_eq!(predicates[0], serde_json::json!("or"));
        assert_eq!(
            predicates[1],
            serde_json::json!(["tag", "=", ["g505", TAG_MAX_SPOILER, TAG_MIN_RATING]])
        );
    }

    #[test]
    fn voices_run_through_the_staff_filter() {
        // Verified live: the documented `staff` filter matches voice-acting
        // credits, so one predicate shape covers both tabs.
        let filter = build_filter(
            ForYouSource::Voices,
            &seeds(),
            &request(ForYouSource::Voices),
        )
        .expect("seeds exist");
        assert_eq!(
            filter,
            serde_json::json!(["staff", "=", ["id", "=", "s2001"]])
        );
    }

    #[test]
    fn combined_or_s_tags_developers_and_staff_but_not_voices() {
        let filter = build_filter(
            ForYouSource::Combined,
            &seeds(),
            &request(ForYouSource::Combined),
        )
        .expect("seeds exist");
        let text = filter.to_string();
        assert!(
            text.contains("Cherry Blossom") || text.contains("g505"),
            "tags in: {text}"
        );
        assert!(text.contains("p8"), "developers in: {text}");
        assert!(text.contains("s545"), "staff in: {text}");
        assert!(
            !text.contains("s2001"),
            "voices stay out of the federation: {text}"
        );
    }

    #[test]
    fn optional_filters_narrow_with_and() {
        let mut refined = request(ForYouSource::Studios);
        refined.rating_min = Some(75);
        refined.language = Some("en".to_owned());
        refined.length = Some(3);
        let filter = build_filter(ForYouSource::Studios, &seeds(), &refined).expect("seeds exist");
        let serde_json::Value::Array(predicates) = &filter else {
            panic!("and array, got {filter}");
        };
        // "and", the developer signal, and the three narrowing predicates.
        assert_eq!(predicates.len(), 5, "got {filter}");
        assert_eq!(predicates[0], serde_json::json!("and"));
        assert_eq!(predicates[2], serde_json::json!(["rating", ">=", 75]));
        assert_eq!(predicates[3], serde_json::json!(["length", "=", 3]));
        assert_eq!(predicates[4], serde_json::json!(["lang", "=", "en"]));
    }

    #[test]
    fn a_source_without_seeds_builds_nothing() {
        let mut bare = seeds();
        bare.voices.clear();
        assert!(
            build_filter(ForYouSource::Voices, &bare, &request(ForYouSource::Voices)).is_none()
        );
    }

    #[test]
    fn candidates_carry_the_signals_ranking_reads() {
        let vn: Vn = serde_json::from_str(
            r#"{
              "id": "v17",
              "title": "Ever17",
              "rating": 84.5,
              "votecount": 8800,
              "devstatus": 0,
              "tags": [{"id": "g8", "name": "Fear of Death", "rating": 2.2, "spoiler": 2}],
              "developers": [{"id": "p8", "name": "KID", "type": "co"}],
              "staff": [{"id": "s545", "name": "Nakazawa Takumi", "original": "中澤 工", "eid": null, "role": "scenario", "note": null}],
              "va": [{"staff": {"id": "s2001", "name": "Suzumura Kenichi", "original": "鈴村 健一"}}]
            }"#,
        )
        .expect("decode");
        let candidates = candidates_from_vns(&[vn]);
        let candidate = &candidates[0];
        assert_eq!(candidate.id, "v17");
        assert_eq!(candidate.developers[0].id, "p8");
        assert_eq!(candidate.staff[0].role, "scenario");
        assert_eq!(candidate.voices[0].id, "s2001");
        assert_eq!(candidate.tags[0].id.as_deref(), Some("g8"));
        assert_eq!(candidate.tags[0].spoiler, 2);
    }

    #[test]
    fn a_card_joins_the_rank_with_the_entry() {
        let vn: Vn = serde_json::from_str(
            r#"{
              "id": "v17",
              "title": "Ever17",
              "released": "2002-08-29",
              "image": {"url": "https://t.vndb.org/cv/71/17871.jpg", "sexual": 0.0, "violence": 0.4},
              "length": 4,
              "length_minutes": 1987,
              "rating": 84.5,
              "votecount": 8800,
              "devstatus": 0,
              "developers": [{"id": "p8", "name": "KID", "type": "co"}]
            }"#,
        )
        .expect("decode");
        let rank = Ranked {
            id: "v17".to_owned(),
            score: 7.0,
            because: Some("Ever17".to_owned()),
            themes: vec!["Fear of Death".to_owned()],
            creators: vec!["KID".to_owned()],
            voices: Vec::new(),
        };
        let card = card_from_vn(&vn, &rank);
        assert_eq!(card.vn_id, "v17");
        assert_eq!(
            card.image_url.as_deref(),
            Some("https://t.vndb.org/cv/71/17871.jpg")
        );
        // Hundredths, the units the frontend's NSFW policy already reads.
        assert_eq!(card.sexual, Some(0));
        assert_eq!(card.violence, Some(40));
        assert_eq!(card.length, Some(4));
        assert_eq!(card.developers, vec!["KID"]);
        assert_eq!(card.because.as_deref(), Some("Ever17"));
    }

    #[test]
    fn taste_pages_use_overlap_order_and_exclude_owned_or_unmatched_entries() {
        // VNDB's popular entry arrives first, while the lower-vote entries
        // have stronger local evidence. The response must retain that evidence
        // and its order rather than copying VNDB's popularity order back in.
        let vns: Vec<Vn> = serde_json::from_value(serde_json::json!([
            { "id": "v9", "title": "Popular", "votecount": 9000 },
            { "id": "v2", "title": "Related B", "votecount": 200 },
            { "id": "v1", "title": "Related A", "votecount": 100 },
            { "id": "v3", "title": "Owned" },
            { "id": "v4", "title": "No matching evidence" }
        ]))
        .expect("decode candidates");
        let rank = |id: &str, score: f32| Ranked {
            id: id.to_owned(),
            score,
            because: Some("Library seed".to_owned()),
            themes: vec!["Shared theme".to_owned()],
            creators: Vec::new(),
            voices: Vec::new(),
        };
        let ranked = vec![
            rank("v3", 10.0),
            rank("v1", 6.0),
            rank("v2", 6.0),
            rank("v9", 1.0),
        ];
        let owned = std::collections::HashSet::from(["v3".to_owned()]);
        for sort in [None, Some(ForYouSort::Taste)] {
            let items = result_items(&vns, &ranked, &owned, sort);
            assert_eq!(
                items
                    .iter()
                    .map(|item| item.vn_id.as_str())
                    .collect::<Vec<_>>(),
                ["v1", "v2", "v9"]
            );
            assert_eq!(items[0].title, "Related A");
            assert_eq!(items[0].because.as_deref(), Some("Library seed"));
        }
        for sort in [
            ForYouSort::Rating,
            ForYouSort::Votes,
            ForYouSort::Released,
            ForYouSort::Title,
        ] {
            let items = result_items(&vns, &ranked, &owned, Some(sort));
            assert_eq!(
                items
                    .iter()
                    .map(|item| item.vn_id.as_str())
                    .collect::<Vec<_>>(),
                ["v9", "v2", "v1"],
                "explicit catalogue sorts keep the incoming order"
            );
        }
    }

    #[test]
    fn seed_hashes_change_when_the_library_changes() {
        let mut changed = seeds();
        changed.tags[0].count += 1;
        assert_ne!(seeds_hash(&seeds()), seeds_hash(&changed));
    }

    #[test]
    fn taste_result_items_preserve_rank_order_including_voice_diversity() {
        let vns: Vec<Vn> = serde_json::from_value(serde_json::json!([
            { "id": "v1", "title": "First Raw" },
            { "id": "v2", "title": "Diverse Second" },
            { "id": "v3", "title": "First Actor Repeated" }
        ]))
        .expect("decode candidates");
        // Even if the diversified rank order puts v2 ahead of v3, result_items
        // preserves `ranked` order for Taste rather than re-sorting blindly.
        let rank = |id: &str, score: f32| Ranked {
            id: id.to_owned(),
            score,
            because: Some("Seed".to_owned()),
            themes: Vec::new(),
            creators: Vec::new(),
            voices: vec!["Actor".to_owned()],
        };
        let ranked = vec![rank("v2", 14.0), rank("v3", 14.0), rank("v1", 10.0)];
        let items = result_items(
            &vns,
            &ranked,
            &std::collections::HashSet::new(),
            Some(ForYouSort::Taste),
        );
        let ids: Vec<&str> = items.iter().map(|item| item.vn_id.as_str()).collect();
        assert_eq!(ids, ["v2", "v3", "v1"]);
    }

    #[test]
    fn underrepresented_voice_detection_targets_missing_top_actors() {
        let make_va = |id: &str| SeedSignal {
            id: id.to_owned(),
            name: id.to_owned(),
            weight: 10.0,
            count: 2,
            vn_count: None,
        };
        let seeds = vec![make_va("va1"), make_va("va2"), make_va("va3")];
        let vns: Vec<Vn> = serde_json::from_value(serde_json::json!([
            { "id": "v1", "va": [{ "staff": { "id": "va1", "name": "A" } }] },
            { "id": "v2", "va": [{ "staff": { "id": "va1", "name": "A" } }] },
            { "id": "v3", "va": [{ "staff": { "id": "va2", "name": "B" } }] }
        ]))
        .expect("decode");
        // va1 has 2 hits (well represented); va2 has 1 (< 2); va3 has 0 (< 2).
        let missing = underrepresented_voice_ids(&seeds, &vns);
        assert_eq!(missing, ["va2", "va3"]);
    }

    #[test]
    fn affinity_filter_targets_creators_and_rare_standout_tags() {
        let mut s = seeds();
        s.tags[0].weight = 5.0;
        s.tags[0].vn_count = Some(85);
        s.tags[1].weight = 5.0;
        s.tags[1].vn_count = Some(20_000);
        let filter = affinity_filter(&s, &request(ForYouSource::Combined)).expect("affinity");
        let text = filter.to_string();
        assert!(
            text.contains("\"p8\""),
            "developer in affinity filter: {text}"
        );
        assert!(
            text.contains("\"s545\""),
            "staff in affinity filter: {text}"
        );
        assert!(
            text.contains("\"g505\""),
            "rare standout tag in affinity filter: {text}"
        );
        assert!(
            !text.contains("\"g902\""),
            "saturated tag stays out: {text}"
        );
    }
}
