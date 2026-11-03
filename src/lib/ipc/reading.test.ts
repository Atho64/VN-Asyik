import { describe, expect, it } from "vitest";
import { readingContextSchema } from "./reading";
import { gameSessionsPageSchema } from "./types";

describe("reading IPC", () => {
  it("keeps note previews separate from sessions and accepts older responses", () => {
    const page = { sessions: [], totalSessions: 0, countedMs: 0, afkMs: 0 };
    expect(gameSessionsPageSchema.parse(page).notePreviews).toEqual({});
    expect(
      gameSessionsPageSchema.parse({
        ...page,
        notePreviews: { session: "Continue here" },
      }).notePreviews,
    ).toEqual({ session: "Continue here" });
  });
  it("accepts camelCase context and snake_case portable journal rows", () => {
    const result = readingContextSchema.parse({
      latestSession: {
        id: "session",
        startedAt: 1,
        endedAt: 2,
        countedMs: 1,
        routeName: null,
        chapterName: null,
      },
      entries: [
        {
          meta: {
            id: "entry",
            updated_at: 3,
            device_id_device: "device",
            deleted: false,
          },
          game_id: "game",
          session_id: "session",
          created_at: 3,
          played_at: 2,
          note: "Continue here",
          save_slot: "12",
          route_name: "",
          chapter_name: "",
        },
      ],
      hasMore: false,
    });
    expect(result.entries[0]?.save_slot).toBe("12");
    expect(readingContextSchema.safeParse({ ...result, latestSession: {} }).success).toBe(
      false,
    );
  });
});
