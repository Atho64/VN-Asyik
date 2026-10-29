/**
 * Application store.
 *
 * Kept deliberately small: identity (prefs, library index, toasts) lives here
 * because several screens read it, and so does state the user typed but that must
 * outlive a navigation (the library's search/filter set, the VNDB detail cache).
 * Everything else stays in the screen.
 */

import { create } from "zustand";
import { initialLibraryBrowse, type LibraryBrowse } from "@/lib/library";
import type {
  AppearanceView,
  DetailPayload,
  GameCard,
  PlatformInfo,
  Prefs,
  UpdateView,
} from "@/lib/ipc/types";

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
  /**
   * Search text, status chips, rated-only, view and sort for the library.
   *
   * In the store because the screen unmounts on navigation: a search typed in
   * the library was still applied to the cards, but the query that produced it
   * lived in the component and died on the way to another menu, so coming back
   * showed the whole collection again.
   */
  libraryBrowse: LibraryBrowse;
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
   * `null` records "asked, nothing cached": without it, a game with no VNDB id
   * detail asked again on every visit.
   */
  vndbDetails: Record<string, DetailPayload | null>;

  setPrefs: (prefs: Prefs) => void;
  setPlatform: (platform: PlatformInfo) => void;
  setCards: (cards: GameCard[]) => void;
  setLibraryLoaded: (loaded: boolean) => void;
  setLibraryError: (message: string | null) => void;
  /** Merge a change into the library browsing state. */
  updateLibraryBrowse: (patch: Partial<LibraryBrowse>) => void;
  upsertCard: (card: GameCard) => void;
  removeCard: (id: string) => void;
  pushToast: (kind: ToastKind, message: string) => void;
  dismissToast: (id: number) => void;
  setPaletteOpen: (open: boolean) => void;
  /** Mark a game's metadata refresh as started or finished. */
  setRefreshingVndb: (gameId: string, refreshing: boolean) => void;
  /** Remember a fetched VNDB detail payload. */
  cacheVndbDetail: (vnId: string, detail: DetailPayload | null) => void;
  /**
   * The last update answer. Kept in the store because two screens read it: the
   * library shows the button, Settings shows the version detail.
   */
  update: UpdateView | null;
  setUpdate: (update: UpdateView | null) => void;
  /** Accent colour and background image, applied by the provider. */
  appearance: AppearanceView | null;
  setAppearance: (appearance: AppearanceView | null) => void;
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
  libraryBrowse: initialLibraryBrowse,
  toasts: [],
  paletteOpen: false,
  refreshingVndb: [],
  vndbDetails: {},
  update: null,
  appearance: null,

  setUpdate: (update) => set({ update }),
  setAppearance: (appearance) => set({ appearance }),

  setPrefs: (prefs) => set({ prefs }),
  setPlatform: (platform) => set({ platform }),
  setCards: (cards) => set({ cards, libraryLoaded: true, libraryError: null }),
  setLibraryLoaded: (libraryLoaded) => set({ libraryLoaded }),
  setLibraryError: (libraryError) => set({ libraryError, libraryLoaded: true }),
  updateLibraryBrowse: (patch) =>
    set((state) => ({ libraryBrowse: { ...state.libraryBrowse, ...patch } })),
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
  cacheVndbDetail: (vnId, detail) => {
    // `null` is stored, not deleted: it means "asked, there is nothing cached",
    // which stops a game without VNDB details from re-requesting on every visit.
    // A metadata refresh writes the new payload over it.
    set((state) => ({ vndbDetails: { ...state.vndbDetails, [vnId]: detail } }));
  },
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
