/**
 * Wire-shape guards for the IPC boundary.
 *
 * These exist because a serde `rename_all = "camelCase"` on the Rust side and a
 * snake_case zod schema on this side is not a compile error — it is a runtime
 * `contract` failure that the caller can swallow. That is exactly how a saved
 * custom cover rendered as no art at all: Rust sent `coverDataUrl`, the schema
 * required `cover_data_url`, and the rejection was quietly turned into `null`.
 *
 * Each case below pins the exact key set the Rust structs serialize, so a future
 * rename on either side fails here instead of in the UI.
 */
import { describe, expect, it } from "vitest";

import {
  localArtSchema,
  presenceActivitySchema,
  presenceSettingsSchema,
  presenceStatusSchema,
} from "./types";

describe("localArtSchema", () => {
  it("accepts the snake_case shape both art commands return", () => {
    // `vndb_local_art` and `game_custom_art` both return a Rust `LocalArt` /
    // `CustomArt` with these two fields. Neither struct may rename them.
    const parsed = localArtSchema.safeParse({
      cover_data_url: "data:image/jpeg;base64,AAA",
      backdrop_data_url: "data:image/jpeg;base64,BBB",
    });
    expect(parsed.success).toBe(true);
  });

  it("accepts nulls for artwork that is not cached", () => {
    const parsed = localArtSchema.safeParse({
      cover_data_url: null,
      backdrop_data_url: null,
    });
    expect(parsed.success).toBe(true);
  });

  it("rejects a camelCase payload instead of silently dropping the cover", () => {
    // The bug this guards: a `rename_all = "camelCase"` attribute produced this
    // shape, every field parsed as undefined, and the cover vanished.
    const parsed = localArtSchema.safeParse({
      coverDataUrl: "data:image/jpeg;base64,AAA",
      backdropDataUrl: "data:image/jpeg;base64,BBB",
    });
    expect(parsed.success).toBe(false);
  });
});

describe("presenceSettingsSchema", () => {
  it("accepts the snake_case payload serialized by Rust", () => {
    const parsed = presenceSettingsSchema.safeParse({
      enabled: true,
      client_id: "1561366091559731201",
      details_mode: "vndb",
      manual_details: "",
      manual_state: "",
      custom_details: "{vn_name}",
      custom_state: "{chapter}",
      show_cover: true,
      show_buttons: true,
      show_list_button: false,
      show_timestamps: true,
      show_when_unfocused: true,
      activity_name: null,
      cycle_vndb_details: true,
      cycle_seconds: 5,
      custom_details_items: [],
      custom_state_items: [],
      custom_details_rotate: false,
      custom_state_rotate: false,
      custom_details_cycle_seconds: 5,
      custom_state_cycle_seconds: 5,
      vndb_details_items: [],
      vndb_state_template: "",
      vndb_swap_details_state: false,
      custom_details_transform: {
        prefix: "",
        suffix: "",
        remove: [],
        replace: [],
        trim: false,
      },
      custom_state_transform: {
        prefix: "",
        suffix: "",
        remove: [],
        replace: [{ old: "Playing", new: "Reading" }],
        trim: true,
      },
      vndb_details_transform: {
        prefix: "",
        suffix: "",
        remove: [],
        replace: [],
        trim: false,
      },
      vndb_state_transform: {
        prefix: "",
        suffix: "",
        remove: [],
        replace: [],
        trim: false,
      },
      my_vndb_link: "",
      vndb_loading_details: "Reading",
      vndb_loading_state: "Loading VNDB data...",
      rpc_image_host: "none",
    });
    expect(parsed.success).toBe(true);
  });

  it("rejects camelCase payload", () => {
    const parsed = presenceSettingsSchema.safeParse({
      enabled: true,
      clientId: "1561366091559731201",
      detailsMode: "vndb",
      showCover: true,
    });
    expect(parsed.success).toBe(false);
  });
});

describe("presenceStatusSchema", () => {
  it("accepts tagged status variants", () => {
    expect(presenceStatusSchema.safeParse({ status: "disabled" }).success).toBe(true);
    expect(presenceStatusSchema.safeParse({ status: "connected" }).success).toBe(true);
    expect(
      presenceStatusSchema.safeParse({ status: "discord_not_running" }).success,
    ).toBe(true);
    expect(
      presenceStatusSchema.safeParse({ status: "error", message: "socket fail" }).success,
    ).toBe(true);
  });
});

describe("presenceActivitySchema", () => {
  it("accepts valid presence activity with buttons and snake_case fields", () => {
    const parsed = presenceActivitySchema.safeParse({
      name: "Steins;Gate",
      details: "5pb. / Nitroplus",
      state: "Chapter 1",
      large_image: "https://t.vndb.org/cv/98/1098.jpg",
      large_text: "Steins;Gate",
      small_image: null,
      small_text: null,
      start_timestamp: 1700000000,
      buttons: [{ label: "VNDB Page", url: "https://vndb.org/v2002" }],
    });
    expect(parsed.success).toBe(true);
  });
});
