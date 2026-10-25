/** Validated IPC for imported game scripts and their compaction progress. */
import { z } from "zod";
import { call } from "./client";

/** Mirrors `script::CompactionInfo`. `digest` is set once every chunk is done. */
export const compactionInfoSchema = z.object({
  modelId: z.string(),
  chunkCount: z.number(),
  doneChunks: z.number(),
  agentCount: z.number(),
  digest: z.string().nullable(),
});
export type CompactionInfo = z.infer<typeof compactionInfoSchema>;

/** How many summarizer agents compaction runs in parallel (1..=10). */
export const agentSettingsSchema = z.object({
  agents: z.number(),
});
export type AgentSettings = z.infer<typeof agentSettingsSchema>;

/** Mirrors `vn_core::script::DEFAULT_AGENTS`; the UI fallback while loading. */
export const DEFAULT_AGENT_COUNT = 3;

/** One stored script document. The body itself never crosses IPC; only sizes. */
export const scriptDocumentSchema = z.object({
  id: z.string(),
  gameId: z.string(),
  engine: z.string(),
  sourceKind: z.enum(["plain_text", "json_vntp", "asyik_script"]),
  sourcePath: z.string(),
  sourceSha256: z.string(),
  lineCount: z.number(),
  byteSize: z.number(),
  createdAt: z.number(),
  compaction: compactionInfoSchema.nullable(),
});
export type ScriptDocument = z.infer<typeof scriptDocumentSchema>;

export const scriptApi = {
  /**
   * Import script file(s) as one document; each file's format is detected from
   * its content and several files are merged in natural file-name order.
   */
  import: (gameId: string, paths: string[]) =>
    call("script_import", scriptDocumentSchema, { gameId, paths }),
  /** Stored documents, newest first. */
  list: (gameId?: string) =>
    call("script_list", scriptDocumentSchema.array(), { gameId: gameId ?? null }),
  delete: (id: string) =>
    call(
      "script_delete",
      z.null().transform(() => undefined),
      { id },
    ),
  /**
   * Summarize up to `maxChunks` pending chunks per agent (8 by default) and
   * reduce them into a digest once all are done. Repeat until
   * `doneChunks === chunkCount && digest !== null`; progress survives restarts.
   */
  compact: (id: string, maxChunks?: number) =>
    call("script_compact", compactionInfoSchema, { id, maxChunks: maxChunks ?? null }),
  /** How many summarizer agents compaction runs in parallel. */
  agentSettings: () => call("script_agent_settings", agentSettingsSchema, {}),
  /** Persist the agent count; the backend clamps it into 1..=10. */
  saveAgentSettings: (settings: AgentSettings) =>
    call("script_save_agent_settings", agentSettingsSchema, { settings }),
};
