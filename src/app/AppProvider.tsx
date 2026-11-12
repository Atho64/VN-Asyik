/**
 * Provider that loads the app shell data once and keeps the DOM attributes that
 * the CSS reads (`data-theme`, `data-reduce-effects`, `lang`) in sync with prefs.
 */

import { useEffect, useLayoutEffect, useMemo, type ReactNode } from "react";

import { listenBetaFeatures } from "@/lib/ipc/prefs";
import { api, IpcError } from "@/lib/ipc/client";
import type { GameCard } from "@/lib/ipc/types";
import { updateAppearance } from "@/lib/theme";
import { createTranslator, resolveLocale, type Translator } from "@/i18n";
import { createContext } from "react";
import { focusArt } from "@/components/Backdrop";
import {
  prefersMoreContrast,
  prefersReducedTransparency,
  resolveTheme,
  useAppStore,
} from "./store";

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

export function AppProvider({
  children,
  companionWindow = false,
}: {
  children: ReactNode;
  companionWindow?: boolean;
}) {
  const prefs = useAppStore((state) => state.prefs);
  const setPrefs = useAppStore((state) => state.setPrefs);
  const setPlatform = useAppStore((state) => state.setPlatform);
  const platform = useAppStore((state) => state.platform);
  const setCards = useAppStore((state) => state.setCards);
  const setLibraryError = useAppStore((state) => state.setLibraryError);
  const setUpdate = useAppStore((state) => state.setUpdate);
  const appearance = useAppStore((state) => state.appearance);
  const setAppearance = useAppStore((state) => state.setAppearance);
  const libraryLoaded = useAppStore((state) => state.libraryLoaded);
  const bootError = useAppStore((state) => state.libraryError);

  const locale = resolveLocale(prefs.language);
  const t = useMemo(() => createTranslator(locale), [locale]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listenBetaFeatures((enabled) => {
      if (!disposed)
        useAppStore.setState((state) => ({
          prefs: { ...state.prefs, beta_features: enabled },
        }));
    })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

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
        else useAppStore.setState({ prefsLoaded: true });
        if (platform) setPlatform(platform);
        if (companionWindow) return;

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
    if (companionWindow) window.addEventListener("focus", boot);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", boot);
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
  //
  // A theme carries its own mode, so it decides `data-theme` outright: a dark
  // theme on a light-mode system is the reader's explicit choice, not a bug.
  useEffect(() => {
    const root = document.documentElement;
    const apply = (): void => {
      root.dataset.theme = appearance?.theme?.base ?? resolveTheme(prefs.theme);
      root.dataset.reduceEffects = prefs.reduce_effects ? "true" : "false";
      root.dataset.platform = platform?.os ?? "unknown";
      root.lang = locale;
    };
    apply();

    if (prefs.theme !== "system" || !window.matchMedia) return;
    const query = window.matchMedia("(prefers-color-scheme: light)");
    query.addEventListener("change", apply);
    return () => query.removeEventListener("change", apply);
  }, [prefs.theme, prefs.reduce_effects, locale, platform?.os, appearance?.theme?.base]);

  // The tray menu is built in Rust before the frontend knows the locale, so it
  // starts in English. Relabel it as soon as the UI knows the language, and
  // again on every switch: the tray is a user-visible surface like any other.
  useEffect(() => {
    if (companionWindow) return;
    void api.traySetLabels(t("tray.show"), t("tray.hide"), t("tray.quit")).catch(() => {
      // Platforms without a tray have nothing to relabel.
    });
  }, [t, companionWindow]);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void api
        .appearanceGet()
        .then((result) => {
          if (!cancelled) setAppearance(result);
        })
        .catch(() => undefined);
    };
    refresh();
    if (companionWindow) window.addEventListener("focus", refresh);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", refresh);
    };
  }, [setAppearance, companionWindow]);

  /* Theme and accent are CSS variable overrides rather than a class per colour:
   * the token set already reads them everywhere, so one property recolours
   * buttons, chips, focus rings and charts together. `updateAppearance` is the
   * only writer — it resolves theme, then the reader's accent, then the focused
   * game's tint, so no surface can clobber another's property. */
  useEffect(() => {
    const apply = (): void => {
      const theme = appearance?.theme ?? null;
      updateAppearance({
        theme,
        mode: theme?.base ?? resolveTheme(prefs.theme),
        accent: appearance?.accent ?? null,
        gameTint: appearance?.gameTint ?? true,
        media: {
          reducedTransparency: prefersReducedTransparency(),
          moreContrast: prefersMoreContrast(),
          reduceEffects: prefs.reduce_effects,
        },
      });
    };
    apply();
    // A system theme switch flips the surface the accent must read against, and
    // the two accessibility queries change what a theme is allowed to paint, so
    // re-resolve live rather than waiting for the next prefs change.
    if (!window.matchMedia) return;
    const listeners: Array<() => void> = [];
    const watch = (query: string): void => {
      const list = window.matchMedia(query);
      list.addEventListener("change", apply);
      listeners.push(() => list.removeEventListener("change", apply));
    };
    if (prefs.theme === "system") watch("(prefers-color-scheme: light)");
    watch("(prefers-reduced-transparency: reduce)");
    watch("(prefers-contrast: more)");
    return () => {
      for (const stop of listeners) stop();
    };
  }, [
    appearance?.accent,
    appearance?.theme,
    appearance?.gameTint,
    prefs.theme,
    prefs.reduce_effects,
  ]);

  /* Ask GitHub once per launch; the answer is cached for six hours server-side,
   * so this is one request a day in practice. A failure is silent on purpose: a
   * library screen must not show an error because a release feed was unreachable. */
  useEffect(() => {
    if (companionWindow) return;
    let cancelled = false;
    void api
      .updateCheck(false)
      .then((result) => {
        if (!cancelled) setUpdate(result);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [setUpdate, companionWindow]);

  useLayoutEffect(() => {
    // The focused card can leave the virtual range while its artwork remains
    // in the shell. Revoke every previous reveal when the content policy changes,
    // including when that card is no longer mounted to react to preferences.
    focusArt(null);
  }, [prefs.nsfw]);

  // Reduce effects is suggested, never forced, on platforms that ask for it.
  useEffect(() => {
    if (companionWindow) return;
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
    () => ({ t, locale, ready: companionWindow || libraryLoaded, bootError }),
    [t, locale, companionWindow, libraryLoaded, bootError],
  );

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}
