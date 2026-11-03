import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { useArtwork } from "@/components/useArtwork";
import { api, IpcError } from "@/lib/ipc/client";
import { formatRating, statusOrder } from "@/lib/library";
import { formatDuration } from "@/lib/time";
import type { GameCard } from "@/lib/ipc/types";
import appIcon from "@/assets/vn-asyik.svg";

import { moveFor, nextIndex, readPad, type PadMove } from "./navigation";

/**
 * Big Picture: the library at television distance.
 *
 * The whole surface is one focused game plus a cover rail that can be walked with
 * arrows, WASD, or a gamepad, so a HTPC or a sofa keyboard never needs a mouse.
 * It is a route rather than an overlay, so leaving it is a normal navigation and
 * the browser back button works.
 *
 * Every action is also a real button: the keyboard path is the fast one, not the
 * only one.
 */
export function BigPictureScreen() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const cards = useAppStore((state) => state.cards);
  const pushToast = useAppStore((state) => state.pushToast);
  const [focus, setFocus] = useState(0);
  const [columns, setColumns] = useState(5);
  const [busy, setBusy] = useState(false);
  const launching = useRef(false);
  const keyboardFocus = useRef(false);
  const previousPad = useRef<{ move: PadMove; button: "a" | "b" | null }>({
    move: null,
    button: null,
  });
  const gridRef = useRef<HTMLDivElement>(null);
  const cardRefs = useRef<(HTMLButtonElement | null)[]>([]);

  /* Playing and paused games first: Big Picture is opened when the user wants to
   * continue something, and those are the ones worth reaching with one press. */
  const games = useMemo<GameCard[]>(() => {
    const weight = (status: string): number => {
      const index = statusOrder.indexOf(status as never);
      return index === -1 ? statusOrder.length : index;
    };
    return [...cards].sort(
      (left, right) =>
        weight(left.game.status) - weight(right.game.status) ||
        right.total_ms - left.total_ms ||
        left.game.title.localeCompare(right.game.title),
    );
  }, [cards]);

  const selectedIndex = Math.min(focus, Math.max(games.length - 1, 0));
  const focused = games[selectedIndex] ?? null;
  const pageStart = Math.floor(selectedIndex / columns) * columns;
  const visibleGames = games.slice(pageStart, pageStart + columns);

  const leave = useCallback(() => {
    // A fullscreen surface that leaves the window fullscreen traps the next screen.
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => undefined);
    navigate("/");
  }, [navigate]);

  const launch = useCallback(
    async (gameId = focused?.game.meta.id) => {
      if (!gameId || launching.current) return;
      launching.current = true;
      setBusy(true);
      try {
        const outcome = await api.launchGame(gameId);
        pushToast(outcome.started ? "success" : "info", outcome.explanation);
      } catch (caught) {
        pushToast(
          "error",
          caught instanceof IpcError ? caught.message : t("common.error"),
        );
      } finally {
        launching.current = false;
        setBusy(false);
      }
    },
    [focused, pushToast, t],
  );

  const openDetails = useCallback(() => {
    if (focused) navigate(`/game/${focused.game.meta.id}`);
  }, [focused, navigate]);

  // Grid width decides the row arithmetic. Measured rather than assumed: the same
  // screen runs on a 1080p TV and on a tablet.
  useEffect(() => {
    const element = gridRef.current;
    if (!element || typeof ResizeObserver === "undefined") return undefined;
    const measure = (): void => {
      const style = getComputedStyle(element);
      const template = style.gridTemplateColumns;
      const count = template.split(" ").filter((part) => part.trim() !== "").length;
      setColumns(Math.max(1, Math.min(12, count)));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [games.length]);

  // Each page is bounded. Transfer real keyboard focus after the new cover mounts.
  useEffect(() => {
    if (keyboardFocus.current) {
      cardRefs.current[selectedIndex]?.focus({ preventScroll: true });
      keyboardFocus.current = false;
    }
  }, [selectedIndex, columns]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey)
        return;
      const target = event.target instanceof Element ? event.target : null;
      if (target?.closest("input, textarea, select, [contenteditable=true]")) return;
      if (event.key === "Escape") {
        event.preventDefault();
        leave();
        return;
      }
      if (event.key === "Enter") {
        // Tabbed controls keep their native action (especially Exit and Details).
        if (target?.closest("button:not(.bp-card), a")) return;
        event.preventDefault();
        if (!event.repeat) void launch();
        return;
      }
      if (event.key === "e" || event.key === "E") {
        event.preventDefault();
        openDetails();
        return;
      }
      const move = moveFor(event.key);
      if (!move) return;
      event.preventDefault();
      keyboardFocus.current = true;
      setFocus((current) => nextIndex(move, current, games.length, columns));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [columns, games.length, launch, leave, openDetails]);

  /* Gamepad polling. One direction held is one move: the pad is read every frame,
   * and the previous direction is remembered so a thumb resting on the stick does
   * not walk the whole grid. */
  useEffect(() => {
    if (typeof navigator === "undefined" || !navigator.getGamepads) return undefined;
    let frame = 0;
    const tick = (): void => {
      const { move, held, button } = readPad(
        Array.from(navigator.getGamepads()),
        previousPad.current.move,
      );
      const pressed = button !== previousPad.current.button ? button : null;
      previousPad.current = { move: held, button };
      if (move && document.hasFocus() && document.visibilityState !== "hidden") {
        const intent =
          move === "left"
            ? { kind: "step" as const, delta: -1 }
            : move === "right"
              ? { kind: "step" as const, delta: 1 }
              : move === "up"
                ? { kind: "row" as const, delta: -1 }
                : { kind: "row" as const, delta: 1 };
        setFocus((current) => nextIndex(intent, current, games.length, columns));
      }
      if (document.hasFocus() && document.visibilityState !== "hidden") {
        if (pressed === "a") void launch();
        if (pressed === "b") leave();
      }
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [columns, games.length, launch, leave]);

  return (
    <div className="bp">
      <header className="bp__bar">
        <div className="bp__brand">
          <img src={appIcon} alt="" />
          <span>{t("app.name")}</span>
          <h1 className="bp__heading">{t("bp.title")}</h1>
        </div>
        <Button variant="ghost" className="bp__exit" onClick={leave}>
          <Icon name="chevronLeft" />
          {t("bp.exit")}
          <kbd>Esc</kbd>
        </Button>
      </header>
      {focused ? (
        <>
          <FeaturedGame
            card={focused}
            busy={busy}
            onLaunch={() => void launch()}
            onDetails={openDetails}
          />
          <section className="bp__collection" aria-label={t("bp.collection")}>
            <div className="bp__collection-bar">
              <h2>{t("bp.collection")}</h2>
              <div className="bp__paging">
                <span className="bp__position tnum">
                  {selectedIndex + 1} / {games.length}
                </span>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("bp.previous")}
                  disabled={pageStart === 0}
                  onClick={() => setFocus(Math.max(0, pageStart - columns))}
                >
                  <Icon name="chevronLeft" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("bp.next")}
                  disabled={pageStart + columns >= games.length}
                  onClick={() => setFocus(pageStart + columns)}
                >
                  <Icon name="chevronLeft" className="bp__next-icon" />
                </Button>
              </div>
            </div>
            <div
              className="bp__grid"
              ref={gridRef}
              role="listbox"
              aria-label={t("bp.collection")}
            >
              {visibleGames.map((card, offset) => {
                const index = pageStart + offset;
                return (
                  <BigCard
                    key={card.game.meta.id}
                    card={card}
                    focused={index === selectedIndex}
                    position={index + 1}
                    total={games.length}
                    onFocus={() => setFocus(index)}
                    onOpen={() => void launch(card.game.meta.id)}
                    ref={(element) => {
                      cardRefs.current[index] = element;
                    }}
                  />
                );
              })}
            </div>
          </section>
          <footer className="bp__controls">
            <span>
              <kbd>← →</kbd>
              {t("bp.browse")}
            </span>
            <span>
              <kbd>Enter</kbd>
              {t("bp.launch")}
            </span>
            <span>
              <kbd>E</kbd>
              {t("bp.details")}
            </span>
            <span className="bp__pad-hint">
              <Icon name="gamepad" />
              {t("bp.padKeys")}
            </span>
          </footer>
        </>
      ) : (
        <div className="bp__empty">
          <Icon name="library" size={40} />
          <p>{t("bp.empty")}</p>
          <Button onClick={leave}>{t("bp.toLibrary")}</Button>
        </div>
      )}
    </div>
  );
}

