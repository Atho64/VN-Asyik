/** Validated IPC for the live text ring the companion chat reads. */
import { z } from "zod";
import { call } from "./client";

/** One captured dialogue line. `thread` tells names and dialogue apart. */
export const capturedLineSchema = z.object({
  text: z.string(),
  thread: z.number(),
  at: z.number(),
});
export type CapturedLine = z.infer<typeof capturedLineSchema>;

/** `HookView`: flattened ring status plus the recent captured lines. */
export const hookStatusSchema = z.object({
  connected: z.boolean(),
  pid: z.number().nullable(),
  address: z.string().nullable(),
  error: z.string().nullable(),
  lineCount: z.number(),
  lines: capturedLineSchema.array(),
});
export type HookStatusView = z.infer<typeof hookStatusSchema>;

export const hookApi = {
  /**
   * The live text ring. `LunaTranslator`'s network service is its only writer
   * now that in-process hooking is gone, so `pid` and `address` are always null;
   * they stay in the shape because the panel reads the whole object.
   */
  status: () => call("hook_status", hookStatusSchema),
};
