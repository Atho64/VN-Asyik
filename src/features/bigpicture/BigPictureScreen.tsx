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

import { moveFor, nextIndex, readPad, type PadMove } from "./navigation";

/**
 * Big Picture: the library at television distance.
 *
 * The whole surface is one focused card plus a grid that can be walked with
 * arrows, WASD, or a gamepad, so a HTPC or a sofa keyboard never needs a mouse.
 * It is a route rather than an overlay, so leaving it is a normal navigation and
 * the browser back button works.
 *
 * Every action is also a real button: the keyboard path is the fast one, not the
 * only one.
 */
export function BigPictureScreen() {
  const { t, locale } = useI18n();
  const navigate = useNavigate();
  const cards = useAppStore((state) => state.cards);
  const pushToast = useAppStore((state) => state.pushToast);
  const [focus, setFocus] = useState(0);
  const [columns, setColumns] = useState(5);
  const [busy, setBusy] = useState(false);
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

  const focused = games[Math.min(focus, Math.max(games.length - 1, 0))] ?? null;

  const leave = useCallback(() => {
    // A fullscreen surface that leaves the window fullscreen traps the next screen.
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => undefined);
    navigate("/");
  }, [navigate]);

  const launch = useCallback(async () => {
    if (!focused) return;
    setBusy(true);
    try {
      const outcome = await api.launchGame(focused.game.meta.id);
      pushToast(outcome.started ? "success" : "info", outcome.explanation);
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  }, [focused, pushToast, t]);

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
      setColumns(Math.max(1, count));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [games.length]);

  // Keep the focused card on screen as it moves. Guarded because a test
  // environment (and an older webview) may not implement scrollIntoView.
  useEffect(() => {
    const element = cardRefs.current[focus];
    if (typeof element?.scrollIntoView === "function") {
      element.scrollIntoView({ block: "nearest", behavior: "smooth" });
    }
  }, [focus]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        leave();
        return;
      }
      if (event.key === "Enter") {
        event.preventDefault();
        void launch();
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
    let previous: PadMove = null;
    const tick = (): void => {
      const { move, button } = readPad(Array.from(navigator.getGamepads()), previous);
      previous = move ?? previous;
      if (move) {
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
      if (button === "a") void launch();
      if (button === "b") leave();
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [columns, games.length, launch, leave]);

  if (games.length === 0) {
    return (
      <div className="bp">
        <header className="bp__bar">
          <h1 className="bp__heading">{t("bp.title")}</h1>
          <Button onClick={leave}>{t("bp.exit")}</Button>
        </header>
        <p className="bp__empty">{t("bp.empty")}</p>
      </div>
    );
  }

  return (
    <div className="bp">
      <header className="bp__bar">
        <div>
          <h1 className="bp__heading">{t("bp.title")}</h1>
          <p className="bp__hint">{t("bp.keys")}</p>
        </div>
        <Button onClick={leave}>{t("bp.exit")}</Button>
      </header>

      <div className="bp__layout">
        <div className="bp__grid" ref={gridRef} role="listbox" aria-label={t("bp.title")}>
          {games.map((card, index) => (
            <BigCard
              key={card.game.meta.id}
              card={card}
              focused={index === focus}
              onFocus={() => setFocus(index)}
              onOpen={() => void launch()}
              ref={(element) => {
                cardRefs.current[index] = element;
              }}
            />
          ))}
        </div>

        {focused ? (
          <aside className="bp__panel" aria-live="polite">
            <p className="bp__status">
              {t(`status.${focused.game.status}`)}
              {/* Ratings are stored in tenths, as everywhere else in the app. */}
              {focused.game.rating !== null
                ? ` · ${t("game.rating", { rating: formatRating(focused.game.rating) })}`
                : ""}
            </p>
            <h2 className="bp__title">{focused.game.title}</h2>
            {focused.game.title_original ? (
              <p className="bp__alt">{focused.game.title_original}</p>
            ) : null}
            <p className="bp__meta">
              {t("bp.played", {
                time: formatDuration(focused.total_ms, locale),
              })}
              {focused.game.length_minutes
                ? ` · ${t("bp.length", {
                    time: formatDuration(focused.game.length_minutes * 60_000, locale),
                  })}`
                : ""}
            </p>
            {focused.game.notes ? (
              <p className="bp__notes">{focused.game.notes}</p>
            ) : null}
            <div className="bp__actions">
              <Button variant="primary" onClick={() => void launch()} disabled={busy}>
                <Icon name="play" />
                {t("bp.launch")}
              </Button>
              <Button onClick={openDetails}>
                <Icon name="library" />
                {t("bp.details")}
              </Button>
            </div>
          </aside>
        ) : null}
      </div>
    </div>
  );
}

/**
 * One cover in the grid. A real button, so a pointer or Tab reaches it too, and
 * so the browser keeps the focus ring honest.
 */
function BigCard({
  card,
  focused,
  onFocus,
  onOpen,
  ref,
}: {
  card: GameCard;
  focused: boolean;
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
      className={focused ? "bp-card bp-card--focused" : "bp-card"}
      onFocus={onFocus}
      onMouseEnter={onFocus}
      onClick={onOpen}
      title={card.game.title}
    >
      {art.imageUrl ? (
        <img className="bp-card__art" src={art.imageUrl} alt="" loading="lazy" />
      ) : (
        <span className="bp-card__art bp-card__art--empty">
          <Icon name="image" />
        </span>
      )}
      {art.policy === "hide" ? (
        <span className="bp-card__veil">{t("game.explicitHidden")}</span>
      ) : null}
      <span className="bp-card__label">{card.game.title}</span>
    </button>
  );
}

/** Kept for the router's lazy import to find a named export. */
export default BigPictureScreen;