function FeaturedGame({
  card,
  busy,
  onLaunch,
  onDetails,
}: {
  card: GameCard;
  busy: boolean;
  onLaunch: () => void;
  onDetails: () => void;
}) {
  const { t, locale } = useI18n();
  const art = useArtwork(card.game);
  return (
    <section className="bp__hero" aria-label={card.game.title}>
      {art.backdropUrl && (
        <img className="bp__backdrop" src={art.backdropUrl} alt="" aria-hidden="true" />
      )}
      <div className="bp__hero-copy">
        <p className="bp__status">{t(`status.${card.game.status}`)}</p>
        <h2 className="bp__title">{card.game.title}</h2>
        {card.game.title_original && card.game.title_original !== card.game.title && (
          <p className="bp__alt">{card.game.title_original}</p>
        )}
        <div className="bp__meta">
          <span>
            <Icon name="clock" />
            {t("bp.played", { time: formatDuration(card.total_ms, locale) })}
          </span>
          {card.game.rating !== null && (
            <span>
              <Icon name="star" />
              {t("game.rating", { rating: formatRating(card.game.rating) })}
            </span>
          )}
          {card.game.length_minutes ? (
            <span>
              {t("bp.length", {
                time: formatDuration(card.game.length_minutes * 60_000, locale),
              })}
            </span>
          ) : null}
        </div>
        {card.game.notes && <p className="bp__notes">{card.game.notes}</p>}
        <div className="bp__actions">
          <Button variant="primary" onClick={onLaunch} disabled={busy}>
            <Icon name="play" />
            {t(busy ? "play.starting" : "bp.launch")}
          </Button>
          <Button variant="ghost" onClick={onDetails}>
            {t("bp.details")}
            <Icon name="chevronLeft" className="bp__next-icon" />
          </Button>
        </div>
      </div>
      <div className="bp__hero-cover" aria-hidden="true">
        {art.imageUrl ? (
          <img src={art.imageUrl} alt="" onError={art.imageFailed} />
        ) : (
          <Icon name={art.policy === "hide" ? "eyeOff" : "image"} size={48} />
        )}
      </div>
    </section>
  );
}

