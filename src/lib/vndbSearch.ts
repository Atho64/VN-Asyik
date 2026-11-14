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
 * Conceals explicit search and recommendation hits using the same threshold as
 * `isExplicit` in `@/lib/library` (`EXPLICIT_FLAG_LEVEL`).
 */
export function isExplicitHit(
  hit: { sexual: number | null; violence: number | null },
  mode: string,
): boolean {
  if (mode === "show") return false;
  return flagLevelFromVote(hit.sexual) >= EXPLICIT_FLAG_LEVEL ||
    flagLevelFromVote(hit.violence) >= EXPLICIT_FLAG_LEVEL;
}

