/** Validated IPC for local chat history and ordered live events. */
import { Channel } from "@tauri-apps/api/core";
import { z } from "zod";
import { call } from "./client";

export const chatOptionsSchema = z.object({
  gameId: z.string().nullable(),
  spoilers: z.enum(["none", "boundary", "all"]),
  boundary: z.string(),
  allowOnline: z.boolean(),
  language: z.enum(["en", "id"]),
  /** How the companion talks; the same facts and spoiler policy for every one. */
  persona: z.enum(["companion", "kawaii", "analyst", "playful", "concise"]),
});
export type ChatOptions = z.infer<typeof chatOptionsSchema>;
export const chatThreadSchema = z.object({
  id: z.string(),
  title: z.string(),
  options: chatOptionsSchema,
  createdAt: z.number(),
  updatedAt: z.number(),
});
export type ChatThread = z.infer<typeof chatThreadSchema>;
export const chatSourceSchema = z.object({
  id: z.string(),
  title: z.string(),
  url: z.string().nullable(),
  gameId: z.string().nullable(),
});
export const chatTraceSchema = z.object({
  id: z.string(),
  name: z.string(),
  arguments: z.string(),
  status: z.string(),
  result: z.unknown().nullable(),
});
export type ChatTrace = z.infer<typeof chatTraceSchema>;
export const chatMessageSchema = z.object({
  id: z.string(),
  role: z.enum(["user", "assistant"]),
  content: z.string(),
  status: z.enum(["complete", "running", "cancelled", "error"]),
  error: z.string().nullable(),
  tools: chatTraceSchema.array(),
  sources: chatSourceSchema.array(),
  tokens: z.number().nullable(),
  createdAt: z.number(),
});
export type ChatMessage = z.infer<typeof chatMessageSchema>;
export const chatEventSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("started"), message: chatMessageSchema }),
  z.object({ type: z.literal("delta"), text: z.string() }),
  z.object({ type: z.literal("tool"), trace: chatTraceSchema }),
  z.object({ type: z.literal("sources"), sources: chatSourceSchema.array() }),
]);
export type ChatEvent = z.infer<typeof chatEventSchema>;

export const chatApi = {
  list: () => call("chat_list", chatThreadSchema.array()),
  create: (options: ChatOptions) => call("chat_create", chatThreadSchema, { options }),
  messages: (id: string) => call("chat_messages", chatMessageSchema.array(), { id }),
  update: (id: string, title: string, options: ChatOptions) =>
    call("chat_update", chatThreadSchema, { id, title, options }),
  delete: (id: string) =>
    call(
      "chat_delete",
      z.null().transform(() => undefined),
      { id },
    ),
  cancel: (id: string) =>
    call(
      "chat_cancel",
      z.null().transform(() => undefined),
      { id },
    ),
  send: (
    id: string,
    text: string,
    retry: boolean,
    receive: (event: ChatEvent) => void,
  ) => {
    const onEvent = new Channel<unknown>((raw) => receive(chatEventSchema.parse(raw)));
    return call("chat_send", chatMessageSchema, { id, text, retry, onEvent });
  },
};
