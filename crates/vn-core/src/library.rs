//! Pure library queries: sorting, filtering, search and aggregation.
//!
//! These operate on in-memory slices so the same code serves the desktop table
//! view, the mobile list, and the stats page without `SQL` duplication.

use serde::{Deserialize, Serialize};

use crate::model::{Game, GameStatus, NsfwMode};

/// How the library list is ordered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    /// Most recently touched first.
    #[default]
    Recent,
    /// Title, case-insensitive.
    Title,
    /// Highest rating first, unrated last.
    Rating,
    /// Most tracked time first.
    Playtime,
    /// Newest `started_at` first.
    Started,
}

/// Filter criteria for the library.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryFilter {
    /// Free text matched against title and alternate title.
    pub query: String,
    /// Restrict to these statuses; empty means any.
    pub statuses: Vec<GameStatus>,
    /// Restrict to rated games only.
    pub rated_only: bool,
}

impl LibraryFilter {
    /// True when nothing is being filtered out.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.query.trim().is_empty() && self.statuses.is_empty() && !self.rated_only
    }
}

/// Normalise a title for comparison: lowercase, trim, collapse whitespace.
#[must_use]
pub fn normalize_title(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = true;
    for ch in text.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(ch);
            last_was_space = false;
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Case-insensitive substring match over the title and original title.
#[must_use]
pub fn matches_query(game: &Game, query: &str) -> bool {
    let needle = normalize_title(query);
    if needle.is_empty() {
        return true;
    }
    let title = normalize_title(&game.title);
    if title.contains(&needle) {
        return true;
    }
    game.title_original
        .as_deref()
        .is_some_and(|alt| normalize_title(alt).contains(&needle))
}

/// Apply a filter to a slice, returning matching games in original order.
#[must_use]
pub fn apply_filter<'a>(games: &'a [Game], filter: &LibraryFilter) -> Vec<&'a Game> {
    games
        .iter()
        .filter(|game| {
            if game.meta.deleted {
                return false;
            }
            if !matches_query(game, &filter.query) {
                return false;
            }
            if !filter.statuses.is_empty() && !filter.statuses.contains(&game.status) {
                return false;
            }
            if filter.rated_only && game.rating.is_none() {
                return false;
            }
            true
        })
        .collect()
}

/// Sort games in place by `key` and `descending`.
pub fn sort_games(
    games: &mut [&Game],
    key: SortKey,
    descending: bool,
    playtime_of: impl Fn(&Game) -> i64,
) {
    match key {
        SortKey::Recent => games.sort_by_key(|g| g.meta.updated_at),
        SortKey::Title => games.sort_by_cached_key(|g| normalize_title(&g.title)),
        SortKey::Rating => games.sort_by_key(|g| g.rating.unwrap_or(0)),
        SortKey::Playtime => games.sort_by_key(|g| playtime_of(g)),
        SortKey::Started => games.sort_by_key(|g| g.started_at.unwrap_or_default()),
    }
    if descending {
        games.reverse();
    }
}

/// Aggregate play counts per status, for the stats donut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatusBreakdown {
    /// Games currently reading.
    pub playing: usize,
    /// Games planned.
    pub wishlist: usize,
    /// Games finished.
    pub completed: usize,
    /// Games paused.
    pub on_hold: usize,
    /// Games abandoned.
    pub dropped: usize,
}

impl StatusBreakdown {
    /// Count live games by status.
    #[must_use]
    pub fn from_games(games: &[Game]) -> Self {
        let mut out = Self::default();
        for game in games.iter().filter(|g| !g.meta.deleted) {
            match game.status {
                GameStatus::Wishlist => out.wishlist += 1,
                GameStatus::Playing => out.playing += 1,
                GameStatus::Completed => out.completed += 1,
                GameStatus::OnHold => out.on_hold += 1,
                GameStatus::Dropped => out.dropped += 1,
            }
        }
        out
    }

    /// Build from per-status totals produced by SQL (`GROUP BY status`).
    ///
    /// The database does the counting; this maps rows to the shape the UI wants
    /// so the two paths (in-memory and `SQL`) cannot disagree on field names.
    #[must_use]
    pub fn from_rows(rows: &[StatusRowTotal]) -> Self {
        let mut out = Self::default();
        for row in rows {
            match row.status {
                GameStatus::Wishlist => out.wishlist = row.count,
                GameStatus::Playing => out.playing = row.count,
                GameStatus::Completed => out.completed = row.count,
                GameStatus::OnHold => out.on_hold = row.count,
                GameStatus::Dropped => out.dropped = row.count,
            }
        }
        out
    }

    /// Total across all statuses.
    #[must_use]
    pub fn total(&self) -> usize {
        self.playing + self.wishlist + self.completed + self.on_hold + self.dropped
    }
}

/// One status and how many games carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusRowTotal {
    /// The status.
    pub status: GameStatus,
    /// Number of live games with that status.
    pub count: usize,
}

impl StatusRowTotal {
    /// Pair a status with a count.
    #[must_use]
    pub const fn new(status: GameStatus, count: usize) -> Self {
        Self { status, count }
    }
}

