import { useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { api } from "@/lib/ipc/client";
import { isExplicit } from "@/lib/library";
import { fallbackPalette } from "@/lib/palette";
import { usePalette } from "@/lib/usePalette";
import type { Game, LocalArt } from "@/lib/ipc/types";

/**
 * How much of a game's artwork the current content setting allows.
 *
 * `visible` — nothing to conceal (or the user asked for it).
 * `blur`    — explicit art is covered by its own blurred image.
 * `hide`    — explicit art is replaced by a neutral surface.
 *
 * Blur is a real state rather than a synonym for hidden: the setting is called
 * "Blur until revealed", and for two phases it behaved exactly like `hide`, so
 * the control did nothing a user could see.
 */
export type ArtPolicy = "visible" | "blur" | "hide";

export interface Artwork {
  game: Game;
  /** The whole image URL candidate list resolved, or `null` while unknown. */
  artUrl: string | null;
  /**
   * The image to paint. This is the blurred source while the policy is `blur`,
   * which is why it is a separate field from `artUrl`: the sharp original is
   * never rendered by a concealing component.
   */
  imageUrl: string | null;
  /**
   * Where to send the library backdrop. `null` while art must stay concealed:
   * a blurred downscale is still a legible picture of the artwork at backdrop
   * scale, so a hidden cover cannot be laundered through it.
   */
  backdropUrl: string | null;
  /** What the calling surface must do with the artwork. */
  policy: ArtPolicy;
  palette: ReturnType<typeof fallbackPalette>;
  fallback: ReturnType<typeof fallbackPalette>;
  /** Turn this artwork's concealment off for the current policy value. */
  reveal: () => void;
  /** True once the user revealed this exact artwork under this exact policy. */
  revealed: boolean;
  /** Remember an image URL that failed to load, so the next candidate is used. */
  imageFailed: () => void;
  /** True when no candidate is left to try and nothing is cached. */
  hasArt: boolean;
}

interface UseArtworkOptions {
  /**
   * Re-read this URL instead of the locally cached art. The library prefers the
   * cache; the detail screen renders the cover the row actually stored.
   */
  preferRemote?: boolean;
  /** Key used to publish the accent to the shell. Defaults to the game id. */
  accentOwner?: string;
}

/**
 * Shared artwork resolution, caching and content policy.
 *
 * Every surface that shows a game's art goes through this hook so the content
 * setting cannot be honoured by one view and ignored by the next. Resolution
 * order is local cache first (offline browsing must not flash an uncached remote
 * URL), then the remote cover, then a neutral title-hash surface.
 */
export function useArtwork(game: Game, options: UseArtworkOptions = {}): Artwork {
  const nsfw = useAppStore((state) => state.prefs.nsfw);
  const { preferRemote = false } = options;
  const artKey = `${game.meta.id}:${game.vndb_id ?? ""}:${game.cover_url ?? ""}:${game.custom_cover_path ?? ""}`;
  const revealScope = `${artKey}:${nsfw}:${game.cover_sexual}:${game.cover_violence}`;
  const [reveal, setReveal] = useState({ scope: revealScope, shown: false });
  // A policy round-trip must not resurrect a previous "Show once" action.
  if (reveal.scope !== revealScope) {
    setReveal({ scope: revealScope, shown: false });
  }
  const [cached, setCached] = useState<{ key: string; art: LocalArt | null } | null>(
    null,
  );
  // A cover the user picked, keyed by game id. Fetched for every game, not only
  // VNDB ones: this is the whole point of the feature — a manually added game has
  // no VN id and would otherwise never resolve any art at all.
  const [custom, setCustom] = useState<{
    key: string;
    art: LocalArt | null;
  } | null>(null);
  const [failed, setFailed] = useState<{ key: string; urls: string[] } | null>(null);

  useEffect(() => {
    if (!game.vndb_id) return undefined;
    let cancelled = false;
    void api.vndbLocalArt(game.vndb_id).then(
      (art) => {
        if (!cancelled) setCached({ key: artKey, art });
      },
      () => {
        if (!cancelled) setCached({ key: artKey, art: null });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [artKey, game.vndb_id]);

  // Only fetch when the row says there is a file to read. A game with no custom
  // cover must not pay for an IPC round trip on every card in a virtualized grid.
  const hasCustomCover = Boolean(game.custom_cover_path);
  useEffect(() => {
    if (!hasCustomCover) return undefined;
    let cancelled = false;
    void api.gameCustomArt(game.meta.id).then(
      (art) => {
        if (!cancelled) setCustom({ key: artKey, art });
      },
      (caught: unknown) => {
        // A missing file resolves to nulls on the Rust side, so a rejection is a
        // real failure (typically an IPC contract break), not "no art". Log it:
        // swallowing this into `art: null` is how a camelCase/snake_case mismatch
        // made a correctly saved cover render as nothing at all.
        if (!cancelled) console.error("game_custom_art failed", caught);
        if (!cancelled) setCustom({ key: artKey, art: null });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [artKey, game.meta.id, hasCustomCover]);

  const localArt = cached?.key === artKey ? cached.art : null;
  const customArt = custom?.key === artKey ? custom.art : null;
  // The custom cover wins over both the VNDB cache and the remote URL: the user
  // chose it deliberately, and a stale VNDB cover must not shadow it.
  const customReady = !hasCustomCover || custom?.key === artKey;
  const cacheReady = !game.vndb_id || cached?.key === artKey;
  const failedUrls = failed?.key === artKey ? failed.urls : [];
  // Wait for the local lookups before requesting a remote cover. Browsing a
  // cached library offline should not briefly render an uncached URL first.
  const candidates = preferRemote
    ? [game.cover_url, customArt?.cover_data_url, localArt?.cover_data_url]
    : [customArt?.cover_data_url, localArt?.cover_data_url, game.cover_url];
  const artUrl =
    cacheReady && customReady
      ? (candidates.find(
          (url): url is string =>
            typeof url === "string" && url.length > 0 && !failedUrls.includes(url),
        ) ?? null)
      : null;
  const hasArt = Boolean(
    game.cover_url ||
    game.custom_cover_path ||
    localArt?.cover_data_url ||
    localArt?.backdrop_data_url,
  );
  const revealed = reveal.scope === revealScope && reveal.shown;
  const explicit = isExplicit(game);
  const concealed = nsfw !== "show" && explicit && hasArt && !revealed;
  // The cached pre-blurred backdrop is the only blur this app can afford on a
  // scrolling surface (DESIGN.md §2). Without it there is nothing to blur, so
  // the artwork falls back to the neutral surface rather than being shown.
  const blurredUrl = customArt?.backdrop_data_url ?? localArt?.backdrop_data_url ?? null;
  const policy: ArtPolicy = !concealed
    ? "visible"
    : nsfw === "blur" && blurredUrl
      ? "blur"
      : "hide";
  const imageUrl = policy === "blur" ? blurredUrl : policy === "hide" ? null : artUrl;
  // Revealed artwork may supply the backdrop: the user asked for exactly that.
  const backdropUrl = concealed
    ? null
    : (customArt?.backdrop_data_url ?? localArt?.backdrop_data_url ?? artUrl);
  // Never sample a concealed cover for the accent: a blurred thumbnail still
  // leaks the dominant colours of artwork the user asked to conceal.
  const palette = usePalette(concealed ? null : artUrl, game.accent_color, game.title);
  const fallback = fallbackPalette(game.title);

  return {
    game,
    artUrl,
    imageUrl,
    backdropUrl,
    policy,
    palette,
    fallback,
    reveal: () => setReveal({ scope: revealScope, shown: true }),
    revealed,
    imageFailed: () => {
      if (!artUrl) return;
      setFailed((current) => ({
        key: artKey,
        urls: [...(current?.key === artKey ? current.urls : []), artUrl],
      }));
    },
    hasArt,
  };
}

/** Point the palette variables at an artwork's accent. */
export function applyAccent(palette: ReturnType<typeof fallbackPalette>): void {
  const root = document.documentElement;
  root.style.setProperty("--accent", palette.accent);
  root.style.setProperty("--accent-soft", palette.accentSoft);
  root.style.setProperty("--on-accent", palette.onAccent);
}
