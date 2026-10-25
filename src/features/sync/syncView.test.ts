import { describe, expect, it } from "vitest";

import {
  decideAll,
  groupConflicts,
  summarize,
  undecidedCount,
} from "./syncView";
import type { PullResult, SyncRow } from "@/lib/ipc/types";

function row(vnId: string, fields: string[]): SyncRow {
  return {
    vn_id: vnId,
    game_id: `g-${vnId}`,
    title: `Title ${vnId}`,
    conflicts: fields.map((field) => ({
      field,
      local: "local",
      remote: "remote",
    })),
  };
}

const pull: PullResult = {
  entries: 5,
  not_imported: [
    { vnId: "v99", status: null, rating: null, notes: null, startedAt: null, finishedAt: null },
    { vnId: "v98", status: "completed", rating: 8, notes: null, startedAt: null, finishedAt: null },
  ],
  in_sync: 2,
  conflicts: [row("v1", ["status", "rating"]), row("v2", ["notes"])],
};

describe("groupConflicts", () => {
  it("counts conflicts per VN", () => {
    const grouped = groupConflicts(pull);
    expect(grouped).toHaveLength(2);
    expect(grouped[0]?.fieldCount).toBe(2);
    expect(grouped[1]?.fieldCount).toBe(1);
  });

  it("handles an empty pull", () => {
    expect(
      groupConflicts({ entries: 0, not_imported: [], in_sync: 0, conflicts: [] }),
    ).toEqual([]);
  });
});

describe("undecidedCount", () => {
  it("counts every undecided field", () => {
    expect(undecidedCount(pull, {})).toBe(3);
  });

  it("counts a skip as a decision", () => {
    expect(
      undecidedCount(pull, { v1: { status: "skip", rating: "skip" }, v2: { notes: "skip" } }),
    ).toBe(0);
  });

  it("counts partially decided fields", () => {
    expect(undecidedCount(pull, { v1: { status: "keep_local" } })).toBe(2);
  });

  it("ignores decisions for VNs that are not in the pull", () => {
    expect(undecidedCount(pull, { v404: { status: "keep_local" } })).toBe(3);
  });
});

describe("decideAll", () => {
  it("fills every field with the same resolution", () => {
    const decided = decideAll(pull, "keep_remote");
    expect(undecidedCount(pull, decided)).toBe(0);
    expect(decided.v1?.status).toBe("keep_remote");
    expect(decided.v2?.notes).toBe("keep_remote");
  });

  it("replaces previous decisions rather than merging them", () => {
    const first = decideAll(pull, "keep_local");
    const second = decideAll(pull, "skip");
    expect(first.v1?.status).toBe("keep_local");
    expect(second.v1?.status).toBe("skip");
  });
});

describe("summarize", () => {
  it("reports what the pull found", () => {
    const summary = summarize(pull);
    expect(summary.entries).toBe(5);
    expect(summary.conflicts).toBe(2);
    expect(summary.inSync).toBe(2);
    expect(summary.notImported).toBe(2);
    expect(summary.clean).toBe(false);
  });

  it("is clean when nothing needs a decision", () => {
    const summary = summarize({
      entries: 3,
      not_imported: [],
      in_sync: 3,
      conflicts: [],
    });
    expect(summary.clean).toBe(true);
  });
});
