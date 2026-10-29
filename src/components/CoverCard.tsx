import { useEffect } from "react";
import { Link, useNavigate } from "react-router";

import { useContextMenu } from "@/app/ContextMenuProvider";
import { focusArt } from "./Backdrop";
import { useI18n } from "@/app/useI18n";
import { api } from "@/lib/ipc/client";
import { formatRating } from "@/lib/library";
import { formatDuration } from "@/lib/time";
import type { Game, GameCard } from "@/lib/ipc/types";
import { Icon } from "./Icon";
import { ProgressRing } from "./ProgressRing";
import { StatusChip } from "./Chip";
import { applyAccent, useArtwork, type Artwork } from "./useArtwork";

interface CoverCardProps {
  card: GameCard;
  /** `shelf` is the cinematic variant with a taller frame. */
  size?: "grid" | "shelf";
}

// Library backdrop persists after the pointer leaves, until another card takes
// focus. Remember its owner so a content-policy change clears that artwork.
let backdropOwner: string | null = null;

/** Artwork plus the accent publishing that the library backdrop needs. */
function useLibraryArt(game: Game): Artwork & { publishAccent: () => void } {
  const art = useArtwork(game);
  const { policy, backdropUrl, palette } = art;

  useEffect(() => {
    // A card that becomes concealed mid-hover must not leave its artwork behind
    // in the shell backdrop.
    if (policy !== "visible" && backdropOwner === game.meta.id) focusArt(null);
  }, [policy, game.meta.id]);

  return {
    ...art,
    publishAccent: () => {
      backdropOwner = game.meta.id;
      applyAccent(palette);
      focusArt(backdropUrl);
    },
  };
}

/** The blurred wash, scrim and reveal control shared by every library view. */
function ConcealedArt({
  art,
  className,
  label,
}: {
  art: Artwork;
  className: string;
  label: string;
}) {
  return (
    <div className={className}>
      {/* The blur itself is the cached backdrop image behind this scrim: DESIGN.md
          forbids a large CSS blur on a scrolling surface. */}
      <span className="cover__reveal-label">{label}</span>
      <RevealButton art={art} />
    </div>
  );
}

function RevealButton({ art }: { art: Artwork }) {
  const { t } = useI18n();
  return (
    <button
      type="button"
      className="btn btn--secondary cover__reveal-btn"
      onClick={(event) => {
        // Reveal without navigating away from the card.
        event.preventDefault();
        event.stopPropagation();
        art.reveal();
      }}
    >
      {t("game.reveal")}
    </button>
  );
}

/**
 * A game as art.
 *
 * The identity motif in component form: the cover supplies the accent for the
 * whole screen, so pointing at a card changes the room. Explicit artwork obeys
 * the content setting and is revealed only on an explicit action.
 */
