import { z } from "zod";
import { call } from "./client";

const knowledge = z.enum(["metadata", "observed", "all"]);
const scope = z.enum(["shared", "companion", "reading"]);
const memoryClass = z.enum([
  "preference",
  "continuity",
  "episode",
  "observation",
  "summary",
]);
const context = z.object({
  companionId: z.string(),
  gameId: z.string(),
  playthroughId: z.string(),
  knowledge,
});
export const memorySchema = z.object({
  id: z.string(),
  version: z.number().int().positive(),
  class: memoryClass,
  scope,
  companionId: z.string().nullable(),
  gameId: z.string().nullable(),
  playthroughId: z.string().nullable(),
  knowledge,
  body: z.string(),
  evidence: z.enum(["reader", "passage", "inference"]),
  pinned: z.boolean(),
  originSession: z.string().nullable(),
  createdAt: z.number(),
  updatedAt: z.number(),
  expiresAt: z.number().nullable(),
});
const jobSchema = z.object({
  id: z.string(),
  originSession: z.string(),
  context,
  state: z.enum(["pending", "running", "paused", "review"]),
  output: z.string(),
  error: z.string().nullable(),
  model: z.string(),
  automatic: z.boolean(),
  attempts: z.number(),
  fromPosition: z.number(),
  throughPosition: z.number(),
  createdAt: z.number(),
  expiresAt: z.number(),
});
export const memoryViewSchema = z.object({
  context: context.nullable(),
  settings: z.object({
    revision: z.number(),
    retrievalEnabled: z.boolean(),
    autoSummary: z.boolean(),
    gap: z.string().nullable(),
  }),
  items: memorySchema.array(),
  jobs: jobSchema.array(),
  replayExcluded: z.boolean(),
  itemCount: z.number(),
  pinCount: z.number(),
  markerCount: z.number(),
});
const sourceSchema = z.object({
  kind: z.string(),
  originSession: z.string().nullable(),
  messageId: z.string().nullable(),
  knowledge,
  evidence: z.string(),
});
export type Memory = z.infer<typeof memorySchema>;
export type MemoryView = z.infer<typeof memoryViewSchema>;
export type MemorySource = z.infer<typeof sourceSchema>;
export type MemoryJob = z.infer<typeof jobSchema>;
export type MemorySave = {
  expectedCompanionId: string | null;
  id: string | null;
  expectedVersion: number | null;
  scope: Memory["scope"];
  class: Memory["class"];
  body: string;
  pinned: boolean;
  sessionId: string | null;
  messageId: string | null;
  requestId: string;
  expectedRevision: number;
};
export type ForgetTarget =
  | { kind: "item"; id: string; version: number }
  | { kind: "session" | "reading" | "companion"; id: string }
  | { kind: "everything" };
export const memoryApi = {
  view: (session: string | null) =>
    call("companion_memory_view", memoryViewSchema, { session }),
  save: (request: MemorySave) => call("companion_memory_save", memorySchema, { request }),
  forget: (target: ForgetTarget, revision: number) =>
    call("companion_memory_forget", z.null(), { target, revision }),
  settings: (retrieval: boolean, automatic: boolean, revision: number) =>
    call("companion_memory_settings", z.null(), { retrieval, automatic, revision }),
  sources: (id: string, version: number) =>
    call("companion_memory_sources", sourceSchema.array(), { id, version }),
  suggest: (session: string) => call("companion_recap", z.null(), { session }),
  resume: (id: string) => call("companion_recap_resume", z.null(), { id }),
  review: (id: string, body: string | null, pinned: boolean, revision: number) =>
    call("companion_recap_review", z.null(), { id, body, pinned, revision }),
};
