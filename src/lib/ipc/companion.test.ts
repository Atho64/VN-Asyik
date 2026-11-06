import { describe, expect, it } from "vitest";
import {
  companionProfiles,
  companionSession,
  companionSnapshot,
} from "@/test/companionFixture";
import { companionSnapshotSchema, sessionOptionsSchema } from "./companion";

describe("companion profile wire compatibility", () => {
  it("accepts legacy session options and retains a new profile snapshot", () => {
    expect(
      sessionOptionsSchema.parse(companionSession().options).profile,
    ).toBeUndefined();
    const snapshot = companionSnapshot();
    const profile = companionProfiles().presets[7]!;
    snapshot.session!.options.profile = profile;
    expect(companionSnapshotSchema.parse(snapshot).session?.options.profile).toEqual(
      profile,
    );
  });

  it("rejects an invalid version or unbounded personality level", () => {
    const options = companionSession().options;
    const profile = companionProfiles().presets[0]!;
    expect(
      sessionOptionsSchema.safeParse({ ...options, profile: { ...profile, version: 0 } })
        .success,
    ).toBe(false);
    expect(
      sessionOptionsSchema.safeParse({
        ...options,
        profile: { ...profile, settings: { ...profile.settings, humor: 3 } },
      }).success,
    ).toBe(false);
  });
});
