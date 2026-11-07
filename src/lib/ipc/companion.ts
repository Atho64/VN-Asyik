import { listen } from "@tauri-apps/api/event";
import { z } from "zod";
import { call } from "./client";
import { chatMessageSchema, chatOptionsSchema } from "./chat";

export const presetSchema = z.enum([
  "companion",
  "kawaii",
  "analyst",
  "playful",
  "concise",
  "quiet_listener",
  "hype_friend",
  "mystery_partner",
  "romance_enthusiast",
  "cozy_book_club",
  "language_buddy",
  "wry_rival",
]);
export const profileSettingsSchema = z.object({
  name: z.string(),
  preset: presetSchema,
  style: z.string(),
  interests: z.string(),
  length: z.enum(["short", "balanced", "detailed"]),
  humor: z.number().int().min(0).max(2),
  energy: z.number().int().min(0).max(2),
  warmth: z.number().int().min(0).max(2),
  questions: z.number().int().min(0).max(2),
  language: z.enum(["app", "en", "id", "ja"]),
});
export const profileSnapshotSchema = z.object({
  id: z.string(),
  version: z.number().int().positive(),
  settings: profileSettingsSchema,
});
export const profileCatalogueSchema = z.object({
  presets: profileSnapshotSchema.array(),
  profiles: profileSnapshotSchema.array(),
});
export type ProfileSettings = z.infer<typeof profileSettingsSchema>;
export type ProfileSnapshot = z.infer<typeof profileSnapshotSchema>;
export type ProfileCatalogue = z.infer<typeof profileCatalogueSchema>;

export const sessionOptionsSchema = z.object({
  gameId: z.string(),
  playthroughLabel: z.string(),
  knowledge: z.enum(["metadata", "observed", "all"]),
  boundary: z.string(),
  source: z.enum(["manual", "luna"]),
  language: z.string(),
  persona: chatOptionsSchema.shape.persona,
  profile: profileSnapshotSchema.nullish(),
});
export type SessionOptions = z.infer<typeof sessionOptionsSchema>;

export const companionSessionSchema = z.object({
  replayExcluded: z.boolean().default(false),
  id: z.string(),
  threadId: z.string(),
  playthroughId: z.string(),
  gameTitle: z.string().nullable(),
  options: sessionOptionsSchema,
  sourceUrl: z.string().nullable(),
  state: z.enum(["active", "paused", "ended"]),
  reason: z.string().nullable(),
  createdAt: z.number(),
  updatedAt: z.number(),
  endedAt: z.number().nullable(),
});
export type CompanionSession = z.infer<typeof companionSessionSchema>;

const availabilitySchema = z.enum(["available", "unsupported"]);

export const visionViewSchema = z.object({
  status: z.string(),
  error: z.string().nullable(),
  window: z.string().nullable(),
  provider: z
    .object({ provider: z.string(), baseUrl: z.string(), model: z.string() })
    .nullable(),
  generation: z.number(),
  support: z.enum(["unknown", "testing", "accepted", "unavailable"]),
  hourly: z.number(),
  total: z.number(),
  watching: z.boolean(),
  quiet: z.boolean(),
  automatic: z.boolean(),
});
export const emptyVision = {
  status: "off",
  error: null,
  window: null,
  provider: null,
  generation: 0,
  support: "unknown" as const,
  hourly: 0,
  total: 0,
  watching: false,
  quiet: false,
  automatic: false,
};
export const companionSnapshotSchema = z.object({
  vision: visionViewSchema.default(emptyVision),
  requestId: z.string().nullable(),
  runtimeId: z.string(),
  sequence: z.number(),
  epoch: z.number(),
  lifecycle: z.enum(["off", "starting", "active", "paused", "ending"]),
  session: companionSessionSchema.nullable(),
  busy: z.boolean(),
  live: chatMessageSchema.nullable(),
  source: z.object({
    state: z.enum([
      "off",
      "manual",
      "waiting",
      "receiving",
      "stale",
      "disconnected",
      "paused",
    ]),
    lastLineAt: z.number().nullable(),
    lineCount: z.number(),
    preview: z.string().array(),
  }),
  error: z.string().nullable(),
  processId: z.number().nullable(),
  capabilities: z.object({
    overlay: availabilitySchema,
    text: availabilitySchema,
    luna: availabilitySchema,
    backgroundRuntime: availabilitySchema,
    processLifecycle: availabilitySchema,
    vision: availabilitySchema,
    speech: availabilitySchema,
    durableMemory: availabilitySchema,
  }),
});
export type CompanionSnapshot = z.infer<typeof companionSnapshotSchema>;

export const companionNoticeSchema = z.object({
  runtimeId: z.string(),
  sequence: z.number(),
  sessionId: z.string().nullable(),
  epoch: z.number(),
});
export type CompanionNotice = z.infer<typeof companionNoticeSchema>;
export type CompanionAction = "pause" | "resume" | "end" | "cancel_turn";

export const companionApi = {
  profiles: () => call("companion_profiles", profileCatalogueSchema),
  saveProfile: (
    id: string | null,
    expectedVersion: number | null,
    settings: ProfileSettings,
  ) =>
    call("companion_save_profile", profileSnapshotSchema, {
      id,
      expectedVersion,
      settings,
    }),
  deleteProfile: (id: string, version: number) =>
    call(
      "companion_delete_profile",
      z.null().transform(() => undefined),
      { id, version },
    ),
  applyProfile: (id: string, profile: ProfileSnapshot) =>
    call("companion_apply_profile", companionSnapshotSchema, { id, profile }),
  snapshot: () => call("companion_snapshot", companionSnapshotSchema),
  start: (id: string, options: SessionOptions) =>
    call("companion_start", companionSnapshotSchema, { id, options }),
  control: (id: string, action: CompanionAction) =>
    call("companion_control", companionSnapshotSchema, { id, action }),
  send: (id: string, requestId: string, text: string) =>
    call("companion_send", companionSnapshotSchema, { id, requestId, text }),
  list: () => call("companion_list", companionSessionSchema.array()),
  messages: (id: string) => call("companion_messages", chatMessageSchema.array(), { id }),
  delete: (id: string) =>
    call(
      "companion_delete",
      z.null().transform(() => undefined),
      { id },
    ),
  listen: (receive: (notice: CompanionNotice) => void) =>
    listen<unknown>("companion://changed", (event) =>
      receive(companionNoticeSchema.parse(event.payload)),
    ),
};
