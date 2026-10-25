import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { LunaPanel } from "./LunaPanel";
import { lunaApi } from "@/lib/ipc/luna";
import { hookApi } from "@/lib/ipc/hook";
import { IpcError } from "@/lib/ipc/client";

vi.mock("@/lib/ipc/luna", () => ({
  lunaApi: {
    settings: vi.fn(),
    save: vi.fn(),
    status: vi.fn(),
    test: vi.fn(),
  },
}));

vi.mock("@/lib/ipc/hook", () => ({
  hookApi: {
    status: vi.fn(),
  },
}));

const offSettings = { enabled: false, baseUrl: "http://127.0.0.1:2333" };

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({ toasts: [], platform: null });
  vi.mocked(lunaApi.settings).mockResolvedValue(offSettings);
  vi.mocked(lunaApi.save).mockResolvedValue(offSettings);
  vi.mocked(lunaApi.status).mockResolvedValue({
    enabled: false,
    connected: false,
    url: null,
    lastLineAt: null,
    lastError: null,
  });
  vi.mocked(lunaApi.test).mockResolvedValue({ ok: true, detail: "" });
  vi.mocked(hookApi.status).mockResolvedValue({
    connected: false,
    pid: null,
    address: null,
    error: null,
    lineCount: 0,
    lines: [],
  });
});

function renderPanel() {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <LunaPanel />
    </AppContext.Provider>,
  );
}

describe("LunaPanel", () => {
  it("renders the title, the saved address and an off status by default", async () => {
    renderPanel();

    expect(await screen.findByText("LunaTranslator live text")).toBeInTheDocument();
    expect(screen.getByDisplayValue("http://127.0.0.1:2333")).toBeInTheDocument();
    expect(screen.getByText("Disabled")).toBeInTheDocument();
    expect(screen.getByText("Off")).toBeInTheDocument();
  });

  it("enabling saves the switch together with the address in the box", async () => {
    const user = userEvent.setup();
    vi.mocked(lunaApi.save).mockResolvedValue({ enabled: true, baseUrl: "http://127.0.0.1:2334" });

    renderPanel();
    await user.click(await screen.findByText("Disabled"));

    // The draft URL travels with the toggle: enabling a freshly typed port must
    // not connect to the previously saved one.
    await waitFor(() => {
      expect(lunaApi.save).toHaveBeenCalledWith({
        enabled: true,
        baseUrl: "http://127.0.0.1:2333",
      });
    });
  });

  it("saving persists the edited address", async () => {
    const user = userEvent.setup();
    vi.mocked(lunaApi.save).mockResolvedValue({
      enabled: false,
      baseUrl: "http://localhost:2334",
    });

    renderPanel();
    const input = await screen.findByDisplayValue("http://127.0.0.1:2333");
    await user.clear(input);
    await user.type(input, "http://localhost:2334");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(lunaApi.save).toHaveBeenCalledWith({
        enabled: false,
        baseUrl: "http://localhost:2334",
      });
    });
  });

  it("shows the validation error from a rejected save", async () => {
    const user = userEvent.setup();
    // Rust-side validation arrives as an IpcError carrying the reason.
    vi.mocked(lunaApi.save).mockRejectedValue(
      new IpcError("luna", "invalid_base_url: host must be localhost"),
    );

    renderPanel();
    await user.click(await screen.findByRole("button", { name: "Save" }));

    expect(
      await screen.findByText("invalid_base_url: host must be localhost"),
    ).toBeInTheDocument();
  });

  it("tests the address in the box and reports the probe", async () => {
    const user = userEvent.setup();
    vi.mocked(lunaApi.test).mockResolvedValue({ ok: false, detail: "connection refused" });

    renderPanel();
    const input = await screen.findByDisplayValue("http://127.0.0.1:2333");
    await user.clear(input);
    await user.type(input, "http://127.0.0.1:2334");
    await user.click(screen.getByRole("button", { name: "Test" }));

    await waitFor(() => {
      expect(lunaApi.test).toHaveBeenCalledWith("http://127.0.0.1:2334");
    });
    expect(await screen.findByText("No listener: connection refused")).toBeInTheDocument();
  });

  it("shows the connected state and the last received text", async () => {
    vi.mocked(lunaApi.settings).mockResolvedValue({ ...offSettings, enabled: true });
    vi.mocked(lunaApi.status).mockResolvedValue({
      enabled: true,
      connected: true,
      url: "ws://127.0.0.1:2333/api/ws/text/origin",
      lastLineAt: 1_760_000_000_000,
      lastError: null,
    });

    renderPanel();

    expect(await screen.findByText("Connected")).toBeInTheDocument();
    expect(
      await screen.findByText(/Last text received/, { exact: false }),
    ).toBeInTheDocument();
  });

  it("previews the recently captured ring inside its own container", async () => {
    vi.mocked(hookApi.status).mockResolvedValue({
      connected: false,
      pid: null,
      address: null,
      error: null,
      lineCount: 2,
      lines: [
        { text: "「おはよう、先輩」", thread: 0, at: 1_760_000_000_000 },
        { text: "Morning already?", thread: 0, at: 1_760_000_001_000 },
      ],
    });

    renderPanel();

    expect(await screen.findByText("Recently captured text:")).toBeInTheDocument();
    // The container carries the layout, so pin it rather than the strings alone.
    const preview = document.querySelector(".luna-preview__lines");
    expect(preview).not.toBeNull();
    expect(preview?.querySelectorAll(".luna-preview__line")).toHaveLength(2);
    expect(screen.getByText("Morning already?")).toBeInTheDocument();
  });

  it("shows no preview when the ring is empty", async () => {
    renderPanel();

    await screen.findByText("LunaTranslator live text");
    expect(screen.queryByText("Recently captured text:")).not.toBeInTheDocument();
  });
});
