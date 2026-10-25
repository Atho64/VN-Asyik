import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";

const mocks = vi.hoisted(() => ({ status: vi.fn() }));
vi.mock("@/lib/ipc/client", () => ({
  api: {
    vndbStatus: mocks.status,
    vndbQueue: vi.fn().mockResolvedValue([]),
    vndbLog: vi.fn().mockResolvedValue([]),
  },
  IpcError: class extends Error {},
}));
const { SyncScreen } = await import("./SyncScreen");

describe("VNDB connection loading", () => {
  it("shows a recoverable error instead of an endless skeleton when status fails", async () => {
    mocks.status.mockRejectedValueOnce(new Error("key store unavailable"));
    mocks.status.mockResolvedValue({
      storage_supported: true,
      account: null,
      pending: 0,
    });
    render(
      <AppContext.Provider
        value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
      >
        <SyncScreen />
      </AppContext.Provider>,
    );
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByLabelText("API token")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
