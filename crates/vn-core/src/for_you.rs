//! "For you": ranking VNDB candidates against the library's own taste.
//!
//! Deterministic like [`crate::taste`]: every score sums library seed overlaps
//! (recurring themes, studios, staff, voices) against VNDB query candidates.
//!
//! # Spoilers
//!
//! Spoiler tags are excluded, as are candidates matched *only* by a spoiler tag.

use serde::{Deserialize, Serialize};

use crate::taste::{
    BOILERPLATE_BELOW, MINIMUM_COUNT, NamedEntity, ProfiledGame, StaffCredit, TagWeight,
};

/// Minimum rating (8.0 on the decivote scale) where a game scales up its seed weight.
pub(crate) const LIKED_RATING: u8 = 80;
/// Rating (9.0 on the 10-point scale) where standout favourites unlock higher
/// weights and can carry single-game studio/theme seeds.
const STANDOUT_RATING: u8 = 90;
/// Games rated below 6.0 are treated as neutral/disliked and do not seed.
const DISLIKED_BELOW_RATING: u8 = 60;
/// Weight floor for a single 9.0+ favourite to carry a studio or theme seed.
const STANDOUT_SOLO_WEIGHT: f32 = 3.75;

/// Catalogue-score multiplier on a shared developer or production credit so
/// creator matches compete cleanly against rare tag matches (`rarity` ≈ 3–6).
const CREATOR_SCORE_FACTOR: f32 = 3.0;
/// Diminishing-return factor (D'Hondt divisor `weight / (1 + picked)`)
/// applied to repeated voice actors in the Voices tab so every favoured actor
/// gets a spotlight before one prolific actor repeats.
const VOICE_REPEAT_PENALTY: f32 = 1.0;

/// Seeds below this many usable games do not query VNDB at all (minimum for "several games").
pub(crate) const MINIMUM_SEEDS: usize = 3;

/// How many seeds of each kind to keep, strongest first.
const MAX_TAG_SEEDS: usize = 24;
const MAX_DEVELOPER_SEEDS: usize = 10;
const MAX_STAFF_SEEDS: usize = 20;
const MAX_VOICE_SEEDS: usize = 16;

/// Staff roles that seed the query (writers and artists; directors/composers span whole catalogues).
/// All roles stay stored so future signals need no re-fetch.
pub(crate) const SEED_STAFF_ROLES: [&str; 2] = ["scenario", "art"];

/// Approximate VNDB catalogue size (measured 2026-11-13: 67,911) used on log scale.
pub(crate) const CATALOGUE_VNS: f64 = 68_000.0;

/// Fallback tag breadth when the catalogue lookup is unavailable.
pub(crate) const UNKNOWN_TAG_COUNT: f64 = 3_000.0;

/// Tags on >= this many VNs (e.g. `ADV` 36,329, `Male Protagonist` 23,226, `Romance` 20,367)
/// still score lightly in-panel but do not drive the VNDB query filter.
pub(crate) const FILTER_SATURATION: u64 = 10_000;

/// Extra weight for a shared studio/writer in "Because you liked X" (direct links beat themes).
pub(crate) const CREATOR_EXPLANATION_BOOST: f32 = 2.0;

/// Restricts scoring to one tab's categories and drops unexplainable matches
/// (e.g. VNDB's `staff` filter also returns voice credits, which belong in Voices only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankScope {
    /// Tags, developers, and staff (default "Combined" view).
    Combined,
    /// Tags only.
    Tags,
    /// Developers only.
    Studios,
    /// Production staff only.
    Staff,
    /// Voice actors only.
    Voices,
}

impl RankScope {
    /// Whether this category contributes to the score in this scope.
    fn scores(self, category: Category) -> bool {
        match self {
            RankScope::Combined => {
                matches!(
                    category,
                    Category::Tag | Category::Developer | Category::Staff
                )
            }
            RankScope::Tags => category == Category::Tag,
            RankScope::Studios => category == Category::Developer,
            RankScope::Staff => category == Category::Staff,
            RankScope::Voices => category == Category::Voice,
        }
    }
}

/// The four signal categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Tag,
    Developer,
    Staff,
    Voice,
}

/// One recurring signal in the library's taste.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedSignal {
    /// Stable VNDB id: `g505` for a tag, `p8` for a developer, `s123` for a
    /// person. VNDB queries are written against ids, not names.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Summed seed weight of the games carrying it.
    pub weight: f32,
    /// Distinct games carrying this signal (shown in the panel).
    pub count: usize,
    /// Capped titles behind `count` so claims match the shelf (after collapsing
    /// duplicate works, matching `crate::taste::Theme::examples`).
    #[serde(default)]
    pub examples: Vec<String>,
    /// Catalogue-wide VN count for tags from `POST /tag`; `None` falls back to
    /// a neutral middle weight when the lookup was unavailable.
    #[serde(default)]
    pub vn_count: Option<u64>,
}

/// One seed game, with the signals it carries, so a candidate can be
/// explained by the game it overlaps most.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedGame {
    /// Game id in the library.
    pub id: String,
    /// Display title.
    pub title: String,
    /// 1, or 2 when the user rated it [`LIKED_RATING`] or higher.
    pub weight: f32,
    /// Seed tag ids this game carries.
    pub tags: Vec<String>,
    /// Seed developer ids.
    pub developers: Vec<String>,
    /// Seed staff ids.
    pub staff: Vec<String>,
    /// Seed voice-actor ids.
    pub voices: Vec<String>,
}

/// The library's taste as the "For you" query needs it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForYouSeeds {
    /// Distinct works that contributed at least one signal.
    pub considered: usize,
    /// False while the library is too small to ask VNDB anything.
    pub ready: bool,
    /// Seed games, strongest first; the source of every explanation.
    pub games: Vec<SeedGame>,
    /// Recurring themes.
    pub tags: Vec<SeedSignal>,
    /// Recurring studios.
    pub developers: Vec<SeedSignal>,
    /// Recurring scenario writers and artists.
    pub staff: Vec<SeedSignal>,
    /// Recurring voice actors.
    pub voices: Vec<SeedSignal>,
}

/// One VNDB candidate as ranking needs it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// VN id, e.g. `v17`.
    pub id: String,
    /// Title.
    pub title: String,
    /// Bayesian rating 10..100, when voted.
    pub rating: Option<f32>,
    /// Number of votes.
    pub votecount: u32,
    /// Tags applied directly to the VN, with their ratings and spoiler levels.
    pub tags: Vec<TagWeight>,
    /// Developers.
    pub developers: Vec<NamedEntity>,
    /// Production staff credits.
    pub staff: Vec<StaffCredit>,
    /// Voice actors.
    pub voices: Vec<NamedEntity>,
}

