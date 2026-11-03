/** Reading checkpoints and their validated Rust boundary. */
import { z } from "zod";
import { call } from "./client";

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
}

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
};
