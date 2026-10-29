/**
 * Provider that loads the app shell data once and keeps the DOM attributes that
 * the CSS reads (`data-theme`, `data-reduce-effects`, `lang`) in sync with prefs.
 */

import { useEffect, useLayoutEffect, useMemo, type ReactNode } from "react";

import { api, IpcError } from "@/lib/ipc/client";
import type { GameCard } from "@/lib/ipc/types";
import { createTranslator, resolveLocale, type Translator } from "@/i18n";
import { createContext } from "react";
import { focusArt } from "@/components/Backdrop";
import { prefersReducedTransparency, resolveTheme, useAppStore } from "./store";

interface AppContextValue {
  t: Translator;
  locale: ReturnType<typeof resolveLocale>;
  ready: boolean;
  bootError: string | null;
}

export const AppContext = createContext<AppContextValue | null>(null);

async function loadLibrary(os: string | undefined): Promise<GameCard[]> {
  if (os === "android") {
    try {
      // Recover UsageStats before reading totals, including after process death.
      await api.trackingAndroidSession();
    } catch {
      // Keep saved data usable; the tracking screen reports recovery failures.
    }
  }
  return api.listGames();
}

export function AppProvider({ children }: { children: ReactNode }) {
  const prefs = useAppStore((state) => state.prefs);
  const setPrefs = useAppStore((state) => state.setPrefs);
  const setPlatform = useAppStore((state) => state.setPlatform);
  const platform = useAppStore((state) => state.platform);
  const setCards = useAppStore((state) => state.setCards);
  const setLibraryError = useAppStore((state) => state.setLibraryError);
  const libraryLoaded = useAppStore((state) => state.libraryLoaded);
  const bootError = useAppStore((state) => state.libraryError);

  const locale = resolveLocale(prefs.language);
  const t = useMemo(() => createTranslator(locale), [locale]);

  // One boot pass: prefs, platform capabilities, then the library itself. The
  // library is separate so its failure does not blank the whole app.
  useEffect(() => {
    let cancelled = false;

    const boot = async (): Promise<void> => {
      try {
        const [loadedPrefs, platform] = await Promise.all([
          api.getPrefs().catch(() => null),
          api.platformInfo().catch(() => null),
        ]);
        if (cancelled) return;
        if (loadedPrefs) setPrefs(loadedPrefs);
        if (platform) setPlatform(platform);

        try {
          const cards = await loadLibrary(platform?.os);
          if (!cancelled) setCards(cards);
        } catch (error) {
          if (!cancelled) {
            setLibraryError(
              error instanceof IpcError ? error.message : t("library.errorTitle"),
            );
          }
        }
      } catch (error) {
        if (!cancelled) {
          setLibraryError(error instanceof Error ? error.message : t("common.error"));
        }
      }
    };

    void boot();
    return () => {
      cancelled = true;
    };
    // Boot runs once. `t` is only used for the fallback message.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (platform?.os !== "android") return;
    let cancelled = false;
    const refreshAfterPlay = async (): Promise<void> => {
      if (document.visibilityState === "hidden") return;
      try {
        const cards = await loadLibrary(platform.os);
        if (!cancelled) setCards(cards);
      } catch {
        // The tracking screen exposes usage access and import errors.
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
  }, [platform?.os, setCards]);

  // DOM attributes are the contract between prefs and CSS (tokens.css).
  useEffect(() => {
    const root = document.documentElement;
    const apply = (): void => {
      root.dataset.theme = resolveTheme(prefs.theme);
      root.dataset.reduceEffects = prefs.reduce_effects ? "true" : "false";
      root.dataset.platform = platform?.os ?? "unknown";
      root.lang = locale;
    };
    apply();

    if (prefs.theme !== "system" || !window.matchMedia) return;
    const query = window.matchMedia("(prefers-color-scheme: light)");
    query.addEventListener("change", apply);
    return () => query.removeEventListener("change", apply);
  }, [prefs.theme, prefs.reduce_effects, locale, platform?.os]);

  // The tray menu is built in Rust before the frontend knows the locale, so it
  // starts in English. Relabel it as soon as the UI knows the language, and
  // again on every switch: the tray is a user-visible surface like any other.
  useEffect(() => {
    void api
      .traySetLabels(t("tray.show"), t("tray.hide"), t("tray.quit"))
      .catch(() => {
        // Platforms without a tray have nothing to relabel.
      });
  }, [t]);

  useLayoutEffect(() => {
    // The focused card can leave the virtual range while its artwork remains
    // in the shell. Revoke every previous reveal when the content policy changes,
    // including when that card is no longer mounted to react to preferences.
    focusArt(null);
  }, [prefs.nsfw]);

  // Reduce effects is suggested, never forced, on platforms that ask for it.
  useEffect(() => {
    if (prefersReducedTransparency() && !prefs.reduce_effects) {
      void api
        .setReduceEffects(true)
        .then(setPrefs)
        .catch(() => undefined);
    }
    // Only on first resolution of prefs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const value = useMemo<AppContextValue>(
    () => ({ t, locale, ready: libraryLoaded, bootError }),
    [t, locale, libraryLoaded, bootError],
  );

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}
