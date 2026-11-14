import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";

import { AdjustPlaytimeSheet } from "./GameDetailScreen";

function renderSheet(overrides: {
  manualPlaytimeMs?: number;
  onSave?: (ms: number) => Promise<boolean>;
} = {}) {
  const onSave =
    overrides.onSave ?? vi.fn(async () => true);
  render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <AdjustPlaytimeSheet
        open
        trackedMs={7_200_000}
        manualPlaytimeMs={overrides.manualPlaytimeMs ?? 0}
        onClose={() => {}}
        onSave={onSave}
      />
    </AppContext.Provider>,
  );
  return { onSave };
}

describe("AdjustPlaytimeSheet", () => {
  it("shows the tracked time and the current adjustment it edits", () => {
    // A negative adjustment must read as negative: the shared formatter
    // clamps to "0 min", so the sheet renders the sign itself.
    renderSheet({ manualPlaytimeMs: -3_600_000 });
    expect(screen.getByText("2h")).toBeInTheDocument();
    expect(screen.getByText("-1h")).toBeInTheDocument();
  });

  it("seeds the input from the saved adjustment when it opens", () => {
    // A stale figure from a previous visit must not look current.
    renderSheet({ manualPlaytimeMs: 7_800_000 });
    const input = screen.getByLabelText("Adjustment in minutes");
    expect((input as HTMLInputElement).value).toBe("130");
  });

  it("saves a negative correction in whole minutes", async () => {
    const user = userEvent.setup();
    const { onSave } = renderSheet();
    const input = screen.getByLabelText("Adjustment in minutes");
    await user.clear(input);
    await user.type(input, "-45");
    await user.click(screen.getByRole("button", { name: "Save session" }));
    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith(-2_700_000);
    });
  });

  it("refuses a fraction of a minute instead of silently rounding", async () => {
    const user = userEvent.setup();
    const { onSave } = renderSheet();
    const input = screen.getByLabelText("Adjustment in minutes");
    await user.clear(input);
    await user.type(input, "1.5");
    await user.click(screen.getByRole("button", { name: "Save session" }));
    expect(
      await screen.findByText("Enter a whole number of minutes."),
    ).toBeInTheDocument();
    expect(onSave).not.toHaveBeenCalled();
  });
});