/// A ranked candidate: the overlap a user can check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ranked {
    /// VN id, mirroring the candidate's, so a card can be joined back.
    pub id: String,
    /// Summed match weight (a weighted count, not a probability; higher is stronger).
    pub score: f32,
    /// The seed game this candidate overlaps most, for "Because you liked X".
    pub because: Option<String>,
    /// Matched themes the library carries, strongest seed first.
    pub themes: Vec<String>,
    /// Matched studios and staff names.
    pub creators: Vec<String>,
    /// Matched voice actors.
    pub voices: Vec<String>,
}

/// The seed weight of one game: dropped and unrated wishlist entries stay out,
/// while played games scale steeply through the top ratings so 9.0–10.0
/// favourites clearly lead the profile.
fn seed_weight(status: &str, rating: Option<u8>) -> f32 {
    if status == "dropped" {
        return 0.0;
    }
    if status == "wishlist" && rating.is_none() {
        return 0.0;
    }
    match rating {
        Some(value) if value < DISLIKED_BELOW_RATING => 0.0,
        Some(value) if value < 70 => 0.5,
        Some(value) if value < LIKED_RATING => 1.0,
        Some(value) if value < 85 => 1.75,
        Some(value) if value < STANDOUT_RATING => 2.75,
        Some(value) if value < 95 => 3.75,
        Some(value) if value < 97 => 4.5,
        Some(value) if value < 100 => 5.0,
        Some(_) => 5.5,
        None => 1.0,
    }
}

/// Whether a tag says something about taste, as [`crate::taste`] decides it.
fn usable_tag(tag: &TagWeight) -> bool {
    tag.spoiler == 0 && tag.rating >= BOILERPLATE_BELOW
}

/// Whether a staff credit seeds the query.
fn usable_staff(credit: &StaffCredit) -> bool {
    SEED_STAFF_ROLES.contains(&credit.role.as_str())
}

/// Builds library seeds after collapsing duplicate works like [`crate::taste::profile`];
/// `considered` counts only games that contributed at least one usable signal.
#[must_use]
pub fn for_you_seeds(games: &[ProfiledGame]) -> ForYouSeeds {
    // Collapse duplicate works, keeping the first entry, like taste does.
    let mut seen_works = std::collections::HashSet::new();
    let distinct: Vec<&ProfiledGame> = games
        .iter()
        .filter(|game| {
            game.work_id
                .as_ref()
                .is_none_or(|work| seen_works.insert(work.clone()))
        })
        .collect();

    let mut games_out: Vec<SeedGame> = Vec::new();
    let mut tallies = SeedTallies::default();
    for game in &distinct {
        if let Some(seed) = seed_game(game, &mut tallies) {
            games_out.push(seed);
        }
    }

    let considered = games_out.len();
    // Strongest games first, so "because" ties resolve to the stronger seed.
    games_out.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.title.cmp(&b.title))
    });

    ForYouSeeds {
        considered,
        ready: considered >= MINIMUM_SEEDS,
        games: games_out,
        tags: ordered(tallies.tags, MAX_TAG_SEEDS, true),
        developers: ordered(tallies.developers, MAX_DEVELOPER_SEEDS, true),
        staff: ordered(tallies.staff, MAX_STAFF_SEEDS, false),
        voices: ordered(tallies.voices, MAX_VOICE_SEEDS, false),
    }
}

/// Per-category signal evidence for one library pass.
#[derive(Default)]
struct SeedTallies {
    tags: Tally,
    developers: Tally,
    staff: Tally,
    voices: Tally,
}

/// One signal's evidence: id, display name, summed seed weight, how many
/// distinct games carry it, and up to [`MAX_SIGNAL_EXAMPLES`] of those games'
/// titles.
type Tally = Vec<(String, String, f32, usize, Vec<String>)>;

/// How many example titles a seed signal keeps, as
/// `crate::taste::Theme::examples` does for themes.
const MAX_SIGNAL_EXAMPLES: usize = 3;

/// Record one game's weight behind a signal. One game counts once per signal
/// id even if its row lists a credit twice.
fn tally(tally: &mut Tally, id: &str, name: &str, weight: f32, title: &str) {
    match tally.iter_mut().find(|(existing, ..)| existing == id) {
        Some((_, _, sum, count, examples)) => {
            *sum += weight;
            *count += 1;
            if examples.len() < MAX_SIGNAL_EXAMPLES {
                examples.push(title.to_owned());
            }
        }
        None => tally.push((
            id.to_owned(),
            name.to_owned(),
            weight,
            1,
            vec![title.to_owned()],
        )),
    }
}

/// Orders seeds by summed weight, distinct-game count, then name for stable runs.
/// Keeps recurring signals (`count >= MINIMUM_COUNT`) and, when `allow_standout_solo`
/// is set, single-game `9.0+` favourites (`weight >= STANDOUT_SOLO_WEIGHT`).
fn ordered(tally: Tally, cap: usize, allow_standout_solo: bool) -> Vec<SeedSignal> {
    let mut signals: Vec<SeedSignal> = tally
        .into_iter()
        .map(|(id, name, weight, count, examples)| SeedSignal {
            id,
            name,
            weight,
            count,
            examples,
            // Filled by the caller from the catalogue lookup, where known.
            vn_count: None,
        })
        .collect();
    signals.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.count.cmp(&a.count))
            .then_with(|| a.name.cmp(&b.name))
    });
    signals.retain(|signal| {
        signal.count >= MINIMUM_COUNT
            || (allow_standout_solo && signal.weight >= STANDOUT_SOLO_WEIGHT)
    });
    signals.truncate(cap);
    signals
}

