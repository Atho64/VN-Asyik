/** Validated IPC for the `LunaTranslator` live-text source. */
import { z } from "zod";
import { call } from "./client";

/** Mirrors `tracking::luna::LunaSettings` (`deny_unknown_fields`, camelCase). */
export const lunaSettingsSchema = z.object({
  enabled: z.boolean(),
  baseUrl: z.string(),
});
export type LunaSettings = z.infer<typeof lunaSettingsSchema>;

/** Connection state of the websocket monitor, polled by the settings card. */
export const lunaStatusSchema = z.object({
  enabled: z.boolean(),
  connected: z.boolean(),
  url: z.string().nullable(),
  lastLineAt: z.number().nullable(),
  lastError: z.string().nullable(),
});
export type LunaStatus = z.infer<typeof lunaStatusSchema>;

/** One-shot reachability probe of a base URL. */
export const lunaProbeSchema = z.object({
  ok: z.boolean(),
  detail: z.string(),
});
export type LunaProbe = z.infer<typeof lunaProbeSchema>;

export const lunaApi = {
  settings: () => call("luna_settings", lunaSettingsSchema),
  /** Validate, persist and apply; the monitor reconnects without a restart. */
  save: (settings: LunaSettings) =>
    call("luna_save_settings", lunaSettingsSchema, { settings }),
  status: () => call("luna_status", lunaStatusSchema),
  /** Probe a candidate base URL (or the saved one when omitted). */
  test: (baseUrl?: string) =>
    call("luna_test", lunaProbeSchema, { baseUrl: baseUrl ?? null }),
};
