import { useEffect, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { useAppStore } from "@/app/store";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { Icon } from "@/components/Icon";
import { api, IpcError } from "@/lib/ipc/client";
import type {
  ForYouCard,
  ForYouProfile,
  ForYouRequest,
  ForYouSort,
  ForYouSource,
} from "@/lib/ipc/types";
import { isExplicitHit } from "@/lib/vndbSearch";

/**
 * "For you" recommendation feed scored deterministically from local rating-weighted
 * library seeds (tags, studios, staff, voices >= 8.0 weighted 2x); obeys per-image concealment.
 */

/** The seed categories, in display order. */
const SOURCE_TABS = [
  ["combined", "recommend.foryou.sourceCombined"],
  ["tags", "recommend.foryou.sourceTags"],
  ["studios", "recommend.foryou.sourceStudios"],
  ["staff", "recommend.foryou.sourceStaff"],
  ["voices", "recommend.foryou.sourceVoices"],
] as const;

/** The length buckets VNDB's `length` filter accepts, 0 meaning any. */
const LENGTHS = [
  [0, "recommend.foryou.lengthAny"],
  [1, "recommend.foryou.length1"],
  [2, "recommend.foryou.length2"],
  [3, "recommend.foryou.length3"],
  [4, "recommend.foryou.length4"],
  [5, "recommend.foryou.length5"],
] as const;

/** Rating floors on VNDB's 10..100 scale, 0 meaning any. */
const RATINGS = [
  [0, "recommend.foryou.ratingAny"],
  [70, "recommend.foryou.rating70"],
  [75, "recommend.foryou.rating75"],
  [80, "recommend.foryou.rating80"],
  [85, "recommend.foryou.rating85"],
] as const;

/** Vote-count floors: "popularity" in the reference design. */
const VOTES = [
  [0, "recommend.foryou.popularityAny"],
  [100, "recommend.foryou.popularity100"],
  [500, "recommend.foryou.popularity500"],
  [2000, "recommend.foryou.popularity2000"],
] as const;

/** Languages the panel offers as a first cut. */
const LANGUAGES = [
  ["", "recommend.foryou.languageAny"],
  ["en", "recommend.foryou.languageEn"],
  ["ja", "recommend.foryou.languageJa"],
  ["zh", "recommend.foryou.languageZh"],
  ["ko", "recommend.foryou.languageKo"],
] as const;

/** The sorts; `taste` is the local ranking, the rest pass through to VNDB. */
const SORTS = [
  ["taste", "recommend.foryou.sortTaste"],
  ["rating", "recommend.foryou.sortRating"],
  ["votes", "recommend.foryou.sortVotes"],
  ["released", "recommend.foryou.sortReleased"],
  ["title", "recommend.foryou.sortTitle"],
] as const;

/** Release-date windows, in years back from today. */
const RELEASED = [
  ["", "recommend.foryou.releasedAny"],
  ["1", "recommend.foryou.released1"],
  ["5", "recommend.foryou.released5"],
  ["10", "recommend.foryou.released10"],
] as const;

interface Filters {
  length: number;
  ratingMin: number;
  votesMin: number;
  released: (typeof RELEASED)[number][0];
  language: string;
  sort: ForYouSort;
}

const DEFAULT_FILTERS: Filters = {
  length: 0,
  ratingMin: 0,
  votesMin: 0,
  released: "",
  language: "",
  sort: "taste",
};

function releasedAfter(range: Filters["released"]): string | undefined {
  if (range === "") return undefined;
  const cutoff = new Date();
  cutoff.setFullYear(cutoff.getFullYear() - Number(range));
  return cutoff.toISOString().slice(0, 10);
}

function buildRequest(
  source: ForYouSource,
  filters: Filters,
  page: number,
): ForYouRequest {
  // Built conditionally rather than with `undefined` holes: the request type
  // keeps its optionals optional under exactOptionalPropertyTypes.
  const request: ForYouRequest = { source, page, sort: filters.sort };
  if (filters.length) request.length = filters.length;
  if (filters.ratingMin) request.ratingMin = filters.ratingMin;
  if (filters.votesMin) request.votesMin = filters.votesMin;
  const after = releasedAfter(filters.released);
  if (after) request.releasedAfter = after;
  if (filters.language) request.language = filters.language;
  return request;
}

export function ForYouPanel() {
  const { t } = useI18n();
  const nsfw = useAppStore((state) => state.prefs.nsfw);
  const [profile, setProfile] = useState<ForYouProfile | null>(null);
  const [source, setSource] = useState<ForYouSource>("combined");
  const [filters, setFilters] = useState<Filters>(DEFAULT_FILTERS);
  const [items, setItems] = useState<ForYouCard[]>([]);
  const [more, setMore] = useState(false);
  const [fromCache, setFromCache] = useState(false);
  const [page, setPage] = useState(1);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);

  // Deferred, like the ask tab's settings fetch: state changes inside the
  // awaited closure, never synchronously in the effect body.
  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const next = await api.forYouProfile();
        if (active) setProfile(next);
      } catch (caught) {
        if (active) {
          setError(caught instanceof IpcError ? caught.message : t("common.error"));
        }
      }
    })();
    return () => {
      active = false;
    };
  }, [t]);

  async function refresh() {
    try {
      const next = await api.forYouProfile();
      setProfile(next);
      if (next.ready) setRefreshTick((count) => count + 1);
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    }
  }

  useEffect(() => {
    if (!profile?.ready) return;
    let active = true;
    void (async () => {
      setBusy(true);
      setError(null);
      setNote(null);
      try {
        const result = await api.forYouSearch(buildRequest(source, filters, 1));
        if (!active) return;
        setItems(result.items);
        setMore(result.more);
        setFromCache(result.fromCache);
        setNote(result.note);
        setPage(1);
      } catch (caught) {
        if (active) {
          setError(caught instanceof IpcError ? caught.message : t("common.error"));
        }
      } finally {
        if (active) setBusy(false);
      }
    })();
    return () => {
      active = false;
    };
  }, [profile?.ready, source, filters, refreshTick, t]);

  async function loadMore() {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const next = await api.forYouSearch(buildRequest(source, filters, page + 1));
      setItems((current) => [...current, ...next.items]);
      setMore(next.more);
      setFromCache(next.fromCache);
      setPage(page + 1);
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  }

  // VNDB urls come from the API response, so they go through Rust's scheme
  // allowlist like every third-party link, never an href the webview can hit.
  async function openOnVndb(vnId: string) {
    try {
      await api.openExternal(`https://vndb.org/${vnId}`);
    } catch {
      // A rejected link simply does nothing, like the citation links beside.
    }
  }

  if (!profile) {
    return <p className="settings__hint">{t("common.loading")}</p>;
  }

  if (!profile.ready) {
    return (
      <section className="foryou" aria-busy={busy}>
        <header className="foryou__head">
          <h2 className="foryou__title">{t("recommend.foryou.title")}</h2>
          <p className="page-sub">
            {t("recommend.foryou.subtitle", { count: profile.considered })}
          </p>
        </header>
        <section className="discover__empty">
          <Icon name="spark" size={22} />
          <h3>{t("recommend.foryou.emptyTitle")}</h3>
          <p>{t("recommend.foryou.emptyBody")}</p>
        </section>
      </section>
    );
  }

  return (
    <section className="foryou" aria-busy={busy}>
      <header className="foryou__head">
        <div>
          <h2 className="foryou__title">{t("recommend.foryou.title")}</h2>
          <p className="page-sub">
            {t("recommend.foryou.subtitle", { count: profile.considered })}
          </p>
        </div>
        <Button variant="ghost" onClick={() => void refresh()}>
          <Icon name="sync" size={14} />
          {t("recommend.foryou.refresh")}
        </Button>
      </header>

      {profile.tags.length > 0 && (
        <div className="foryou__taste">
          <span className="foryou__taste-label">{t("recommend.foryou.taste")}</span>
          {profile.tags.map((tag) => (
            <Chip key={tag.id}>
              {tag.name}
              <span className="foryou__taste-count">{tag.count}</span>
            </Chip>
          ))}
        </div>
      )}

      <div className="foryou__filters">
        <label className="field" style={{ gap: 4 }}>
          <span className="foryou__filter-label">{t("recommend.foryou.length")}</span>
          <select
            className="select"
            value={filters.length}
            onChange={(event) =>
              setFilters({ ...filters, length: Number(event.target.value) })
            }
          >
            {LENGTHS.map(([value, key]) => (
              <option key={value} value={value}>
                {t(key)}
              </option>
            ))}
          </select>
        </label>
        <label className="field" style={{ gap: 4 }}>
          <span className="foryou__filter-label">{t("recommend.foryou.rating")}</span>
          <select
            className="select"
            value={filters.ratingMin}
            onChange={(event) =>
              setFilters({ ...filters, ratingMin: Number(event.target.value) })
            }
          >
            {RATINGS.map(([value, key]) => (
              <option key={value} value={value}>
                {t(key)}
              </option>
            ))}
          </select>
        </label>
        <label className="field" style={{ gap: 4 }}>
          <span className="foryou__filter-label">{t("recommend.foryou.released")}</span>
          <select
            className="select"
            value={filters.released}
            onChange={(event) =>
              setFilters({ ...filters, released: event.target.value as Filters["released"] })
            }
          >
            {RELEASED.map(([value, key]) => (
              <option key={value} value={value}>
                {t(key)}
              </option>
            ))}
          </select>
        </label>
        <label className="field" style={{ gap: 4 }}>
          <span className="foryou__filter-label">{t("recommend.foryou.language")}</span>
          <select
            className="select"
            value={filters.language}
            onChange={(event) =>
              setFilters({ ...filters, language: event.target.value })
            }
          >
            {LANGUAGES.map(([value, key]) => (
              <option key={value} value={value}>
                {t(key)}
              </option>
            ))}
          </select>
        </label>
        <label className="field" style={{ gap: 4 }}>
          <span className="foryou__filter-label">{t("recommend.foryou.popularity")}</span>
          <select
            className="select"
            value={filters.votesMin}
            onChange={(event) =>
              setFilters({ ...filters, votesMin: Number(event.target.value) })
            }
          >
            {VOTES.map(([value, key]) => (
              <option key={value} value={value}>
                {t(key)}
              </option>
            ))}
          </select>
        </label>
        <label className="field" style={{ gap: 4 }}>
          <span className="foryou__filter-label">{t("recommend.foryou.sort")}</span>
          <select
            className="select"
            value={filters.sort}
            onChange={(event) =>
              setFilters({ ...filters, sort: event.target.value as ForYouSort })
            }
          >
            {SORTS.map(([value, key]) => (
              <option key={value} value={value}>
                {t(key)}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="tabs" role="tablist" aria-label={t("recommend.foryou.title")}>
        {SOURCE_TABS.map(([value, key]) => (
          <button
            key={value}
            type="button"
            role="tab"
            aria-selected={source === value}
            className="tabs__tab"
            onClick={() => setSource(value)}
          >
            {t(key)}
          </button>
        ))}
      </div>

      {error && (
        <p className="notice notice--danger" role="alert">
          {error}
        </p>
      )}

      {fromCache && (
        <p className="notice">{t("recommend.foryou.fromCache")}</p>
      )}

      {note === "seeds" && <p className="notice">{t("recommend.foryou.noteSeeds")}</p>}
      {note === "source" && <p className="notice">{t("recommend.foryou.noteSource")}</p>}

      {busy && items.length === 0 && (
        <section className="discover__busy" aria-live="polite">
          <p role="status">{t("recommend.foryou.searching")}</p>
          <div className="skeleton skeleton--row" />
          <div className="skeleton skeleton--row" />
          <div className="skeleton skeleton--row" />
        </section>
      )}

      {!busy && items.length === 0 && !error && note === null && (
        <p className="notice">{t("recommend.foryou.noMatches")}</p>
      )}

      {items.length > 0 && (
        <ol className="foryou__list">
          {items.map((card, index) => (
            <ForYouCardView
              key={card.vnId}
              card={card}
              rank={index + 1}
              nsfw={nsfw}
              onOpen={() => void openOnVndb(card.vnId)}
            />
          ))}
        </ol>
      )}

      {more && (
        <Button variant="ghost" onClick={() => void loadMore()} disabled={busy}>
          {busy ? t("recommend.foryou.searching") : t("recommend.foryou.more")}
        </Button>
      )}
    </section>
  );
}

interface CardProps {
  card: ForYouCard;
  rank: number;
  nsfw: string;
  onOpen: () => void;
}

function ForYouCardView({ card, rank, nsfw, onOpen }: CardProps) {
  const { t } = useI18n();
  const hidden = isExplicitHit(card, nsfw);

  const meta: string[] = [];
  if (card.rating !== null) meta.push((card.rating / 10).toFixed(1));
  if (card.released) meta.push(card.released.slice(0, 4));
  if (card.developers.length > 0) meta.push(card.developers.join(", "));
  if (card.lengthMinutes !== null && card.lengthMinutes > 0) {
    meta.push(
      t("recommend.foryou.lengthHours", {
        hours: Math.round(card.lengthMinutes / 60),
      }),
    );
  }
  if (card.devstatus === 1) meta.push(t("recommend.foryou.inDevelopment"));

  return (
    <li className="foryou-card">
      <span className="foryou-card__rank" aria-hidden>
        {rank}
      </span>
      <div className="foryou-card__art">
        {hidden || card.imageUrl === null ? (
          <div className="foryou-card__art-fallback" aria-hidden>
            <Icon name="image" size={20} />
          </div>
        ) : (
          <img src={card.imageUrl} alt="" loading="lazy" />
        )}
      </div>
      <div className="foryou-card__body">
        <div className="foryou-card__head">
          <h3 className="foryou-card__title">{card.title}</h3>
          <button
            type="button"
            className="foryou-card__open"
            onClick={onOpen}
            aria-label={t("recommend.foryou.openVndb", { title: card.title })}
          >
            <Icon name="external" size={14} />
          </button>
        </div>
        {meta.length > 0 && <p className="foryou-card__meta">{meta.join(" · ")}</p>}
        {card.because && (
          <p className="foryou-card__because">
            {t("recommend.foryou.because", { title: card.because })}
          </p>
        )}
        {card.creators.length > 0 && (
          <p className="foryou-card__match">
            {t("recommend.foryou.creators", { names: card.creators.join(", ") })}
          </p>
        )}
        {card.themes.length > 0 && (
          <p className="foryou-card__match">
            {t("recommend.foryou.themes", { names: card.themes.join(", ") })}
          </p>
        )}
        {card.voices.length > 0 && (
          <p className="foryou-card__match">
            {t("recommend.foryou.voicesLiked", { names: card.voices.join(", ") })}
          </p>
        )}
      </div>
    </li>
  );
}