/// Extract one game's seed signals and tally them.
///
/// `None` when the game is dropped, unrated on the wishlist, rated poorly, or
/// carries no usable signal at all: it neither queries nor explains.
fn seed_game(game: &ProfiledGame, tallies: &mut SeedTallies) -> Option<SeedGame> {
    let weight = seed_weight(&game.status, game.rating);
    if weight <= 0.0 {
        return None;
    }

    let tags: Vec<String> = game
        .tags
        .iter()
        .filter(|tag| usable_tag(tag))
        .filter_map(|tag| tag.id.clone())
        .collect();
    let developers: Vec<String> = game.developers.iter().map(|d| d.id.clone()).collect();
    let staff: Vec<String> = game
        .staff
        .iter()
        .filter(|credit| usable_staff(credit))
        .map(|credit| credit.id.clone())
        .collect();
    let voices: Vec<String> = game.voices.iter().map(|v| v.id.clone()).collect();

    let has_signal =
        !tags.is_empty() || !developers.is_empty() || !staff.is_empty() || !voices.is_empty();
    if !has_signal {
        return None;
    }

    for id in &tags {
        let name = game
            .tags
            .iter()
            .find(|tag| tag.id.as_deref() == Some(id.as_str()))
            .map(|tag| tag.name.clone())
            .unwrap_or_default();
        tally(&mut tallies.tags, id, &name, weight, &game.title);
    }
    for entity in &game.developers {
        tally(
            &mut tallies.developers,
            &entity.id,
            &entity.name,
            weight,
            &game.title,
        );
    }
    for credit in game.staff.iter().filter(|credit| usable_staff(credit)) {
        tally(
            &mut tallies.staff,
            &credit.id,
            &credit.name,
            weight,
            &game.title,
        );
    }
    for entity in &game.voices {
        tally(
            &mut tallies.voices,
            &entity.id,
            &entity.name,
            weight,
            &game.title,
        );
    }

    Some(SeedGame {
        id: game.id.clone(),
        title: game.title.clone(),
        weight,
        tags,
        developers,
        staff,
        voices,
    })
}

/// Inverse-document-frequency tag discount: `ln(total / carried)`.
/// (e.g. Cherry Blossom 85 -> 6.7, Nakige 410 -> 5.1, Romance 20,367 -> 1.2, ADV 36,329 -> 0.6;
/// missing count falls back to `UNKNOWN_TAG_COUNT`).
#[allow(
    clippy::cast_possible_truncation, // a log of a bounded count, far inside f32
    clippy::cast_precision_loss // catalogue counts are thousands, far inside f64's exact range
)]
fn rarity(vn_count: Option<u64>) -> f32 {
    let carried = vn_count
        .map_or(UNKNOWN_TAG_COUNT, |count| count as f64)
        .max(1.0);
    (CATALOGUE_VNS / carried).ln() as f32
}

/// Whether a tag seed may drive the VNDB query (see [`FILTER_SATURATION`]).
#[must_use]
pub fn filter_worthy(signal: &SeedSignal) -> bool {
    signal
        .vn_count
        .is_none_or(|count| count < FILTER_SATURATION)
}

/// One matched signal between a candidate and a seed.
struct Match {
    /// Signal id, for crediting the seed games that carry it.
    id: String,
    /// Display name for the card's explanation lines.
    name: String,
    /// Which evidence this is.
    category: Category,
    /// Summed seed weight behind the signal.
    weight: f32,
    /// Per-game multiplier: the candidate's own tag rating (centre themes
    /// count triple faint ones) scaled by the tag's rarity.
    factor: f32,
}

/// Every signal a candidate shares with a seed, ready for scoring.
fn collect_matches(seeds: &ForYouSeeds, candidate: &Candidate) -> Vec<Match> {
    let mut matched: Vec<Match> = Vec::new();
    for seed in &seeds.tags {
        let Some(applied) = candidate
            .tags
            .iter()
            .find(|tag| tag.id.as_deref() == Some(seed.id.as_str()) && usable_tag(tag))
        else {
            continue;
        };
        matched.push(Match {
            id: seed.id.clone(),
            name: seed.name.clone(),
            category: Category::Tag,
            weight: seed.weight,
            factor: rarity(seed.vn_count) * applied.rating / 3.0,
        });
    }
    let mut push_all = |list: &[SeedSignal], category: Category| {
        for seed in list {
            let present = match category {
                Category::Developer => candidate
                    .developers
                    .iter()
                    .any(|developer| developer.id == seed.id),
                Category::Staff => candidate
                    .staff
                    .iter()
                    .any(|credit| usable_staff(credit) && credit.id == seed.id),
                Category::Voice => candidate.voices.iter().any(|voice| voice.id == seed.id),
                Category::Tag => false,
            };
            if present {
                let factor = if matches!(category, Category::Developer | Category::Staff) {
                    CREATOR_SCORE_FACTOR
                } else {
                    1.0
                };
                matched.push(Match {
                    id: seed.id.clone(),
                    name: seed.name.clone(),
                    category,
                    weight: seed.weight,
                    factor,
                });
            }
        }
    };
    push_all(&seeds.developers, Category::Developer);
    push_all(&seeds.staff, Category::Staff);
    push_all(&seeds.voices, Category::Voice);
    matched
}

/// Picks the seed game sharing the most weighted evidence for "Because you liked X".
/// Shared creators carry double weight and constrain the choice so a creator match
/// is never paired with an explanation game that lacks that creator.
fn explanation_for<'a>(games: &'a [SeedGame], scoring: &[&Match]) -> Option<(&'a SeedGame, f32)> {
    let boost = |m: &Match| {
        if matches!(m.category, Category::Developer | Category::Staff) {
            m.factor * CREATOR_EXPLANATION_BOOST
        } else {
            m.factor
        }
    };
    let carries = |game: &SeedGame, m: &Match| {
        // Production and voice credits share VNDB person ids. Credit the
        // role that actually matched, otherwise a highly rated voice-only
        // game can explain a writer/artist recommendation.
        let ids = match m.category {
            Category::Tag => &game.tags,
            Category::Developer => &game.developers,
            Category::Staff => &game.staff,
            Category::Voice => &game.voices,
        };
        ids.contains(&m.id)
    };
    let shared_creators: Vec<&Match> = scoring
        .iter()
        .filter(|m| matches!(m.category, Category::Developer | Category::Staff))
        .copied()
        .collect();
    let mut best: Option<(&SeedGame, f32)> = None;
    for game in games {
        let evidence: &[&Match] = if shared_creators.is_empty() {
            scoring
        } else {
            &shared_creators
        };
        let part: f32 = evidence
            .iter()
            .filter(|m| carries(game, m))
            .map(|m| game.weight * boost(m))
            .sum();
        if part > 0.0 && best.is_none_or(|(_, best_part)| part > best_part) {
            best = Some((game, part));
        }
    }
    best
}

struct VoiceEntry {
    ranked: Ranked,
    rating: Option<f32>,
    votecount: u32,
    pairs: Vec<(String, f32)>,
}

fn marginal_voice_score(
    pairs: &[(String, f32)],
    picked: &std::collections::HashMap<String, u16>,
) -> f32 {
    pairs
        .iter()
        .map(|(id, weight)| {
            let count = picked.get(id).copied().unwrap_or(0);
            *weight / (1.0 + VOICE_REPEAT_PENALTY * f32::from(count))
        })
        .sum()
}

