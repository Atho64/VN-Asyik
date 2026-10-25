/**
 * Application store.
 *
 * Kept deliberately small: a screen's data lives in the screen, while identity
 * (prefs, library index, toasts) lives here because several screens read it.
 */

import { create } from "zustand";
import type { DetailPayload, GameCard, PlatformInfo, Prefs } from "@/lib/ipc/types";

export type ToastKind = "info" | "success" | "error";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

export interface ThemeResolution {
  /** What the user chose. */
  mode: string;
  /** What is actually applied right now (`dark` or `light`). */
  applied: "dark" | "light";
}

interface AppState {
  prefs: Prefs;
  platform: PlatformInfo | null;
  cards: GameCard[];
  libraryLoaded: boolean;
  libraryError: string | null;
  toasts: Toast[];
  paletteOpen: boolean;
  /**
   * Game ids whose VNDB metadata refresh is in flight.
   *
   * In the store rather than component state because the game page unmounts on
   * navigation: a refresh started before leaving kept running, but the flag that
   * said so died with the component, so coming back showed "Refresh" again over a
   * request that was still going.
   */
  refreshingVndb: string[];
  /**
   * VNDB detail payloads already fetched, keyed by VN id.
   *
   * The game page unmounts on navigation and its local copy dies with it, so
   * returning re-requested a row that had not changed. A metadata refresh is the
   * only thing that invalidates an entry, and it writes the new value here.
   */
  vndbDetails: Record<string, DetailPayload>;

  setPrefs: (prefs: Prefs) => void;
  setPlatform: (platform: PlatformInfo) => void;
  setCards: (cards: GameCard[]) => void;
  setLibraryLoaded: (loaded: boolean) => void;
  setLibraryError: (message: string | null) => void;
  upsertCard: (card: GameCard) => void;
  removeCard: (id: string) => void;
  pushToast: (kind: ToastKind, message: string) => void;
  dismissToast: (id: number) => void;
  setPaletteOpen: (open: boolean) => void;
  /** Mark a game's metadata refresh as started or finished. */
  setRefreshingVndb: (gameId: string, refreshing: boolean) => void;
  /** Remember a fetched VNDB detail payload. */
  cacheVndbDetail: (vnId: string, detail: DetailPayload | null) => void;
}

const defaultPrefs: Prefs = {
  theme: "dark",
  language: "en",
  nsfw: "blur",
  reduce_effects: false,
};

let toastId = 0;

export const useAppStore = create<AppState>((set) => ({
  prefs: defaultPrefs,
  platform: null,
  cards: [],
  libraryLoaded: false,
  libraryError: null,
  toasts: [],
  paletteOpen: false,
  refreshingVndb: [],
  vndbDetails: {},

  setPrefs: (prefs) => set({ prefs }),
  setPlatform: (platform) => set({ platform }),
  setCards: (cards) => set({ cards, libraryLoaded: true, libraryError: null }),
  setLibraryLoaded: (libraryLoaded) => set({ libraryLoaded }),
  setLibraryError: (libraryError) => set({ libraryError, libraryLoaded: true }),
  upsertCard: (card) =>
    set((state) => {
      const index = state.cards.findIndex((entry) => entry.game.meta.id === card.game.meta.id);
      if (index === -1) return { cards: [card, ...state.cards] };
      const next = [...state.cards];
      next[index] = card;
      return { cards: next };
    }),
  removeCard: (id) =>
    set((state) => ({
      cards: state.cards.filter((entry) => entry.game.meta.id !== id),
    })),
  pushToast: (kind, message) =>
    set((state) => {
      toastId += 1;
      const toast: Toast = { id: toastId, kind, message };
      return { toasts: [...state.toasts, toast].slice(-3) };
    }),
  dismissToast: (id) =>
    set((state) => ({ toasts: state.toasts.filter((toast) => toast.id !== id) })),
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
  cacheVndbDetail: (vnId, detail) =>
    set((state) => {
      if (detail === null) {
        const next = { ...state.vndbDetails };
        delete next[vnId];
        return { vndbDetails: next };
      }
      return { vndbDetails: { ...state.vndbDetails, [vnId]: detail } };
    }),
  setRefreshingVndb: (gameId, refreshing) =>
    set((state) => ({
      refreshingVndb: refreshing
        ? [...new Set([...state.refreshingVndb, gameId])]
        : state.refreshingVndb.filter((id) => id !== gameId),
    })),
}));

/** Resolve `system` against the OS preference without a layout effect. */
export function resolveTheme(mode: string): "dark" | "light" {
  if (mode === "light") return "light";
  if (mode === "dark") return "dark";
  if (typeof window !== "undefined" && window.matchMedia) {
    return window.matchMedia("(prefers-color-scheme: light)").matches
      ? "light"
      : "dark";
  }
  return "dark";
}

/** Whether the OS asks for reduced transparency. */
export function prefersReducedTransparency(): boolean {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  return window.matchMedia("(prefers-reduced-transparency: reduce)").matches;
}
