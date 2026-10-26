/** Validated IPC for Seiya-Saiga walkthroughs and smart translations. */
import { call } from "./client";
import {
  walkthroughViewSchema,
  walkthroughIndexEntrySchema,
  type WalkthroughView,
  type WalkthroughIndexEntry,
} from "./types";
import { z } from "zod";

export const walkthroughApi = {
  /** Get stored walkthrough, progress, and available translations for a game. */
  get: (gameId: string, vnId?: string | null): Promise<WalkthroughView | null> =>
    call("walkthrough_get", walkthroughViewSchema.nullable(), { gameId, vnId: vnId ?? null }),

  /** Search candidate entries on Seiya-Saiga's master index. */
  searchCandidate: (
    gameId: string,
    vnId?: string | null,
  ): Promise<WalkthroughIndexEntry[]> =>
    call("walkthrough_search_candidate", walkthroughIndexEntrySchema.array(), {
      gameId,
      vnId: vnId ?? null,
    }),

  /** Free-text search across Seiya-Saiga's master catalog. */
  search: (query: string): Promise<WalkthroughIndexEntry[]> =>
    call("walkthrough_search", walkthroughIndexEntrySchema.array(), { query }),

  /** Fetch and parse walkthrough from URL (or auto-match from index). */
  fetch: (
    gameId: string,
    vnId?: string | null,
    url?: string | null,
  ): Promise<WalkthroughView> =>
    call("walkthrough_fetch", walkthroughViewSchema, {
      gameId,
      vnId: vnId ?? null,
      url: url ?? null,
    }),

  /** Translate walkthrough routes using AI (with VNDB glossary) or Google Translate. */
  translate: (
    gameId: string,
    provider: "ai" | "google",
    targetLang: "en" | "id",
    vnId?: string | null,
  ): Promise<WalkthroughView> =>
    call("walkthrough_translate", walkthroughViewSchema, {
      gameId,
      provider,
      targetLang,
      vnId: vnId ?? null,
    }),

  /** Toggle a checked choice/step item. */
  toggleItem: (gameId: string, itemId: string): Promise<string[]> =>
    call("walkthrough_toggle_item", z.string().array(), { gameId, itemId }),

  /** Reset/clear all checked progress. */
  clearProgress: (gameId: string): Promise<void> =>
    call("walkthrough_clear_progress", z.void(), { gameId }),
};