fn is_better_voice(cand_score: f32, cand: &VoiceEntry, best_score: f32, best: &VoiceEntry) -> bool {
    match cand_score.total_cmp(&best_score) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match cand
            .rating
            .unwrap_or(0.0)
            .total_cmp(&best.rating.unwrap_or(0.0))
        {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => cand
                .votecount
                .cmp(&best.votecount)
                .then_with(|| best.ranked.id.cmp(&cand.ranked.id))
                .is_gt(),
        },
    }
}

fn diversify_voices(mut entries: Vec<VoiceEntry>) -> Vec<Ranked> {
    let mut picked: std::collections::HashMap<String, u16> = std::collections::HashMap::new();
    let mut out = Vec::with_capacity(entries.len());
    while !entries.is_empty() {
        let mut best_idx = 0;
        let mut best_score = marginal_voice_score(&entries[0].pairs, &picked);
        for idx in 1..entries.len() {
            let score = marginal_voice_score(&entries[idx].pairs, &picked);
            if is_better_voice(score, &entries[idx], best_score, &entries[best_idx]) {
                best_idx = idx;
                best_score = score;
            }
        }
        let mut chosen = entries.remove(best_idx);
        chosen.ranked.score = best_score;
        for (id, _) in &chosen.pairs {
            let entry = picked.entry(id.clone()).or_insert(0);
            *entry = entry.saturating_add(1);
        }
        out.push(chosen.ranked);
    }
    out
}

