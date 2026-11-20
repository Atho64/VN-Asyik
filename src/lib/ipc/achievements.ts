/**
 * Achievement catalog IPC: list, evaluate, the Discover-import marker, and the
 * unlock event. One door to Rust, every response parsed by its zod schema.
 *
 * The backend already auto-triggers playtime, streak, startup, sync and import
 * refreshes. `requestAchievementsRefresh` complements those for the non-playtime
 * mutations the frontend owns (endings, journal, notes, screenshots, goals); a
 * single shell listener coalesces the calls so a burst of edits evaluates once.
 */

import { listen } from "@tauri-apps/api/event";
import { z } from "zod";

import { call } from "./client";
import {
  achievementViewSchema,
  achievementsEvaluateSchema,
  achievementsUnlockedSchema,
  type AchievementView,
} from "./types";

/** `void` commands still resolve to `null` over IPC. */
const voidSchema = z.unknown().transform(() => undefined as void);

/** All 65 catalog items with live progress; locked items still show progress. */
export const achievementsList = (): Promise<AchievementView[]> =>
  call("achievements_list", z.array(achievementViewSchema));

/** Evaluate now; resolves to the slugs unlocked by this call (a bare array). */
export const achievementsEvaluate = (): Promise<string[]> =>
  call("achievements_evaluate", achievementsEvaluateSchema);

/** Record the F4 marker (a game imported from Discover), then refresh backend-side. */
export const achievementsNoteDiscoverImport = (): Promise<void> =>
  call("achievements_note_discover_import", voidSchema);

/**
 * Subscribe to `vn:achievements-unlocked`. The payload is validated before the
 * callback runs, so a malformed event cannot push a toast full of undefined.
 * Returns the Tauri unlisten function, like the screenshot shortcut listener.
 */
export const listenAchievementsUnlocked = (onUnlocked: (slugs: string[]) => void) =>
  listen<unknown>("vn:achievements-unlocked", ({ payload }) => {
    const parsed = achievementsUnlockedSchema.safeParse(payload);
    if (parsed.success && parsed.data.slugs.length > 0) onUnlocked(parsed.data.slugs);
  });

/** Window event a mutation dispatches to ask the shell for one coalesced evaluate. */
export const ACHIEVEMENTS_REFRESH_EVENT = "vn:achievements-refresh";

/**
 * Ask for an achievement refresh after a non-playtime mutation. Mirrors the
 * `vn:tracking-changed` pattern PlayButton uses: fire-and-forget, one listener.
 */
export function requestAchievementsRefresh(): void {
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent(ACHIEVEMENTS_REFRESH_EVENT));
  }
}