import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";

import { Sheet } from "./Sheet";
import { ProgressRing } from "./ProgressRing";
import { EmptyState } from "./EmptyState";
import { StatusChip } from "./Chip";
import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import type { ReactNode } from "react";

const t = createTranslator("en");

function withProviders(ui: ReactNode) {
  return (
    <AppContext.Provider
      value={{ t, locale: "en", ready: true, bootError: null }}
    >
      <MemoryRouter>{ui}</MemoryRouter>
    </AppContext.Provider>
  );
}

describe("Sheet", () => {
  it("is not rendered when closed", () => {
    render(
      withProviders(
        <Sheet open={false} title="Hidden" onClose={() => {}}>
          <p>body</p>
        </Sheet>,
      ),
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("renders as a labelled modal dialog when open", () => {
    render(
      withProviders(
        <Sheet open title="Add a game" onClose={() => {}}>
          <input aria-label="Title" />
        </Sheet>,
      ),
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveAccessibleName("Add a game");
  });

  it("closes on Escape", async () => {
    const onClose = vi.fn();
    render(
      withProviders(
        <Sheet open title="Add a game" onClose={onClose}>
          <input aria-label="Title" />
        </Sheet>,
      ),
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes via the labelled close button", async () => {
    const onClose = vi.fn();
    render(
      withProviders(
        <Sheet open title="Add a game" onClose={onClose}>
          <p>body</p>
        </Sheet>,
      ),
    );
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("ProgressRing", () => {
  it("exposes the percentage as an accessible name", () => {
    render(<ProgressRing progress={0.42} />);
    expect(screen.getByRole("img")).toHaveAccessibleName("42%");
  });

  it("clamps out-of-range values instead of wrapping", () => {
    render(<ProgressRing progress={1.8} title="full" />);
    expect(screen.getByRole("img")).toHaveAccessibleName("full");
  });

  it("treats a non-finite value as zero", () => {
    render(<ProgressRing progress={Number.NaN} />);
    expect(screen.getByRole("img")).toHaveAccessibleName("0%");
  });
});

describe("EmptyState", () => {
  it("names the cause and offers the action", () => {
    render(
      <EmptyState
        title="Nothing here yet"
        body="Add a game to start tracking."
        action={<button type="button">Add a game</button>}
      />,
    );
    expect(screen.getByRole("heading", { name: "Nothing here yet" })).toBeTruthy();
    expect(screen.getByText("Add a game to start tracking.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add a game" })).toBeTruthy();
  });
});

describe("StatusChip", () => {
  it("carries the status as text, not colour alone", () => {
    render(withProviders(<StatusChip status="on_hold" />));
    expect(screen.getByText("On hold")).toBeTruthy();
  });
});
