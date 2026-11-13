/**
 * Search helpers shared by the add sheet and the command palette.
 */

import { api } from "@/lib/ipc/client";
import type { SearchResponse } from "@/lib/ipc/types";
import { EXPLICIT_FLAG_LEVEL, flagLevelFromVote } from "@/lib/library";

/** Search VNDB. Thin wrapper so call sites do not import the client directly. */
export async function searchHits(query: string, page = 1): Promise<SearchResponse> {
  return api.vndbSearch(query, page);
}

/**
 * Whether a cover should be hidden under the current content setting.
 *
 * VNDB reports cover ratings as averages on a 0..2 scale, delivered here in
 * hundredths. This must agree with `isExplicit` in `@/lib/library`, which is the
 * rule the library itself applies; the two used to disagree (this one required a
 * rating of 2.0 or more) so a partly-rated explicit cover was concealed in the
 * library and shown in the add-game sheet. Level 1 is "the community voted this
 * explicit at all", which is the point a cover should stop being shown in full:
 * Sanoba Witch is rated 0.93 sexual on VNDB and is unquestionably explicit.
 *
 * Takes any payload that carries the two votes — search hits and For-you cards
 * alike — so one rounding serves every remote-entry surface.
 */
export function isExplicitHit(
  hit: { sexual: number | null; violence: number | null },
  mode: string,
): boolean {
  if (mode === "show") return false;
  return flagLevelFromVote(hit.sexual) >= EXPLICIT_FLAG_LEVEL ||
    flagLevelFromVote(hit.violence) >= EXPLICIT_FLAG_LEVEL;
}