/// Which games have art the user must reveal first.
#[must_use]
pub fn needs_reveal(game: &Game, mode: NsfwMode) -> bool {
    game.is_explicit(mode) && game.cover_url.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Timestamp;
    use crate::ids::{DeviceId, EntityId};

    /// `rating` is in decivotes, matching what the model stores.
    fn game(title: &str, status: GameStatus, rating: Option<u8>) -> Game {
        let device = DeviceId::parse("dev").unwrap();
        let mut g = Game::new(&device, Timestamp::from_millis(1_000), title, status).unwrap();
        g.set_rating(rating).unwrap();
        g
    }

    #[test]
    fn normalize_handles_unicode_and_spaces() {
        assert_eq!(normalize_title("  Hello   World  "), "hello world");
        assert_eq!(normalize_title("ＡＢＣ"), "ａｂｃ");
        assert_eq!(normalize_title("うみねこのなく頃に"), "うみねこのなく頃に");
    }

    #[test]
    fn search_matches_title_and_original_title() {
        let mut g = game("Little Busters!", GameStatus::Playing, None);
        assert!(matches_query(&g, "busters"));
        assert!(!matches_query(&g, "clannad"));

        g.title_original = Some("リトルバスターズ！".to_owned());
        assert!(matches_query(&g, "リトル"));
        assert!(matches_query(&g, "LITTLE"));
    }

    #[test]
    fn empty_query_matches_everything() {
        let g = game("Anything", GameStatus::Playing, None);
        assert!(matches_query(&g, ""));
        assert!(matches_query(&g, "   "));
    }

    #[test]
    fn filter_excludes_tombstones() {
        let mut a = game("A", GameStatus::Playing, None);
        let b = game("B", GameStatus::Playing, None);
        a.meta = a.meta.tombstone(Timestamp::from_millis(2_000));
        let games = [a, b];
        let result = apply_filter(&games, &LibraryFilter::default());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "B");
    }

    #[test]
    fn filter_by_status_and_rating() {
        let games = vec![
            game("A", GameStatus::Playing, Some(90)),
            game("B", GameStatus::Completed, None),
            game("C", GameStatus::Playing, None),
        ];
        let playing = apply_filter(
            &games,
            &LibraryFilter {
                statuses: vec![GameStatus::Playing],
                ..LibraryFilter::default()
            },
        );
        assert_eq!(playing.len(), 2);

        let rated = apply_filter(
            &games,
            &LibraryFilter {
                rated_only: true,
                ..LibraryFilter::default()
            },
        );
        assert_eq!(rated.len(), 1);
        assert_eq!(rated[0].title, "A");
    }

    #[test]
    fn sort_by_rating_puts_unrated_last_ascending() {
        let games = [
            game("A", GameStatus::Playing, None),
            game("B", GameStatus::Playing, Some(50)),
            game("C", GameStatus::Playing, Some(90)),
        ];
        let mut refs: Vec<&Game> = games.iter().collect();
        sort_games(&mut refs, SortKey::Rating, false, |_| 0);
        assert_eq!(
            refs.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(),
            vec!["A", "B", "C"]
        );
    }

    #[test]
    fn sort_descending_reverses() {
        let games = [
            game("A", GameStatus::Playing, Some(50)),
            game("B", GameStatus::Playing, Some(90)),
        ];
        let mut refs: Vec<&Game> = games.iter().collect();
        sort_games(&mut refs, SortKey::Rating, true, |_| 0);
        assert_eq!(refs[0].title, "B");
    }

    #[test]
    fn status_breakdown_counts_live_games_only() {
        let mut dead = game("dead", GameStatus::Playing, None);
        dead.meta = dead.meta.tombstone(Timestamp::from_millis(1));
        let games = vec![
            game("a", GameStatus::Playing, None),
            game("b", GameStatus::Playing, None),
            game("c", GameStatus::Completed, None),
            game("d", GameStatus::Wishlist, None),
            dead,
        ];
        let b = StatusBreakdown::from_games(&games);
        assert_eq!(b.playing, 2);
        assert_eq!(b.completed, 1);
        assert_eq!(b.wishlist, 1);
        assert_eq!(b.total(), 4);
    }

    #[test]
    fn reveal_needed_only_when_art_exists() {
        let mut g = game("A", GameStatus::Playing, None);
        g.cover_sexual = Some(3);
        assert!(!needs_reveal(&g, NsfwMode::Blur));
        g.cover_url = Some("https://example.invalid/c.jpg".to_owned());
        assert!(needs_reveal(&g, NsfwMode::Blur));
        assert!(!needs_reveal(&g, NsfwMode::Show));
    }

    #[test]
    fn status_breakdown_from_rows_matches_from_games() {
        let games = vec![
            game("a", GameStatus::Playing, None),
            game("b", GameStatus::Playing, None),
            game("c", GameStatus::Completed, None),
            game("d", GameStatus::OnHold, None),
            game("e", GameStatus::Dropped, None),
            game("f", GameStatus::Wishlist, None),
        ];
        let from_games = StatusBreakdown::from_games(&games);
        let from_rows = StatusBreakdown::from_rows(&[
            StatusRowTotal::new(GameStatus::Playing, 2),
            StatusRowTotal::new(GameStatus::Completed, 1),
            StatusRowTotal::new(GameStatus::OnHold, 1),
            StatusRowTotal::new(GameStatus::Dropped, 1),
            StatusRowTotal::new(GameStatus::Wishlist, 1),
        ]);
        assert_eq!(from_games, from_rows);
    }

    #[test]
    fn entity_ids_are_stable_in_filtered_output() {
        let g = game("A", GameStatus::Playing, None);
        let id: EntityId = g.meta.id;
        let games = [g];
        let out = apply_filter(&games, &LibraryFilter::default());
        assert_eq!(out[0].meta.id, id);
    }
}
