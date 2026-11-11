/** Reading checkpoints and their validated Rust boundary. */
import { z } from "zod";
import { call } from "./client";
import { gameSchema } from "./types";

export const readingEntrySchema = z.object({
  meta: z.object({
    id: z.string(),
    updated_at: z.number(),
    device_id_device: z.string(),
    deleted: z.boolean(),
  }),
  game_id: z.string(),
  session_id: z.string().nullable(),
  created_at: z.number(),
  played_at: z.number().nullable(),
  note: z.string(),
  save_slot: z.string(),
  route_name: z.string(),
  chapter_name: z.string(),
  /**
   * VNDB image id of the screenshot pinned to this checkpoint, or empty.
   *
   * Defaulted, not required: a payload written before the column existed has no
   * field, and a missing photo is exactly what that means.
   */
  photo_image_id: z.string().default(""),
});
export type ReadingEntry = z.infer<typeof readingEntrySchema>;

export const readingSessionSchema = z.object({
  id: z.string(),
  startedAt: z.number(),
  endedAt: z.number(),
  countedMs: z.number(),
  afkMs: z.number().default(0),
  routeName: z.string().nullable(),
  chapterName: z.string().nullable(),
});
export type ReadingSession = z.infer<typeof readingSessionSchema>;

export const readingContextSchema = z.object({
  latestSession: readingSessionSchema.nullable(),
  entries: z.array(readingEntrySchema),
  hasMore: z.boolean(),
});
export type ReadingContext = z.infer<typeof readingContextSchema>;

export const sessionReadingContextSchema = z.object({
  session: readingSessionSchema,
  entries: z.array(readingEntrySchema),
  hasMore: z.boolean(),
});
export type SessionReadingContext = z.infer<typeof sessionReadingContextSchema>;

export interface SaveReadingInput {
  id: string | null;
  expected_updated_at: number | null;
  game_id: string;
  session_id: string | null;
  note: string;
  save_slot: string;
  route_name: string;
  chapter_name: string;
  /** VNDB image id of a pinned screenshot, or empty for a text-only checkpoint. */
  photo_image_id: string;
}

/** One search hit: the checkpoint and its live parent game, for the row's cover. */
export const journalSearchResultSchema = z.object({
  entry: readingEntrySchema,
  game: gameSchema,
});
export type JournalSearchResult = z.infer<typeof journalSearchResultSchema>;

export const journalSearchPageSchema = z.object({
  results: journalSearchResultSchema.array(),
  hasMore: z.boolean(),
});
export type JournalSearchPage = z.infer<typeof journalSearchPageSchema>;

export const readingApi = {
  session: (gameId: string, sessionId: string, offset = 0) =>
    call("reading_session_context", sessionReadingContextSchema, {
      gameId,
      sessionId,
      offset,
    }),
  context: (gameId: string, offset = 0) =>
    call("reading_context", readingContextSchema, { gameId, offset }),
  save: (input: SaveReadingInput) => call("reading_save", readingEntrySchema, { input }),
  remove: (gameId: string, entry: ReadingEntry) =>
    call("reading_delete", z.null(), {
      gameId,
      id: entry.meta.id,
      expectedUpdatedAt: entry.meta.updated_at,
    }),
  /**
   * Search checkpoints across the library, or within one game, newest first.
   *
   * An empty query returns an empty page: clearing the box returns to the feed
   * without a failure flash. Pages walk the same `(ts, id)` cursor the timeline
   * uses, so results arriving mid-read never shift the older ones.
   */
  search: (query: string, gameId?: string, before?: { ts: number; id: string }) =>
    call("journal_search", journalSearchPageSchema, { query, gameId, before }),
};