/// Ranks candidates against `seeds` within `scope`, dropping unexplainable matches
/// (e.g. voice-only matches from VNDB's `staff` filter) and weighting tags by [`rarity`].
#[must_use]
pub fn rank_candidates(
    seeds: &ForYouSeeds,
    candidates: &[Candidate],
    scope: RankScope,
) -> Vec<Ranked> {
    let mut entries = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let matched = collect_matches(seeds, candidate);
        let scoring: Vec<&Match> = matched
            .iter()
            .filter(|m| scope.scores(m.category))
            .collect();
        if scoring.is_empty() {
            continue;
        }
        let total: f32 = scoring.iter().map(|m| m.weight * m.factor).sum();
        let because = explanation_for(&seeds.games, &scoring);
        let pairs: Vec<(String, f32)> = scoring.iter().map(|m| (m.id.clone(), m.weight)).collect();
        let collect_names = |pred: fn(&Match) -> bool| -> Vec<String> {
            scoring
                .iter()
                .filter(|m| pred(m))
                .map(|m| m.name.clone())
                .collect()
        };

        entries.push(VoiceEntry {
            ranked: Ranked {
                id: candidate.id.clone(),
                score: total,
                because: because.map(|(game, _)| game.title.clone()),
                themes: collect_names(|m| m.category == Category::Tag),
                creators: collect_names(|m| {
                    matches!(m.category, Category::Developer | Category::Staff)
                }),
                voices: collect_names(|m| m.category == Category::Voice),
            },
            rating: candidate.rating,
            votecount: candidate.votecount,
            pairs,
        });
    }

    if scope == RankScope::Voices {
        return diversify_voices(entries);
    }

    // Strongest overlap first, breaking exact ties by VN rating, votecount, and id.
    entries.sort_by(|a, b| {
        b.ranked
            .score
            .total_cmp(&a.ranked.score)
            .then_with(|| b.rating.unwrap_or(0.0).total_cmp(&a.rating.unwrap_or(0.0)))
            .then_with(|| b.votecount.cmp(&a.votecount))
            .then_with(|| a.ranked.id.cmp(&b.ranked.id))
    });
    entries.into_iter().map(|entry| entry.ranked).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(id: &str, name: &str, rating: f32) -> TagWeight {
        TagWeight {
            id: Some(id.to_owned()),
            name: name.to_owned(),
            rating,
            spoiler: 0,
        }
    }

    fn spoiler_tag(id: &str, name: &str) -> TagWeight {
        TagWeight {
            id: Some(id.to_owned()),
            name: name.to_owned(),
            rating: 2.5,
            spoiler: 2,
        }
    }

    fn entity(id: &str, name: &str) -> NamedEntity {
        NamedEntity {
            id: id.to_owned(),
            name: name.to_owned(),
        }
    }

    fn credit(id: &str, name: &str, role: &str) -> StaffCredit {
        StaffCredit {
            id: id.to_owned(),
            name: name.to_owned(),
            role: role.to_owned(),
        }
    }

    fn game(id: &str, title: &str, tags: Vec<TagWeight>) -> ProfiledGame {
        ProfiledGame {
            id: id.to_owned(),
            title: title.to_owned(),
            status: "completed".to_owned(),
            rating: None,
            tags,
            work_id: None,
            developers: Vec::new(),
            staff: Vec::new(),
            voices: Vec::new(),
        }
    }

    #[test]
    fn the_wire_format_is_camel_case() {
        // The frontend parses these keys directly. A snake_case field here would
        // arrive as undefined in the UI and render as a blank, which has happened
        // twice elsewhere in this codebase for exactly this reason.
        let games = vec![
            game(
                "1",
                "A",
                vec![tag("g1", "Nakige", 2.98), tag("g2", "Island", 2.9)],
            ),
            game(
                "2",
                "B",
                vec![tag("g1", "Nakige", 2.96), tag("g2", "Island", 2.9)],
            ),
            game(
                "3",
                "C",
                vec![tag("g1", "Nakige", 2.9), tag("g2", "Island", 2.9)],
            ),
        ];
        let text = serde_json::to_string(&for_you_seeds(&games)).expect("serialise");
        assert!(text.contains("\"considered\""), "got {text}");
        assert!(text.contains("\"ready\""), "got {text}");

        let ranked = rank_candidates(
            &for_you_seeds(&games),
            &[Candidate {
                id: "v1".to_owned(),
                title: "B".to_owned(),
                tags: vec![tag("g1", "Nakige", 3.0)],
                ..Candidate::default()
            }],
            RankScope::Combined,
        );
        let text = serde_json::to_string(&ranked).expect("serialise");
        assert!(text.contains("\"because\""), "got {text}");
        assert!(text.contains("\"creators\""), "got {text}");
    }

    #[test]
    fn high_ratings_scale_seed_weight() {
        let mut liked = game("1", "A", vec![tag("g1", "Nakige", 2.98)]);
        liked.rating = Some(85);
        let plain = game("2", "B", vec![tag("g1", "Nakige", 2.96)]);
        let seeds = for_you_seeds(&[liked, plain]);
        let nakige = &seeds.tags[0];
        assert_eq!(nakige.count, 2, "two games");
        assert!(
            (nakige.weight - 3.75).abs() < 1e-6,
            "2.75 + 1.0, got {}",
            nakige.weight
        );
    }

    #[test]
    fn spoiler_and_boilerplate_tags_do_not_seed() {
        let games = vec![
            game(
                "1",
                "A",
                vec![tag("g1", "Nakige", 2.98), spoiler_tag("g9", "Heroine Dies")],
            ),
            game(
                "2",
                "B",
                vec![tag("g2", "ADV", 2.86), tag("g1", "Nakige", 2.96)],
            ),
        ];
        let seeds = for_you_seeds(&games);
        let ids: Vec<&str> = seeds.tags.iter().map(|signal| signal.id.as_str()).collect();
        assert_eq!(ids, vec!["g1"], "got {ids:?}");
    }

    #[test]
    fn two_entries_for_one_work_do_not_double_a_signal() {
        let mut first = game("1", "Umineko", vec![tag("g1", "Battle of Wits", 2.97)]);
        first.work_id = Some("v24".to_owned());
        let mut second = game("2", "うみねこ", vec![tag("g1", "Battle of Wits", 2.97)]);
        second.work_id = Some("v24".to_owned());
        let seeds = for_you_seeds(&[first, second]);
        assert_eq!(seeds.considered, 1);
        // One work is a single data point: below the pattern threshold, so no
        // seed comes out of it (same rule taste applies to one game).
        assert!(seeds.tags.is_empty(), "got {:?}", seeds.tags);
    }

    #[test]
    fn a_signal_on_one_game_is_not_a_seed() {
        let games = vec![
            game(
                "1",
                "A",
                vec![tag("g1", "Shared", 2.98), tag("g2", "Only A", 2.9)],
            ),
            game("2", "B", vec![tag("g1", "Shared", 2.96)]),
        ];
        let seeds = for_you_seeds(&games);
        let ids: Vec<&str> = seeds.tags.iter().map(|signal| signal.id.as_str()).collect();
        assert_eq!(ids, vec!["g1"], "got {ids:?}");
    }

    #[test]
    fn only_writers_and_artists_seed_staff() {
        let games = vec![
            game("1", "A", vec![tag("g1", "Shared", 2.98)]),
            game("2", "B", vec![tag("g1", "Shared", 2.96)]),
        ];
        let mut with_people = games.clone();
        with_people[0].staff = vec![
            credit("s1", "Writer", "scenario"),
            credit("s2", "Artist", "art"),
            credit("s3", "Composer", "music"),
        ];
        with_people[1].staff = vec![
            credit("s1", "Writer", "scenario"),
            credit("s3", "Composer", "music"),
        ];
        let seeds = for_you_seeds(&with_people);
        let ids: Vec<&str> = seeds
            .staff
            .iter()
            .map(|signal| signal.id.as_str())
            .collect();
        assert_eq!(ids, vec!["s1"], "composer must not seed, got {ids:?}");
    }

    #[test]
    fn a_tiny_library_is_not_ready() {
        let games = vec![
            game("1", "A", vec![tag("g1", "Shared", 2.98)]),
            game("2", "B", vec![tag("g1", "Shared", 2.96)]),
        ];
        assert!(!for_you_seeds(&games).ready);
        let games = vec![
            game("1", "A", vec![tag("g1", "Shared", 2.98)]),
            game("2", "B", vec![tag("g1", "Shared", 2.96)]),
            game("3", "C", vec![tag("g1", "Shared", 2.9)]),
        ];
        assert!(for_you_seeds(&games).ready);
    }

    #[test]
    fn seed_caps_hold() {
        // 25 shared tags must truncate to 24; the strongest survive.
        let games: Vec<ProfiledGame> = (0..3)
            .map(|index| {
                let tags = (0..25)
                    .map(|t| tag(&format!("g{t}"), &format!("Tag {t}"), 2.9))
                    .collect();
                game(&index.to_string(), &format!("G{index}"), tags)
            })
            .collect::<Vec<_>>();
        let seeds = for_you_seeds(&games);
        assert_eq!(seeds.tags.len(), 24);
    }

    #[test]
    fn examples_name_the_games_behind_the_count() {
        // The taste panel shows counts a user must be able to check against
        // their shelf; the seed signal carries the same evidence.
        let games = vec![
            game("1", "A", vec![tag("g1", "Nakige", 2.98)]),
            game("2", "B", vec![tag("g1", "Nakige", 2.96)]),
        ];
        let seeds = for_you_seeds(&games);
        assert_eq!(seeds.tags[0].examples, vec!["A", "B"]);
    }

    #[test]
    fn examples_are_capped() {
        // Five games carry one tag: three titles survive, the count stays five.
        let games: Vec<ProfiledGame> = (0..5)
            .map(|index| {
                game(
                    &index.to_string(),
                    &format!("G{index}"),
                    vec![tag("g1", "Shared", 2.9)],
                )
            })
            .collect();
        let seeds = for_you_seeds(&games);
        assert_eq!(seeds.tags[0].count, 5);
        assert_eq!(seeds.tags[0].examples.len(), 3);
        assert_eq!(seeds.tags[0].examples, vec!["G0", "G1", "G2"]);
    }

    #[test]
    fn examples_follow_the_work_collapse() {
        // Two entries for one work must not put the same story in examples twice;
        // work collapse runs before any tally.
        let mut first = game("1", "Umineko", vec![tag("g1", "Battle of Wits", 2.97)]);
        first.work_id = Some("v24".to_owned());
        let mut second = game("2", "うみねこ", vec![tag("g1", "Battle of Wits", 2.97)]);
        second.work_id = Some("v24".to_owned());
        let third = game("3", "CLANNAD", vec![tag("g1", "Battle of Wits", 2.9)]);
        let seeds = for_you_seeds(&[first, second, third]);
        assert_eq!(seeds.tags[0].examples, vec!["Umineko", "CLANNAD"]);
        assert!(!seeds.tags[0].examples.contains(&"うみねこ".to_owned()));
    }

    #[test]
    fn creator_examples_carry_their_games() {
        // Studios, staff and voices expose the same checkable evidence as tags.
        let mut liked = game("1", "Summer Pockets", Vec::new());
        liked.rating = Some(90);
        liked.developers = vec![entity("p1", "Key")];
        liked.staff = vec![credit("s1", "Na-Ga", "art")];
        liked.voices = vec![entity("va1", "Water Summit")];
        let mut second = game("2", "CLANNAD", Vec::new());
        second.developers = vec![entity("p1", "Key")];
        second.staff = vec![credit("s1", "Na-Ga", "art")];
        second.voices = vec![entity("va1", "Water Summit")];
        let third = game("3", "Air", Vec::new());
        let seeds = for_you_seeds(&[liked, second, third]);
        assert_eq!(
            seeds.developers[0].examples,
            vec!["Summer Pockets", "CLANNAD"]
        );
        assert_eq!(seeds.staff[0].examples, vec!["Summer Pockets", "CLANNAD"]);
        assert_eq!(seeds.voices[0].examples, vec!["Summer Pockets", "CLANNAD"]);
    }

    /// Three games sharing Nakige, with Key/Na-Ga/one VA across two of them,
    /// one of them rated 9.0.
    fn seeds() -> ForYouSeeds {
        let mut liked = game(
            "1",
            "Summer Pockets",
            vec![tag("g1", "Nakige", 2.98), tag("g2", "Island", 2.9)],
        );
        liked.rating = Some(90);
        liked.developers = vec![entity("p1", "Key")];
        liked.staff = vec![credit("s1", "Na-Ga", "art")];
        liked.voices = vec![entity("va1", "Water Summit")];
        let mut second = game("2", "CLANNAD", vec![tag("g1", "Nakige", 2.96)]);
        second.developers = vec![entity("p1", "Key")];
        second.staff = vec![credit("s1", "Na-Ga", "art")];
        second.voices = vec![entity("va1", "Water Summit")];
        for_you_seeds(&[
            liked,
            second,
            game("3", "Air", vec![tag("g1", "Nakige", 2.9)]),
        ])
    }

    #[test]
    fn a_candidate_is_scored_by_what_it_shares() {
        let seed_set = seeds();
        let candidate = Candidate {
            id: "v1".to_owned(),
            title: "A New Key Game".to_owned(),
            tags: vec![tag("g1", "Nakige", 3.0)],
            developers: vec![entity("p1", "Key")],
            ..Candidate::default()
        };
        let ranked = rank_candidates(&seed_set, &[candidate], RankScope::Combined);
        assert_eq!(ranked.len(), 1);
        // The Nakige seed weight (a 90-rated 3.75 + two plain 1.0s = 5.75)
        // times the unknown-count rarity and the candidate's own tag rating
        // (3.0/3.0), plus Key's seed weight (4.75) at CREATOR_SCORE_FACTOR.
        let nakige = 5.75 * rarity(None) * 3.0 / 3.0;
        let expected_key = 4.75 * CREATOR_SCORE_FACTOR;
        let top = &ranked[0];
        assert!(
            (top.score - (nakige + expected_key)).abs() < 1e-4,
            "got {}",
            top.score
        );
        assert_eq!(top.themes, vec!["Nakige"]);
        assert!(top.creators.contains(&"Key".to_owned()));
        assert!(top.voices.is_empty());
    }

    #[test]
    fn a_rare_theme_outranks_a_universal_one() {
        // Measured breadths: Nakige sits on 410 VNs, Male Protagonist on
        // 23,226. Same seed weight, same candidate tag rating: the specific
        // theme must win, or the tab reads as "the most-voted VNs".
        let games = vec![
            game(
                "1",
                "A",
                vec![
                    tag("g1", "Nakige", 2.98),
                    tag("g2", "Male Protagonist", 2.9),
                ],
            ),
            game(
                "2",
                "B",
                vec![
                    tag("g1", "Nakige", 2.96),
                    tag("g2", "Male Protagonist", 2.9),
                ],
            ),
            game(
                "3",
                "C",
                vec![tag("g1", "Nakige", 2.9), tag("g2", "Male Protagonist", 2.9)],
            ),
        ];
        let mut seed_set = for_you_seeds(&games);
        for signal in &mut seed_set.tags {
            signal.vn_count = Some(match signal.id.as_str() {
                "g1" => 410,
                _ => 23_226,
            });
        }
        let specific = Candidate {
            id: "v1".to_owned(),
            title: "Specific".to_owned(),
            // A real usable tag rating (the boilerplate cut sits at 2.88);
            // still weaker per-application than the universal one's 3.0.
            tags: vec![tag("g1", "Nakige", 2.98)],
            ..Candidate::default()
        };
        let universal = Candidate {
            id: "v2".to_owned(),
            title: "Universal".to_owned(),
            tags: vec![tag("g2", "Male Protagonist", 3.0)],
            ..Candidate::default()
        };
        let ranked = rank_candidates(&seed_set, &[universal, specific], RankScope::Tags);
        assert_eq!(
            ranked[0].id,
            "v1",
            "the rare match leads, got {:?}",
            ranked
                .iter()
                .map(|r| (r.id.as_str(), r.score))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn universal_tags_do_not_drive_the_query() {
        let signal = |count: Option<u64>| SeedSignal {
            id: "g1".to_owned(),
            name: "T".to_owned(),
            weight: 1.0,
            count: 2,
            examples: Vec::new(),
            vn_count: count,
        };
        assert!(!filter_worthy(&signal(Some(20_367))), "Romance is plumbing");
        assert!(!filter_worthy(&signal(Some(FILTER_SATURATION))));
        assert!(filter_worthy(&signal(Some(410))), "Nakige is taste");
        assert!(filter_worthy(&signal(None)), "unknown stays in, not out");
    }

    #[test]
    fn the_explanation_names_the_strongest_overlapping_game() {
        let mut liked = game(
            "1",
            "Summer Pockets",
            vec![tag("g1", "Nakige", 2.98), tag("g2", "Island", 2.9)],
        );
        liked.rating = Some(90);
        liked.developers = vec![entity("p1", "Key")];
        let other = game("2", "CLANNAD", vec![tag("g1", "Nakige", 2.96)]);
        let third = game("3", "Air", vec![tag("g1", "Nakige", 2.9)]);
        let seed_set = for_you_seeds(&[liked, other, third]);

        // Shares g1 and p1 with Summer Pockets, only g1 with the others.
        let candidate = Candidate {
            id: "v9".to_owned(),
            title: "Rewrite".to_owned(),
            tags: vec![tag("g1", "Nakige", 2.9)],
            developers: vec![entity("p1", "Key")],
            ..Candidate::default()
        };
        let ranked = rank_candidates(&seed_set, &[candidate], RankScope::Combined);
        assert_eq!(ranked[0].because.as_deref(), Some("Summer Pockets"));
    }

    #[test]
    fn a_candidate_matched_only_through_a_spoiler_tag_scores_zero() {
        let games = vec![
            game("1", "A", vec![tag("g1", "Shared", 2.98)]),
            game("2", "B", vec![tag("g1", "Shared", 2.96)]),
            game("3", "C", vec![tag("g1", "Shared", 2.9)]),
        ];
        let seed_set = for_you_seeds(&games);
        let candidate = Candidate {
            id: "v9".to_owned(),
            title: "Twist Machine".to_owned(),
            tags: vec![spoiler_tag("g1", "Shared")],
            ..Candidate::default()
        };
        let ranked = rank_candidates(&seed_set, &[candidate], RankScope::Tags);
        // Nothing usable matched, so the scope cannot explain this candidate
        // at all: it is dropped rather than ranked at zero.
        assert!(ranked.is_empty(), "got {ranked:?}");
    }

    #[test]
    fn a_staff_tab_drops_a_voice_only_match() {
        // The documented `staff` filter matches voice-acting credits too, so
        // a VN where a seed person *voiced* a character arrives in the Staff
        // tab with no production credit to show for it. The tab must not
        // present what it cannot explain; the Voices tab keeps it.
        let mut liked = game("1", "A", vec![tag("g1", "Shared", 2.98)]);
        liked.staff = vec![credit("s1", "Writer", "scenario")];
        liked.voices = vec![entity("va1", "Famous Voice")];
        let mut second = game("2", "B", vec![tag("g1", "Shared", 2.96)]);
        second.staff = vec![credit("s1", "Writer", "scenario")];
        second.voices = vec![entity("va1", "Famous Voice")];
        let third = game("3", "C", vec![tag("g1", "Shared", 2.9)]);
        let seed_set = for_you_seeds(&[liked, second, third]);

        let voice_only = Candidate {
            id: "v9".to_owned(),
            title: "A VN They Voiced In".to_owned(),
            voices: vec![entity("va1", "Famous Voice")],
            ..Candidate::default()
        };
        assert!(
            rank_candidates(
                &seed_set,
                std::slice::from_ref(&voice_only),
                RankScope::Staff
            )
            .is_empty()
        );
        let kept = rank_candidates(&seed_set, &[voice_only], RankScope::Voices);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].voices, vec!["Famous Voice"]);
    }

    #[test]
    fn the_explanation_prefers_the_game_sharing_the_creator() {
        // The reported distribution: the higher-rated CIRCUS game carries
        // Romance, while the Yuzusoft games contribute the studio alone.
        // Keep the tag above the usable threshold, otherwise this regression
        // silently tests a creator-only candidate and cannot catch the bug.
        let mut circus = game("1", "D.C.5", vec![tag("g1", "Romance", 3.0)]);
        circus.rating = Some(95);
        let mut yuzusoft = game("2", "Sanoba Witch", Vec::new());
        yuzusoft.developers = vec![entity("p1", "Yuzusoft")];
        let mut third = game("3", "Tenshin Ranman", Vec::new());
        third.developers = vec![entity("p1", "Yuzusoft")];
        let fourth = game("4", "D.C.4", vec![tag("g1", "Romance", 3.0)]);
        let mut seed_set = for_you_seeds(&[circus, yuzusoft, third, fourth]);
        assert_eq!(seed_set.tags.len(), 1, "Romance must actually contribute");
        // Romance's real breadth: it must not dominate the explanation.
        for signal in &mut seed_set.tags {
            signal.vn_count = Some(20_367);
        }

        let candidate = Candidate {
            id: "v9".to_owned(),
            title: "Senren Banka".to_owned(),
            tags: vec![tag("g1", "Romance", 3.0)],
            developers: vec![entity("p1", "Yuzusoft")],
            ..Candidate::default()
        };
        let ranked = rank_candidates(&seed_set, &[candidate], RankScope::Combined);
        assert_eq!(ranked[0].because.as_deref(), Some("Sanoba Witch"));
        assert_eq!(ranked[0].creators, vec!["Yuzusoft"]);
        assert_eq!(ranked[0].themes, vec!["Romance"]);
    }

    #[test]
    fn production_explanations_do_not_credit_a_voice_role_with_the_same_person_id() {
        let mut voiced = game("1", "Voice-only game", Vec::new());
        voiced.rating = Some(95);
        voiced.voices = vec![entity("s1", "Creator")];
        let mut written = game("2", "Written game", Vec::new());
        written.staff = vec![credit("s1", "Creator", "scenario")];
        let mut other = game("3", "Other written game", Vec::new());
        other.staff = vec![credit("s1", "Creator", "scenario")];
        let seeds = for_you_seeds(&[voiced, written, other]);
        let candidate = Candidate {
            id: "v9".to_owned(),
            staff: vec![credit("s1", "Creator", "scenario")],
            ..Candidate::default()
        };
        let ranked = rank_candidates(&seeds, &[candidate], RankScope::Staff);
        assert_eq!(ranked[0].because.as_deref(), Some("Other written game"));
        assert_eq!(ranked[0].creators, vec!["Creator"]);
    }

    #[test]
    fn a_staff_tab_requires_a_writer_or_artist_credit_on_the_candidate() {
        let mut first = game("1", "Written game", Vec::new());
        first.staff = vec![credit("s1", "Creator", "scenario")];
        let mut second = game("2", "Other written game", Vec::new());
        second.staff = vec![credit("s1", "Creator", "art")];
        let third = game("3", "Other game", vec![tag("g1", "Theme", 3.0)]);
        let seeds = for_you_seeds(&[first, second, third]);
        let mut candidate = Candidate {
            id: "v9".to_owned(),
            staff: vec![credit("s1", "Creator", "music")],
            ..Candidate::default()
        };
        assert!(
            rank_candidates(&seeds, std::slice::from_ref(&candidate), RankScope::Staff).is_empty(),
            "a composer credit does not establish the requested writer/artist match"
        );
        candidate.staff[0].role = "scenario".to_owned();
        let ranked = rank_candidates(&seeds, &[candidate], RankScope::Staff);
        assert_eq!(ranked[0].creators, vec!["Creator"]);
    }

    #[test]
    fn equal_scores_stay_stable_by_id() {
        let games = vec![
            game("1", "A", vec![tag("g1", "Shared", 2.98)]),
            game("2", "B", vec![tag("g1", "Shared", 2.96)]),
            game("3", "C", vec![tag("g1", "Shared", 2.9)]),
        ];
        let seed_set = for_you_seeds(&games);
        let make = |id: &str| Candidate {
            id: id.to_owned(),
            title: "Same".to_owned(),
            tags: vec![tag("g1", "Shared", 2.9)],
            ..Candidate::default()
        };
        let ranked = rank_candidates(&seed_set, &[make("v2"), make("v1")], RankScope::Tags);
        assert_eq!(ranked[0].id, "v1", "ties break by id, stable between runs");
        let again = rank_candidates(&seed_set, &[make("v2"), make("v1")], RankScope::Tags);
        assert_eq!(ranked, again);
    }

    #[test]
    fn dropped_wishlist_and_low_rated_games_do_not_seed() {
        let mut dropped = game("1", "Dropped Gem", vec![tag("g1", "Nakige", 3.0)]);
        dropped.status = "dropped".to_owned();
        dropped.rating = Some(95);
        dropped.developers = vec![entity("p1", "Yuzusoft")];

        let mut wishlist = game("2", "Backlog VN", vec![tag("g1", "Nakige", 3.0)]);
        wishlist.status = "wishlist".to_owned();
        wishlist.developers = vec![entity("p1", "Yuzusoft")];

        let mut low = game("3", "Disliked", vec![tag("g1", "Nakige", 3.0)]);
        low.rating = Some(50);

        let mut fav = game("4", "Rewrite", vec![tag("g2", "Occult Club", 3.0)]);
        fav.rating = Some(100);
        fav.developers = vec![entity("p2", "Key")];

        let seed_set = for_you_seeds(&[dropped, wishlist, low, fav]);
        assert_eq!(
            seed_set.considered, 1,
            "only the 100-rated played game counts"
        );
        assert!(!seed_set.ready);
        assert_eq!(seed_set.games[0].title, "Rewrite");
        assert!(seed_set.developers.iter().all(|d| d.id != "p1"));
    }

    #[test]
    fn standout_favorites_seed_unique_studios_and_themes_without_flooding_people() {
        let mut gem = game(
            "1",
            "FORTUNE×WORLD",
            vec![tag("g9", "Cherry Blossom", 2.95)],
        );
        gem.rating = Some(100);
        gem.developers = vec![entity("p9", "GLOVETY")];
        gem.staff = vec![credit("s9", "One-Off Writer", "scenario")];
        gem.voices = vec![entity("va9", "One-Off Voice")];

        let other1 = game("2", "Title A", vec![tag("g1", "Nakige", 2.9)]);
        let other2 = game("3", "Title B", vec![tag("g1", "Nakige", 2.9)]);

        let seed_set = for_you_seeds(&[gem, other1, other2]);
        assert!(seed_set.ready);
        assert!(seed_set.developers.iter().any(|d| d.id == "p9"));
        assert!(seed_set.tags.iter().any(|t| t.id == "g9"));
        assert!(
            seed_set.staff.is_empty() && seed_set.voices.is_empty(),
            "single-game ensemble credits must not flood staff or voices"
        );
    }

    #[test]
    fn voice_diversity_prevents_one_prolific_actor_from_monopolizing_page() {
        let mut g1 = game("1", "Rewrite", Vec::new());
        g1.rating = Some(100);
        g1.voices = vec![
            entity("va_tanaka", "Tanaka Ryouko"),
            entity("va_koga", "Koga Aoi"),
            entity("va_sazanami", "Sazanami Suzu"),
        ];

        let mut g2 = game("2", "Little Busters!", Vec::new());
        g2.rating = Some(97);
        g2.voices = vec![
            entity("va_tanaka", "Tanaka Ryouko"),
            entity("va_hanazawa", "Hanazawa Kana"),
        ];

        let mut g3 = game("3", "D.C.5", Vec::new());
        g3.rating = Some(95);
        g3.voices = vec![
            entity("va_koga", "Koga Aoi"),
            entity("va_hanazawa", "Hanazawa Kana"),
            entity("va_sazanami", "Sazanami Suzu"),
        ];

        let seed_set = for_you_seeds(&[g1, g2, g3]);
        // Prolific actor Tanaka has 4 solo candidates; Koga, Hanazawa and Sazanami
        // have fewer. Without diversity all 4 Tanaka entries crowd the top 4.
        let vn = |id: &str, va: &str, rating: f32| Candidate {
            id: id.to_owned(),
            title: id.to_owned(),
            rating: Some(rating),
            votecount: 500,
            voices: vec![entity(va, va)],
            ..Candidate::default()
        };
        let multi = Candidate {
            id: "v_multi".to_owned(),
            title: "Multi".to_owned(),
            rating: Some(82.0),
            votecount: 800,
            voices: vec![
                entity("va_koga", "Koga Aoi"),
                entity("va_hanazawa", "Hanazawa Kana"),
            ],
            ..Candidate::default()
        };
        let candidates = vec![
            vn("t1", "va_tanaka", 85.0),
            vn("t2", "va_tanaka", 84.0),
            vn("t3", "va_tanaka", 83.0),
            vn("t4", "va_tanaka", 82.0),
            vn("s1", "va_sazanami", 81.0),
            vn("k1", "va_koga", 81.0),
            vn("h1", "va_hanazawa", 81.0),
            multi,
        ];

        let ranked = rank_candidates(&seed_set, &candidates, RankScope::Voices);
        let top_ids: Vec<&str> = ranked.iter().take(4).map(|r| r.id.as_str()).collect();
        assert_eq!(top_ids[0], "v_multi", "multi-actor overlap leads");
        assert!(
            top_ids.contains(&"t1"),
            "Tanaka appears in top 4: {top_ids:?}"
        );
        assert!(
            top_ids.contains(&"s1"),
            "untouched actor Sazanami precedes Tanaka's 2nd entry: {top_ids:?}"
        );
        // Every seeded actor gets featured within the top 5 slots instead of
        // waiting behind 4 solo Tanaka titles.
        let all_voiced: std::collections::HashSet<&str> = ranked
            .iter()
            .take(5)
            .flat_map(|r| r.voices.iter().map(String::as_str))
            .collect();
        assert_eq!(
            all_voiced.len(),
            4,
            "all 4 actors represented: {all_voiced:?}"
        );
        assert!(
            ranked.windows(2).all(|pair| pair[0].score >= pair[1].score),
            "marginal voice scores must stay monotonically non-increasing"
        );
    }

    #[test]
    fn equal_overlap_ties_break_by_vndb_rating_before_id() {
        let mut g1 = game("1", "G1", Vec::new());
        g1.rating = Some(95);
        g1.developers = vec![entity("p1", "Key")];
        let mut g2 = game("2", "G2", Vec::new());
        g2.rating = Some(95);
        g2.developers = vec![entity("p1", "Key")];
        let g3 = game("3", "G3", vec![tag("g1", "Shared", 2.9)]);

        let seed_set = for_you_seeds(&[g1, g2, g3]);
        let low_rated = Candidate {
            id: "v10".to_owned(),
            title: "Low".to_owned(),
            rating: Some(60.0),
            votecount: 100,
            developers: vec![entity("p1", "Key")],
            ..Candidate::default()
        };
        let high_rated = Candidate {
            id: "v999".to_owned(),
            title: "High".to_owned(),
            rating: Some(86.0),
            votecount: 100,
            developers: vec![entity("p1", "Key")],
            ..Candidate::default()
        };
        let ranked = rank_candidates(&seed_set, &[low_rated, high_rated], RankScope::Studios);
        assert_eq!(ranked[0].id, "v999");
    }
}
