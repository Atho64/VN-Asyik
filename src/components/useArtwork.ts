import { useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { updateAppearance } from "@/lib/theme";
import { api } from "@/lib/ipc/client";
import { isExplicit } from "@/lib/library";
import { fallbackPalette } from "@/lib/palette";
import { usePalette } from "@/lib/usePalette";
import type { Game, LocalArt } from "@/lib/ipc/types";

/** Content visibility policy for explicit art (`blur` uses a pre-blurred copy, `hide` renders a neutral surface). */
export type ArtPolicy = "visible" | "blur" | "hide";

export interface Artwork {
  game: Game;
  /** Resolved image URL candidate list, or `null` while unknown. */
  artUrl: string | null;
  /** Image to paint (`blur` uses a pre-blurred copy so a concealing component never renders the sharp original). */
  imageUrl: string | null;
  /** Library backdrop target; `null` while art must stay concealed so hidden covers cannot leak. */
  backdropUrl: string | null;
  /** Cached pre-blurred data URL (`null` when absent or concealed) so canvas share cards avoid tainting. */
  localBackdropUrl: string | null;
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

/** Shared artwork resolver enforcing cache-first lookup, remote fallback, and centralized NSFW policy. */
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
  // Always fetch custom covers by game id so manually added games without a VNDB id resolve art too.
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

  // Skip the IPC round trip on every virtualized grid card when no custom cover path exists.
  const hasCustomCover = Boolean(game.custom_cover_path);
  useEffect(() => {
    if (!hasCustomCover) return undefined;
    let cancelled = false;
    void api.gameCustomArt(game.meta.id).then(
      (art) => {
        if (!cancelled) setCustom({ key: artKey, art });
      },
      (caught: unknown) => {
        // Missing files resolve to nulls in Rust, so rejections indicate contract/IO errors; log them.
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
  // User-chosen custom covers take precedence over cached VNDB and remote URLs.
  const customReady = !hasCustomCover || custom?.key === artKey;
  const cacheReady = !game.vndb_id || cached?.key === artKey;
  const failedUrls = failed?.key === artKey ? failed.urls : [];
  // Resolve local lookups before remote URLs so offline libraries avoid flashing uncached covers.
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
  // Scrolling lists cannot afford live CSS blur; require a pre-blurred copy or fall back to hide.
  const blurredUrl = customArt?.backdrop_data_url ?? localArt?.backdrop_data_url ?? null;
  const policy: ArtPolicy = !concealed
    ? "visible"
    : nsfw === "blur" && blurredUrl
      ? "blur"
      : "hide";
  const imageUrl = policy === "blur" ? blurredUrl : policy === "hide" ? null : artUrl;
  // Revealed artwork may supply the backdrop: the user asked for exactly that.
  const localBackdropUrl = concealed
    ? null
    : (customArt?.backdrop_data_url ?? localArt?.backdrop_data_url ?? null);
  const backdropUrl = concealed ? null : (localBackdropUrl ?? artUrl);
  // Never sample a concealed cover for the accent: a blurred thumbnail still
  // leaks the dominant colours of artwork the user asked to conceal.
  const palette = usePalette(concealed ? null : artUrl, game.accent_color, game.title);
  const fallback = fallbackPalette(game.title);

  return {
    game,
    artUrl,
    imageUrl,
    backdropUrl,
    localBackdropUrl,
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

/** Publishes artwork accent variables unless Appearance locks a custom accent colour. */
export function applyAccent(palette: ReturnType<typeof fallbackPalette>): void {
  updateAppearance({ tint: palette });
}
