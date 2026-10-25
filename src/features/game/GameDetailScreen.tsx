import { useEffect, useState } from "react";
import { Link, useParams } from "react-router";
import { open } from "@tauri-apps/plugin-dialog";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { MapExecutable } from "@/features/tracking/MapExecutable";
import { PlayButton } from "@/features/game/PlayButton";
import { alternateTitles } from "@/features/game/alternateTitles";
import { RatingPicker } from "@/features/game/RatingPicker";
import { ScreenshotGallery } from "@/features/game/ScreenshotGallery";
import { CastCard } from "@/features/game/CastCard";
import { VndbFacts } from "@/features/game/VndbFacts";
import { vndbVnUrl } from "@/features/game/vndbMetadata";
import { Card, CardHeader } from "@/components/Card";
import { RoutesCard } from "@/features/game/RoutesCard";
import { ScriptCard } from "@/features/game/ScriptCard";
import { Chip, StatusChip } from "@/components/Chip";
import { EmptyState } from "@/components/EmptyState";
import { Icon } from "@/components/Icon";
import { ProgressRing } from "@/components/ProgressRing";
import { Sheet } from "@/components/Sheet";
import { useArtwork } from "@/components/useArtwork";
import { api, IpcError } from "@/lib/ipc/client";
import { statusOrder } from "@/lib/library";
import { fallbackPalette } from "@/lib/palette";
import { stripFormatting } from "@/lib/vndbText";
import {
  formatDateTime,
  formatDurationLong,
  localDateTimeToMs,
  lengthProgress,
} from "@/lib/time";
import type {
  DetailPayload,
  Game,
  GameCard,
  GameExecutable,
  GameStatus,
  PlaySession,
} from "@/lib/ipc/types";

/**
 * Stand-in card for the artwork policy hook while the real one is loading.
 *
 * Hooks cannot run conditionally, and the policy reads the game's content
 * ratings, so this has to exist before the fetch resolves. It carries no art and
 * no ratings, which means it can only ever resolve to "visible".
 */
const placeholderGame: Game = {
  meta: { id: "", updated_at: 0, device_id_device: "", deleted: false },
  title: "",
  title_original: null,
  vndb_id: null,
  status: "wishlist",
  rating: null,
  notes: null,
  length_minutes: null,
  manual_playtime_ms: 0,
  accent_color: null,
  cover_url: null,
  cover_sexual: null,
  cover_violence: null,
  custom_cover_path: null,
  custom_cover_source_vndb: false,
  started_at: null,
  finished_at: null,
};

/**
 * Game detail.
 *
 * Composition differs from the library on purpose (RHYTHM 3): an asymmetric hero
 * with the cover as a single sharp object, then a two-column reading layout. The
 * cover is the only image at full sharpness in the app, because this is the one
 * screen where the artwork is the subject rather than the backdrop.
 *
 * "Full sharpness" is still subject to the content setting: an explicit cover is
 * blurred or replaced until the user asks to see it.
 */
/**
 * The other names this VN goes by, in the order they are worth reading.
 *
 * Every entry is filtered against the main title and against the names already
 * kept, because VNDB repeats itself freely: an English release often has
 * `title === latin === alttitle`, and printing those three identical strings
 * under the heading is what made the old header look broken.
 *
 * Only Japanese and English are kept. VNDB also publishes Russian, Chinese,
 * Korean, German and more, and a row of five scripts under the title is noise for
 * a reader who wants to match a name rather than survey localisations. `ja` is
 * kept because it is what the cover and the save files say; `en` because it is
 * what most people search for.
 *
 * Aliases come last and are not titles: they are the short names people use
 * ("When They Cry 3" for Umineko), carry no language, and are shown as-is.
 *
 * The primary title is whatever the library row stores, so the caller passes it
 * in rather than this guessing.
 */
// Mirrors `vn_core::model::MAX_NOTES_CHARS`: the backend rejects a longer note.
const MAX_NOTES_CHARS = 20_000;