export function CoverCard({ card, size = "grid" }: CoverCardProps) {
  const { t, locale } = useI18n();
  const { openMenu } = useContextMenu();
  const navigate = useNavigate();
  const { game, total_ms: totalMs, afk_ms: afkMs } = card;
  const art = useLibraryArt(game);
  const { policy, imageUrl, artUrl, palette, fallback, publishAccent } = art;
  const concealed = policy !== "visible";

  // Persist the extracted accent the first time it is computed, so the database
  // carries it and later launches skip the bitmap decode entirely.
  useEffect(() => {
    if (concealed || !artUrl || game.accent_color === palette.accent) return;
    const timer = window.setTimeout(() => {
      void api.setGameAccent(game.meta.id, palette.accent).catch(() => undefined);
    }, 1_500);
    return () => window.clearTimeout(timer);
  }, [concealed, artUrl, game.accent_color, game.meta.id, palette.accent]);

  const progress =
    game.length_minutes && game.length_minutes > 0
      ? Math.min(1, totalMs / (game.length_minutes * 60_000))
      : null;

  return (
    <Link
      to={`/game/${game.meta.id}`}
      className="cover"
      data-size={size}
      onPointerEnter={publishAccent}
      onFocus={publishAccent}
      onContextMenu={(event) => {
        event.preventDefault();
        openMenu(event, [
          {
            label: t("menu.viewDetails"),
            onSelect: () => {
              void navigate(`/game/${game.meta.id}`);
            },
          },
          {
            label: t("menu.copyTitle"),
            onSelect: () => {
              void navigator.clipboard.writeText(game.title);
            },
          },
          null,
          {
            label: t("menu.reload"),
            onSelect: () => {
              window.location.reload();
            },
          },
        ]);
      }}
    >
      <div className="cover__frame">
        {imageUrl ? (
          // The blurred variant is painted through the same element as the sharp
          // cover: it already fits the frame, so this costs no layout change.
          <img
            className={concealed ? "cover__art cover__art--blurred" : "cover__art"}
            src={imageUrl}
            alt={concealed ? "" : t("game.coverAlt", { title: game.title })}
            aria-hidden={concealed ? "true" : undefined}
            loading="lazy"
            decoding="async"
            onError={concealed ? undefined : art.imageFailed}
          />
        ) : (
          <div
            className="cover__placeholder"
            style={{
              background: `linear-gradient(160deg, ${fallback.accent}38, var(--bg-elevated) 72%)`,
            }}
          >
            {concealed ? "" : game.title}
          </div>
        )}

        {concealed ? (
          <ConcealedArt
            art={art}
            className="cover__reveal"
            label={t("game.explicitHidden")}
          />
        ) : null}

        <div className="cover__badge">
          <StatusChip status={game.status} />
        </div>
      </div>

      <div style={{ display: "flex", alignItems: "flex-start", gap: 8 }}>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div className="cover__title" title={game.title}>
            {game.title}
          </div>
          <div className="cover__meta tnum">
            {totalMs > 0
              ? t("game.played", { time: formatDuration(totalMs, locale) })
              : t("game.notPlayed")}
            {/* Away time is always shown separately; it is never merged into the
                playtime figure above. */}
            {afkMs > 0 ? (
              <span title={t("game.afkHint")}>
                {" · "}
                {t("game.afk", { time: formatDuration(afkMs, locale) })}
              </span>
            ) : null}
          </div>
        </div>
        {progress !== null ? (
          <ProgressRing
            progress={progress}
            title={t("game.progress", { percent: Math.round(progress * 100) })}
          />
        ) : null}
      </div>
    </Link>
  );
}

/** Dense row used by the list view. Same data, one line. */
export function GameRow({ card }: { card: GameCard }) {
  const { t, locale } = useI18n();
  const { game, total_ms: totalMs, afk_ms: afkMs } = card;
  const art = useLibraryArt(game);
  const { policy, imageUrl, fallback, publishAccent } = art;
  const concealed = policy !== "visible";

  return (
    <Link
      to={`/game/${game.meta.id}`}
      className="row"
      onPointerEnter={publishAccent}
      onFocus={publishAccent}
    >
      <div
        className="row__thumb"
        style={
          imageUrl
            ? undefined
            : {
                background: `linear-gradient(160deg, ${fallback.accent}44, var(--bg-elevated))`,
              }
        }
      >
        {imageUrl ? (
          <img
            className={concealed ? "cover__art--blurred" : undefined}
            src={imageUrl}
            alt=""
            width={44}
            height={66}
            loading="lazy"
            decoding="async"
            onError={concealed ? undefined : art.imageFailed}
            style={{ width: "100%", height: "100%", objectFit: "cover" }}
          />
        ) : concealed ? (
          <span className="row__concealed" aria-hidden="true">
            <Icon name="eyeOff" size={18} />
          </span>
        ) : null}
      </div>
      <div style={{ minWidth: 0 }}>
        <div className="row__title">{game.title}</div>
        <div className="row__sub">
          {t(`status.${game.status}`)}
          {totalMs > 0 ? ` · ${formatDuration(totalMs, locale)}` : ""}
          {afkMs > 0 ? ` · ${t("game.afk", { time: formatDuration(afkMs, locale) })}` : ""}
        </div>
        {concealed ? (
          <div className="row__reveal">
            <span>{t("game.explicitHidden")}</span>
            <RevealButton art={art} />
          </div>
        ) : null}
      </div>
      <div className="cover__meta tnum">
        {game.rating
          ? t("game.rating", { rating: formatRating(game.rating) })
          : t("game.unrated")}
      </div>
    </Link>
  );
}