/**
 * One cover in the grid. A real button, so a pointer or Tab reaches it too, and
 * so the browser keeps the focus ring honest.
 */
function BigCard({
  card,
  focused,
  position,
  total,
  onFocus,
  onOpen,
  ref,
}: {
  card: GameCard;
  focused: boolean;
  position: number;
  total: number;
  onFocus: () => void;
  onOpen: () => void;
  ref: (element: HTMLButtonElement | null) => void;
}) {
  const { t } = useI18n();
  const art = useArtwork(card.game);
  return (
    <button
      type="button"
      ref={ref}
      role="option"
      aria-selected={focused}
      aria-posinset={position}
      aria-setsize={total}
      tabIndex={focused ? 0 : -1}
      className={focused ? "bp-card bp-card--focused" : "bp-card"}
      onFocus={onFocus}
      onClick={onFocus}
      onDoubleClick={onOpen}
      title={card.game.title}
    >
      <span className="bp-card__cover">
        {art.imageUrl ? (
          <img
            className="bp-card__art"
            src={art.imageUrl}
            alt=""
            loading="lazy"
            onError={art.imageFailed}
          />
        ) : (
          <span className="bp-card__art bp-card__art--empty">
            <Icon name="image" />
          </span>
        )}
        {art.policy === "hide" ? (
          <span className="bp-card__veil">
            <Icon name="eyeOff" />
            {t("game.explicitHidden")}
          </span>
        ) : null}
      </span>
      <span className="bp-card__label">{card.game.title}</span>
    </button>
  );
}

/** Kept for the router's lazy import to find a named export. */
export default BigPictureScreen;