export function GameDetailScreen() {
  const { id } = useParams<{ id: string }>();
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const upsertCard = useAppStore((state) => state.upsertCard);
  const removeCard = useAppStore((state) => state.removeCard);
  const isAndroid = useAppStore((state) => state.platform?.os === "android");
  // The library already holds every card, so returning to a game can paint
  // immediately instead of blanking to a skeleton and re-fetching.
  const cachedCard = useAppStore((state) =>
    state.cards.find((entry) => entry.game.meta.id === id),
  );
  // Read from the store, not local state: the page unmounts on navigation, so a
  // refresh started before leaving kept running while its "in progress" flag died
  // with the component — coming back showed "Refresh" over a live request.
  const refreshingVndb = useAppStore((state) =>
    id === undefined ? false : state.refreshingVndb.includes(id),
  );
  const setRefreshingVndb = useAppStore((state) => state.setRefreshingVndb);
  const cachedDetail = useAppStore((state) =>
    cachedCard?.game.vndb_id ? state.vndbDetails[cachedCard.game.vndb_id] : undefined,
  );
  const cacheVndbDetail = useAppStore((state) => state.cacheVndbDetail);

  const [card, setCard] = useState<GameCard | null>(cachedCard ?? null);
  const [sessions, setSessions] = useState<PlaySession[]>([]);
  const [executables, setExecutables] = useState<GameExecutable[]>([]);
  // Seed from the store so a revisit shows the payload it already fetched
  // instead of re-requesting an unchanged row.
  const [vndb, setVndb] = useState<DetailPayload | null>(cachedDetail ?? null);
  // Bumped whenever the payload is replaced. The screenshot gallery takes its list
  // from the stored payload, so it has no other way to learn that a refresh changed
  // which images exist.
  const [vndbVersion, setVndbVersion] = useState(0);

  // A cached card means there is already something to show: starting in the loading
  // state would flash a skeleton over data the user was just looking at.
  const [loading, setLoading] = useState(cachedCard === undefined);
  const [notFound, setNotFound] = useState(false);
  const [notes, setNotes] = useState("");
  const [savingNotes, setSavingNotes] = useState(false);
  const [sessionOpen, setSessionOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);

  useEffect(() => {
    if (!id) return undefined;
    let cancelled = false;

    const load = async (): Promise<void> => {
      try {
        const [loaded, loadedSessions, loadedExes] = await Promise.all([
          api.getGame(id),
          api.listSessions(id).catch(() => []),
          api.trackingListExecutables().catch(() => []),
        ]);
        if (cancelled) return;
        if (!loaded) {
          // Clearing the card matters: the state is seeded from the cached card, so
          // setting only `notFound` left a deleted game rendering a fully
          // interactive page whose every write targeted a tombstone.
          setNotFound(true);
          setCard(null);
          setNotes("");
        } else {
          setNotFound(false);
          setCard(loaded);
          setNotes(loaded.game.notes ?? "");
        }
        setSessions(loadedSessions);
        setExecutables(loadedExes.filter((entry) => entry.game_id === id));

        // VNDB extras are strictly optional: a missing detail row just hides the
        // section rather than blocking the page.
        const vnId = loaded?.game.vndb_id;
        if (vnId) {
          void api
            .vndbDetail(vnId)
            .then((detail) => {
              if (cancelled) return;
              setVndb(detail);
              cacheVndbDetail(vnId, detail);
            })
            .catch(() => undefined);
        }
      } catch (caught) {
        if (!cancelled) {
          pushToast(
            "error",
            caught instanceof IpcError ? caught.message : t("common.error"),
          );
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    void load();
    return () => {
      cancelled = true;
    };
  }, [id, pushToast, t, cacheVndbDetail]);

  useEffect(() => {
    if (!isAndroid || !id) return;
    let cancelled = false;
    const refreshAfterPlay = async (): Promise<void> => {
      if (document.visibilityState === "hidden") return;
      try {
        await api.trackingAndroidSession();
        const [nextCard, nextSessions] = await Promise.all([
          api.getGame(id),
          api.listSessions(id),
        ]);
        if (cancelled) return;
        if (nextCard) {
          setCard(nextCard);
          upsertCard(nextCard);
        }
        setSessions(nextSessions);
      } catch {
        // Preserve the last known game while the tracking screen reports errors.
      }
    };
    const onReturn = (): void => {
      void refreshAfterPlay();
    };
    window.addEventListener("focus", onReturn);
    document.addEventListener("visibilitychange", onReturn);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", onReturn);
      document.removeEventListener("visibilitychange", onReturn);
    };
  }, [isAndroid, id, upsertCard]);

  // Called before the loading and not-found returns: hooks cannot be conditional,
  // and the placeholder card keeps the artwork policy stable across those states.
  const art = useArtwork(card?.game ?? placeholderGame, { preferRemote: true });
  const concealed = art.policy !== "visible";

  if (loading) {
    return (
      <div aria-busy="true" aria-label={t("common.loading")}>
        <div className="detail__hero">
          <div className="skeleton" style={{ aspectRatio: "2 / 3" }} />
          <div style={{ display: "grid", gap: 12, alignContent: "end" }}>
            <div className="skeleton" style={{ height: 28, width: "60%" }} />
            <div className="skeleton" style={{ height: 14, width: "40%" }} />
          </div>
        </div>
      </div>
    );
  }

  if (notFound || !card) {
    return (
      <EmptyState
        title={t("detail.notFound")}
        body={t("library.emptyBody")}
        action={
          <Link className="btn btn--secondary" to="/">
            <Icon name="chevronLeft" />
            {t("detail.back")}
          </Link>
        }
      />
    );
  }

  const { game, total_ms: totalMs, afk_ms: afkMs } = card;
  const fallback = fallbackPalette(game.title);
  const progress = lengthProgress(totalMs, game.length_minutes);
  // Metadata was frozen at import time: nothing in the app could re-fetch it, so a
  // cached row never learned about fields added later. This re-reads VNDB and
  // leaves the game row (status, rating, notes, playtime) untouched.
  const refreshVndb = async (): Promise<void> => {
    const gameId = game.meta.id;
    setRefreshingVndb(gameId, true);
    try {
      const detail = await api.vndbRefreshDetail(gameId);
      // The page may have unmounted while this was in flight; the store outlives
      // it, so the result still has to land somewhere the next visit can read.
      setVndb(detail);
      setVndbVersion((version) => version + 1);
      if (detail.vn_id) cacheVndbDetail(detail.vn_id, detail);
      pushToast("success", t("detail.refreshed"));
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      // Runs against the store, so it is not a no-op on an unmounted component.
      setRefreshingVndb(gameId, false);
    }
  };

  const otherTitles = alternateTitles(
    game.title,
    game.title_original,
    vndb?.titles ?? [],
    vndb?.aliases ?? [],
  );

  // Reports whether the write landed. A caller that shows its own success message
  // has to know: the notes save used to show a green "saved" next to the red error
  // for the same failed write.
  const updateGame = async (
    patch: Parameters<typeof api.updateGame>[0],
  ): Promise<boolean> => {
    try {
      await api.updateGame(patch);
      const refreshed = await api.getGame(game.meta.id);
      if (refreshed) {
        setCard(refreshed);
        upsertCard(refreshed);
      }
      return true;
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
      return false;
    }
  };

  const saveNotes = async (): Promise<void> => {
    // One save in flight at a time: blur fires on every tab-out, and two writes
    // racing would leave whichever finished last on screen.
    if (savingNotes) return;
    setSavingNotes(true);
    try {
      const saved = await updateGame({
        id: game.meta.id,
        notes: notes.length > 0 ? notes : null,
      });
      if (saved) pushToast("success", t("detail.notes"));
    } finally {
      setSavingNotes(false);
    }
  };

  // The picker returns a path string (a content:// URI on Android, which the
  // Rust side resolves through the content resolver), and the bytes are
  // re-decoded before acceptance: a renamed non-image must fail here with a
  // message rather than become a silently broken cover.
  const pickCustomCover = async (): Promise<void> => {
    try {
      const chosen = await open({
        multiple: false,
        directory: false,
        title: t("detail.coverPickTitle"),
        filters: [
          {
            name: t("detail.coverPickFilter"),
            extensions: ["jpg", "jpeg", "png", "webp", "avif", "gif", "bmp"],
          },
        ],
      });
      if (typeof chosen !== "string") return;
      await api.setGameCustomCover(game.meta.id, chosen);
      await refreshCard();
      pushToast("success", t("detail.coverSaved"));
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    }
  };

  const clearCustomCover = async (): Promise<void> => {
    try {
      await api.clearGameCustomCover(game.meta.id);
      await refreshCard();
      pushToast("success", t("detail.coverCleared"));
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    }
  };

  const refreshCard = async (): Promise<void> => {
    const refreshed = await api.getGame(game.meta.id);
    if (refreshed) {
      setCard(refreshed);
      upsertCard(refreshed);
    }
  };

  return (
    <>
      <Link
        to="/"
        className="btn btn--ghost"
        style={{ marginBottom: "var(--space-2)", paddingInline: 0 }}
      >
        <Icon name="chevronLeft" />
        {t("detail.back")}
      </Link>

      <header className="detail__hero">
        <div className="detail__art">
          {art.imageUrl ? (
            <img
              className={concealed ? "cover__art--blurred" : undefined}
              src={art.imageUrl}
              alt={concealed ? "" : t("game.coverAlt", { title: game.title })}
              aria-hidden={concealed ? "true" : undefined}
              style={{ width: "100%", height: "100%", objectFit: "cover" }}
            />
          ) : (
            <div
              className="cover__placeholder"
              style={{
                background: `linear-gradient(160deg, ${fallback.accent}44, var(--bg-elevated) 72%)`,
                height: "100%",
              }}
            >
              {concealed ? "" : game.title}
            </div>
          )}

          {/* The hero is the app's one sharp artwork. It obeys the same content
              setting as the library instead of being the single screen that
              leaks the cover. */}
          {concealed ? (
            <div className="detail__art-cover">
              <Icon name="eyeOff" size={20} />
              <span>{t("game.explicitHidden")}</span>
              <Button variant="secondary" onClick={art.reveal}>
                {t("game.reveal")}
              </Button>
            </div>
          ) : null}

          {/* The cover control belongs on the artwork itself rather than in a
              settings panel: a game with no VNDB entry shows a blank placeholder,
              and the place a user looks to fix a missing picture is the picture. */}
          {!concealed ? (
            <div className="detail__art-edit">
              <button
                type="button"
                className="detail__art-edit-btn"
                onClick={() => void pickCustomCover()}
                title={
                  game.custom_cover_path ? t("detail.coverChange") : t("detail.coverAdd")
                }
              >
                <Icon name="image" size={14} />
                {game.custom_cover_path ? t("detail.coverChange") : t("detail.coverAdd")}
              </button>
              {game.custom_cover_path ? (
                <button
                  type="button"
                  className="detail__art-edit-btn detail__art-edit-btn--quiet"
                  onClick={() => void clearCustomCover()}
                  title={t("detail.coverRemove")}
                >
                  <Icon name="trash" size={14} />
                  {t("detail.coverRemove")}
                </button>
              ) : null}
            </div>
          ) : null}
        </div>

        <div className="detail__head">
          <h2 className="detail__title over-art">{game.title}</h2>
          {/* Every other name the VN has, minus the one above. Previously this
              showed `title_original` alone, which for KANADE is the same string as
              the title, so the header printed the name twice and said nothing
              about the Latin or English release names. */}
          {otherTitles.length > 0 ? (
            <ul className="detail__alts" aria-label={t("detail.alsoKnownAs")}>
              {otherTitles.map((title) => (
                <li key={title} className="detail__alt">
                  {title}
                </li>
              ))}
            </ul>
          ) : null}

          <div className="detail__facts">
            <StatusChip status={game.status} />
            <RatingPicker
              value={game.rating}
              onChange={(rating) => void updateGame({ id: game.meta.id, rating })}
            />
            {/* The community score, next to the user's own: the pair is the
                interesting thing, and it was only visible at the bottom of the
                VNDB card. */}
            {vndb?.rating ? (
              <span
                className="chip tnum"
                title={t("detail.vndbRatingHint", {
                  count: vndb.votecount ?? 0,
                })}
              >
                <Icon name="star" size={12} />
                {t("detail.vndbRating", {
                  value: (vndb.rating / 10).toFixed(1),
                })}
              </span>
            ) : null}
            {/* The link to the entry itself, next to the score it came from. The
                facts card below repeats it because that is where a reader looks
                for "more", but a VN page is something people want one click away
                from the hero, which is where they decide to go. */}
            {game.vndb_id ? (
              <button
                type="button"
                className="chip chip--link tnum"
                title={t("detail.viewOnVndbHint", { vn: game.vndb_id })}
                onClick={() => {
                  void api
                    .openExternal(vndbVnUrl(game.vndb_id ?? ""))
                    .catch((caught: unknown) => {
                      pushToast(
                        "error",
                        caught instanceof IpcError ? caught.message : t("common.error"),
                      );
                    });
                }}
              >
                {game.vndb_id}
              </button>
            ) : null}
            <span className="chip tnum">
              <Icon name="clock" size={12} />
              {formatDurationLong(totalMs, locale)}
            </span>
            {afkMs > 0 ? (
              <span className="chip tnum" title={t("game.afkHint")}>
                {t("detail.away")}: {formatDurationLong(afkMs, locale)}
              </span>
            ) : null}
          </div>

          {/* One right-aligned column: secondary action, progress, primary action.
              Inline styles put the ring on its own line with an ad-hoc flex, which
              left "Discuss this VN" and the percentage floating at different
              widths and made the group read as scattered rather than stacked. */}
          <div className="detail__actions">
            <div className="detail__actions-row">
              <Link
                className="btn btn--secondary"
                to={`/chat?game=${encodeURIComponent(game.meta.id)}`}
              >
                {t("vnchat.askGame")}
              </Link>
              {progress !== null ? (
                <span className="detail__progress caption">
                  <ProgressRing
                    progress={progress}
                    size={22}
                    title={t("game.progress", {
                      percent: Math.round(progress * 100),
                    })}
                  />
                  {t("game.progress", { percent: Math.round(progress * 100) })}
                </span>
              ) : null}
            </div>
            <PlayButton gameId={game.meta.id} mappedExecutable={executables[0] ?? null} />
          </div>

          <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
            {statusOrder.map((status: GameStatus) => (
              <Chip
                key={status}
                selected={game.status === status}
                onClick={() => void updateGame({ id: game.meta.id, status })}
              >
                {t(`status.${status}`)}
              </Chip>
            ))}
          </div>
        </div>
      </header>

      <div className="detail__columns">
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)" }}>
          <Card>
            <CardHeader title={t("detail.notes")} />
            <label className="visually-hidden" htmlFor="notes">
              {t("detail.notes")}
            </label>
            <textarea
              id="notes"
              className="textarea"
              value={notes}
              // Rust rejects anything past MAX_NOTES_CHARS, so the limit is visible
              // here instead of arriving as a failed save after a paste.
              maxLength={MAX_NOTES_CHARS}
              placeholder={t("detail.notesPlaceholder")}
              onChange={(event) => setNotes(event.target.value)}
              onBlur={() => void saveNotes()}
            />
          </Card>

          {/* The facts come before the description: a reader deciding whether this
              is the VN they are thinking of needs the date, the developer and the
              platforms, not 2,000 words of synopsis. */}
          {vndb && game.vndb_id ? (
            <>
              <VndbFacts payload={vndb} vnId={game.vndb_id} playtimeMs={totalMs} />
              <CastCard characters={vndb.characters} />
            </>
          ) : null}

          {/* The description is prose and belongs at a reading width, not in a
              narrow side rail where it ran taller than the whole page. */}
          {vndb ? (
            <Card>
              <CardHeader
                title={t("detail.fromVndb")}
                aside={
                  <Button
                    variant="ghost"
                    disabled={refreshingVndb}
                    onClick={() => {
                      void refreshVndb();
                    }}
                  >
                    {refreshingVndb ? t("detail.refreshing") : t("detail.refreshVndb")}
                  </Button>
                }
              />
              {vndb.description ? (
                <p className="body" style={{ marginBottom: "var(--space-3)" }}>
                  {stripFormatting(vndb.description)}
                </p>
              ) : null}
              {vndb.tags.length > 0 ? (
                <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
                  {vndb.tags
                    .filter((tag) => (tag.spoiler ?? 0) === 0)
                    .slice(0, 12)
                    .map((tag) => (
                      <span className="chip" key={tag.id ?? tag.name}>
                        {tag.name}
                      </span>
                    ))}
                </div>
              ) : null}
              {vndb.votecount ? (
                <p className="caption" style={{ marginTop: "var(--space-3)" }}>
                  {t("detail.vndbVotes", { count: vndb.votecount })}
                </p>
              ) : null}
            </Card>
          ) : null}

          {/* Screenshots sit after the prose. They are the reason someone scrolls
              this far, and putting them above the description would push the text
              off the first screen for a VN with thirty of them. The gallery caches
              on mount, so nothing downloads until this point is reached. */}
          {game.vndb_id ? (
            <ScreenshotGallery
              vnId={game.vndb_id}
              title={game.title}
              // A refresh changes the payload, not the VN id, so the gallery needs
              // its own signal to re-read the list it was built from.
              version={vndbVersion}
            />
          ) : null}

          {id ? (
            <Card>
              <CardHeader title={t("detail.executable")} />
              {executables.length > 0 ? (
                <ul className="executable-list">
                  {executables.map((entry) => (
                    <li key={entry.id} className="tracking__row tracking__row--divided">
                      <Icon name="play" size={14} />
                      <span style={{ minWidth: 0, flex: 1 }}>
                        <span className="row__title" title={entry.exe_path ?? entry.exe_name}>
                          {entry.exe_path ?? entry.exe_name}
                          {entry.runner_kind ? (
                            <span
                              className="chip"
                              style={{
                                fontSize: "var(--font-size-xs)",
                                padding: "2px 6px",
                                marginLeft: "var(--space-2)",
                              }}
                            >
                              {entry.runner_kind.toUpperCase()}
                            </span>
                          ) : null}
                        </span>
                        {entry.launch_target || entry.title_match ? (
                          <span className="row__sub">
                            {entry.launch_target ?? ""}
                            {entry.title_match ? ` [title: ${entry.title_match}]` : ""}
                          </span>
                        ) : null}
                      </span>
                      <Button
                        variant="ghost"
                        onClick={() => {
                          void (async () => {
                            try {
                              await api.trackingRemoveExecutable(entry.id);
                              const allExes = await api.trackingListExecutables();
                              setExecutables(
                                allExes.filter((item) => item.game_id === id),
                              );
                            } catch (caught) {
                              pushToast(
                                "error",
                                caught instanceof IpcError
                                  ? caught.message
                                  : t("common.error"),
                              );
                            }
                          })();
                        }}
                      >
                        {t("tracking.remove")}
                      </Button>
                    </li>
                  ))}
                </ul>
              ) : null}
              <MapExecutable
                games={[]}
                onMapped={async () => {
                  const allExes = await api.trackingListExecutables().catch(() => []);
                  setExecutables(allExes.filter((entry) => entry.game_id === id));
                  pushToast("success", t("detail.executableMapped"));
                }}
                fixedGameId={id}
              />
            </Card>
          ) : null}

          {id ? <RoutesCard gameId={id} onAddTime={() => setSessionOpen(true)} /> : null}

          {/* The script card sits with the routes: both are per-game material
              for the companion chat, and neither is required to play. */}
          {id ? <ScriptCard gameId={id} /> : null}

          <Card>
            <CardHeader
              title={t("detail.sessions")}
              aside={
                <span className="caption tnum">
                  {formatDurationLong(totalMs, locale)}
                </span>
              }
            />
            {sessions.length === 0 ? (
              <p className="body">{t("detail.sessionsEmpty")}</p>
            ) : (
              <ul className="stats__rows">
                {sessions.map((session) => (
                  <li
                    key={session.meta.id}
                    style={{
                      display: "flex",
                      justifyContent: "space-between",
                      gap: 12,
                      padding: "6px 0",
                      borderBottom: "1px solid var(--border)",
                    }}
                  >
                    <span className="caption">
                      {formatDateTime(session.started_at, locale)}
                    </span>
                    <span className="tnum" style={{ fontSize: "var(--text-s)" }}>
                      {formatDurationLong(session.counted_ms, locale)}
                      {session.afk_ms > 0 ? (
                        <span
                          className="caption"
                          style={{ marginLeft: 8 }}
                          title={t("detail.away")}
                        >
                          +{formatDurationLong(session.afk_ms, locale)}
                        </span>
                      ) : null}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Card>
        </div>

        <div className="detail__aside">
          <Button variant="danger" onClick={() => setDeleteOpen(true)}>
            {t("detail.delete")}
          </Button>
        </div>
      </div>

      <ManualSessionSheet
        open={sessionOpen}
        gameId={game.meta.id}
        onClose={() => setSessionOpen(false)}
        onSaved={async () => {
          const refreshed = await api.getGame(game.meta.id);
          if (refreshed) {
            setCard(refreshed);
            upsertCard(refreshed);
          }
          setSessions(await api.listSessions(game.meta.id));
        }}
      />

      <Sheet
        open={deleteOpen}
        title={t("detail.delete")}
        onClose={() => setDeleteOpen(false)}
        footer={
          <>
            <Button variant="ghost" onClick={() => setDeleteOpen(false)}>
              {t("detail.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                void (async () => {
                  try {
                    await api.deleteGame(game.meta.id);
                    removeCard(game.meta.id);
                    // The row is gone, so this screen has nothing left to show even
                    // if the navigation does not unmount it.
                    setCard(null);
                    setNotFound(true);
                    window.history.back();
                  } catch (caught) {
                    pushToast(
                      "error",
                      caught instanceof IpcError ? caught.message : t("common.error"),
                    );
                  }
                })();
              }}
            >
              {t("detail.deleteYes")}
            </Button>
          </>
        }
      >
        <p className="body">{t("detail.deleteConfirm", { title: game.title })}</p>
      </Sheet>
    </>
  );
}

interface ManualSessionSheetProps {
  open: boolean;
  gameId: string;
  onClose: () => void;
  onSaved: () => Promise<void>;
}

/** Manual session entry. The only way time exists until tracking lands. */
function ManualSessionSheet({ open, gameId, onClose, onSaved }: ManualSessionSheetProps) {
  const { t } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const today = new Date().toISOString().slice(0, 10);
  const [date, setDate] = useState(today);
  const [start, setStart] = useState("20:00");
  const [end, setEnd] = useState("21:00");
  const [error, setError] = useState<string | null>(null);

  const save = async (): Promise<void> => {
    const startMs = localDateTimeToMs(date, start);
    const endMs = localDateTimeToMs(date, end);
    if (startMs === null || endMs === null) {
      setError(t("detail.saveFailed"));
      return;
    }
    // A session that crosses midnight ends on the next day.
    const adjustedEnd = endMs < startMs ? endMs + 86_400_000 : endMs;
    try {
      await api.addManualSession({
        gameId,
        startedAtMs: startMs,
        endedAtMs: adjustedEnd,
      });
      await onSaved();
      setError(null);
      onClose();
    } catch (caught) {
      const message =
        caught instanceof IpcError ? caught.message : t("detail.saveFailed");
      setError(message);
      pushToast("error", message);
    }
  };

  return (
    <Sheet
      open={open}
      title={t("detail.manualTime")}
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("detail.cancel")}
          </Button>
          <Button variant="primary" onClick={() => void save()}>
            {t("detail.save")}
          </Button>
        </>
      }
    >
      <div className="field">
        <label className="field__label" htmlFor="session-date">
          {t("detail.date")}
        </label>
        <input
          id="session-date"
          className="input"
          type="date"
          value={date}
          onChange={(event) => setDate(event.target.value)}
        />
      </div>
      <div style={{ display: "flex", gap: 12 }}>
        <div className="field" style={{ flex: 1 }}>
          <label className="field__label" htmlFor="session-start">
            {t("detail.startTime")}
          </label>
          <input
            id="session-start"
            className="input"
            type="time"
            value={start}
            onChange={(event) => setStart(event.target.value)}
          />
        </div>
        <div className="field" style={{ flex: 1 }}>
          <label className="field__label" htmlFor="session-end">
            {t("detail.endTime")}
          </label>
          <input
            id="session-end"
            className="input"
            type="time"
            value={end}
            onChange={(event) => setEnd(event.target.value)}
          />
        </div>
      </div>
      {error ? <p style={{ color: "var(--danger)", fontSize: 12 }}>{error}</p> : null}
    </Sheet>
  );
}
