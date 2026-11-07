import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router";
import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import {
  companionGame,
  companionMessage,
  companionSession,
  companionSnapshot,
  companionProfiles,
} from "@/test/companionFixture";
import type { CompanionSnapshot } from "@/lib/ipc/companion";

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  messages: vi.fn(),
  control: vi.fn(),
  start: vi.fn(),
  snapshot: vi.fn(),
  send: vi.fn(),
  delete: vi.fn(),
  listen: vi.fn(),
  provider: vi.fn(),
  luna: vi.fn(),
  open: vi.fn(),
  hide: vi.fn(),
  showMain: vi.fn(),
  profiles: vi.fn(),
  saveProfile: vi.fn(),
  deleteProfile: vi.fn(),
  applyProfile: vi.fn(),
}));
vi.mock("@/lib/ipc/companionWindow", () => ({ companionWindowApi: mocks }));
vi.mock("@/lib/ipc/companion", async () => ({
  ...(await vi.importActual("@/lib/ipc/companion")),
  companionApi: mocks,
}));
vi.mock("@/lib/ipc/client", () => ({
  IpcError: class extends Error {},
  api: { aiLlmProfiles: mocks.provider },
}));
vi.mock("@/lib/ipc/luna", () => ({ lunaApi: { status: mocks.luna } }));
const { CompanionScreen } = await import("./CompanionScreen");
const { CompanionSetup } = await import("./CompanionSetup");
const { CompanionBar } = await import("./CompanionBar");
const { CompanionOverlay } = await import("./CompanionOverlay");
const { acceptSnapshot, useCompanionStore } = await import("./store");

function view(ui: React.ReactNode, path = "/companion") {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <AppContext.Provider
        value={{
          t: createTranslator("en"),
          locale: "en",
          ready: true,
          bootError: null,
        }}
      >
        {ui}
      </AppContext.Provider>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  useCompanionStore.setState(useCompanionStore.getInitialState(), true);
  useAppStore.setState({ cards: [companionGame()] });
  mocks.list.mockResolvedValue([companionSession()]);
  mocks.messages.mockResolvedValue([]);
  mocks.profiles.mockResolvedValue(companionProfiles());
  mocks.luna.mockResolvedValue({ enabled: true, connected: true });
  mocks.provider.mockResolvedValue({
    active: "test",
    supported: true,
    profiles: [
      {
        id: "test",
        name: "Local test",
        model: "mock",
        baseUrl: "http://127.0.0.1:9",
        hasKey: false,
        keyHint: null,
        provider: "openai",
      },
    ],
  });
  mocks.listen.mockResolvedValue(vi.fn());
  mocks.snapshot.mockResolvedValue(companionSnapshot());
});

describe("companion profiles", () => {
  it("does not start a session while profile edits are unsaved", async () => {
    const user = userEvent.setup();
    const start = vi.fn(async () => {});
    view(
      <CompanionSetup
        gameId="game-one"
        replacing={false}
        pending={false}
        onStart={start}
      />,
    );
    await screen.findByRole("option", { name: "Mystery partner" });
    await user.click(screen.getByRole("button", { name: "Customize" }));
    await user.type(screen.getByLabelText("Profile name"), " draft{enter}");
    expect(screen.getByRole("button", { name: "Start session" })).toBeDisabled();
    expect(start).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("button", { name: "Start session" })).toBeEnabled();
  });

  it("starts with the selected new preset and preserves the explicit spoiler scope", async () => {
    const user = userEvent.setup();
    const start = vi.fn(async () => {});
    view(
      <CompanionSetup
        gameId="game-one"
        replacing={false}
        pending={false}
        onStart={start}
      />,
    );
    const preset = companionProfiles().presets[7]!;
    await screen.findByRole("option", { name: "Mystery partner" });
    await user.selectOptions(screen.getByLabelText("Personality"), `${preset.id}:1`);
    await user.click(screen.getByRole("button", { name: "Start session" }));
    expect(start).toHaveBeenCalledWith(
      expect.objectContaining({
        profile: preset,
        knowledge: "observed",
        source: "manual",
      }),
    );
    expect(mocks.applyProfile).not.toHaveBeenCalled();
  });

  it("saves custom edits without applying them until the reader chooses Apply", async () => {
    const user = userEvent.setup();
    const current = companionSnapshot();
    current.busy = true;
    acceptSnapshot(current);
    const custom = {
      ...companionProfiles().presets[0]!,
      id: "custom-profile",
      settings: {
        ...companionProfiles().presets[0]!.settings,
        name: "Aki",
        style: "Gentle and brief",
      },
    };
    mocks.saveProfile.mockResolvedValue(custom);
    mocks.applyProfile.mockResolvedValue({
      ...current,
      sequence: 2,
      busy: false,
      session: {
        ...current.session!,
        options: { ...current.session!.options, profile: custom },
      },
    });
    view(<CompanionScreen />);
    await user.click(screen.getByRole("button", { name: "Session options" }));
    await user.click(await screen.findByRole("button", { name: "Customize" }));
    expect(
      document.querySelector(".companion__profile-editor .companion__fields"),
    ).toBeInTheDocument();
    await user.clear(screen.getByLabelText("Profile name"));
    await user.type(screen.getByLabelText("Profile name"), "Aki");
    await user.type(screen.getByLabelText("Speaking style"), "Gentle and brief");
    await user.click(screen.getByText("Sample reply"));
    expect(screen.getByText(/Authored example/)).toBeVisible();
    expect(mocks.send).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Save profile" }));
    expect(mocks.saveProfile).toHaveBeenCalledWith(null, null, custom.settings);
    expect(mocks.applyProfile).not.toHaveBeenCalled();
    expect(
      useCompanionStore.getState().snapshot?.session?.options.profile,
    ).toBeUndefined();
    await user.click(await screen.findByRole("button", { name: "Apply personality" }));
    expect(mocks.applyProfile).toHaveBeenCalledWith("session-one", custom);
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "Apply personality" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("keeps a saved session version selected when a profile is updated or deleted", async () => {
    const user = userEvent.setup();
    const first = {
      ...companionProfiles().presets[0]!,
      id: "custom-profile",
      settings: { ...companionProfiles().presets[0]!.settings, name: "My friend" },
    };
    const second = {
      ...first,
      version: 2,
      settings: { ...first.settings, name: "Updated friend" },
    };
    mocks.profiles.mockResolvedValue({ ...companionProfiles(), profiles: [second] });
    const current = companionSnapshot();
    current.session!.options.profile = first;
    acceptSnapshot(current);
    mocks.deleteProfile.mockResolvedValue(undefined);
    view(<CompanionScreen />);
    await user.click(screen.getByRole("button", { name: "Session options" }));
    await screen.findByRole("option", { name: "Updated friend" });
    expect(screen.getByLabelText("Personality")).toHaveValue("custom-profile:1");
    await user.selectOptions(screen.getByLabelText("Personality"), "custom-profile:2");
    await user.click(screen.getByRole("button", { name: "Customize" }));
    await user.click(screen.getByRole("button", { name: "Delete profile" }));
    expect(mocks.deleteProfile).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Confirm deletion" }));
    expect(mocks.deleteProfile).toHaveBeenCalledWith("custom-profile", 2);
    expect(useCompanionStore.getState().snapshot?.session?.options.profile).toEqual(
      first,
    );
    expect(mocks.applyProfile).not.toHaveBeenCalled();
  });
});

describe("companion overlay", () => {
  it("opens only when native capabilities support it", async () => {
    const user = userEvent.setup();
    const initial = companionSnapshot();
    initial.capabilities.overlay = "available";
    acceptSnapshot(initial);
    view(<CompanionScreen />);
    await user.click(await screen.findByRole("button", { name: "Open overlay" }));
    expect(mocks.open).toHaveBeenCalledOnce();
    act(() =>
      acceptSnapshot({
        ...initial,
        sequence: 2,
        capabilities: { ...initial.capabilities, overlay: "unsupported" },
      }),
    );
    expect(
      screen.queryByRole("button", { name: "Open overlay" }),
    ).not.toBeInTheDocument();
  });

  it("hides and reconnects to the current reply without cancelling or resending", async () => {
    const user = userEvent.setup();
    const current = { ...companionSnapshot(), busy: true, live: companionMessage() };
    acceptSnapshot(current);
    mocks.snapshot.mockResolvedValue(current);
    mocks.messages.mockResolvedValue([companionMessage()]);
    const first = view(<CompanionOverlay />, "/companion-overlay");
    expect(await screen.findAllByText("A quiet scene.")).toHaveLength(1);
    expect(
      first.container.querySelector(".companion-overlay .companion--overlay"),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Choose a session")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Hide" }));
    expect(mocks.hide).toHaveBeenCalledOnce();
    expect(mocks.control).not.toHaveBeenCalled();
    first.unmount();
    view(<CompanionOverlay />, "/companion-overlay");
    expect(await screen.findAllByText("A quiet scene.")).toHaveLength(1);
    expect(mocks.send).not.toHaveBeenCalled();
    expect(mocks.control).not.toHaveBeenCalled();
    mocks.control.mockResolvedValue({ ...current, sequence: 2, busy: false });
    await user.click(screen.getByRole("button", { name: "Stop reply" }));
    expect(mocks.control).toHaveBeenCalledWith("session-one", "cancel_turn");
  });

  it("opens setup in the main window when there is no active session", async () => {
    const user = userEvent.setup();
    acceptSnapshot({ ...companionSnapshot(), session: null, lifecycle: "off" });
    view(<CompanionScreen overlay />, "/companion-overlay");
    await user.click((await screen.findAllByRole("button", { name: "Open in app" }))[0]!);
    expect(mocks.showMain).toHaveBeenCalledWith();
    expect(mocks.list).not.toHaveBeenCalled();
    expect(
      screen.queryByRole("button", { name: "Start session" }),
    ).not.toBeInTheDocument();
  });
});

describe("companion setup and controls", () => {
  it("requires a game-specific Luna confirmation and retains the form's layout containers", async () => {
    const user = userEvent.setup();
    const start = vi.fn(async () => {});
    const result = view(
      <CompanionSetup
        gameId="game-one"
        replacing={false}
        pending={false}
        onStart={start}
      />,
    );
    expect(result.container.querySelector(".companion__fields")).toBeInTheDocument();
    expect(result.container.querySelector(".companion__actions")).toBeInTheDocument();
    expect(screen.getByLabelText("Session name")).not.toBeVisible();
    expect(screen.getByText("Spoiler scope")).toBeVisible();
    await user.click(screen.getByText("Reading context"));
    await user.clear(screen.getByLabelText("Session name"));
    await user.type(screen.getByLabelText("Session name"), "Route B");
    await user.type(
      screen.getByLabelText("Reading boundary (optional)"),
      "Only this scene",
    );
    await user.click(screen.getByText("Reading context"));
    await user.selectOptions(screen.getByLabelText("Dialogue source"), "luna");
    expect(screen.getByRole("button", { name: "Start session" })).toBeDisabled();
    await user.click(
      await screen.findByRole("checkbox", { name: /Luna is showing this game/ }),
    );
    await user.click(screen.getByRole("button", { name: "Start session" }));
    expect(start).toHaveBeenCalledWith(
      expect.objectContaining({
        gameId: "game-one",
        source: "luna",
        knowledge: "observed",
        playthroughLabel: "Route B",
        boundary: "Only this scene",
      }),
    );
  });

  it("renders an empty setup without a history-null crash", async () => {
    acceptSnapshot({ ...companionSnapshot(), session: null, lifecycle: "off" });
    mocks.list.mockResolvedValue([]);
    view(<CompanionScreen />);
    expect(await screen.findByLabelText("Game")).toBeVisible();
    expect(screen.getByLabelText("Personality")).toBeVisible();
    expect(screen.getByLabelText("Spoiler scope")).toBeVisible();
    expect(screen.getByLabelText("Dialogue source")).toBeVisible();
    expect(screen.getByRole("button", { name: "Start session" })).toBeDisabled();
    expect(mocks.messages).not.toHaveBeenCalled();
  });

  it("keeps a removed game's archive readable and removable after restart", async () => {
    const user = userEvent.setup();
    acceptSnapshot({ ...companionSnapshot(), session: null, lifecycle: "off" });
    mocks.list.mockResolvedValue([
      { ...companionSession(), gameTitle: null, state: "ended" },
    ]);
    mocks.messages.mockResolvedValue([{ ...companionMessage(), status: "complete" }]);
    view(<CompanionScreen />);
    await user.click(await screen.findByRole("button", { name: "Session options" }));
    await user.selectOptions(
      await screen.findByLabelText("Choose a session"),
      "session-one",
    );
    expect(await screen.findByRole("heading", { name: "Removed game" })).toBeVisible();
    expect(await screen.findByText("A quiet scene.")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Send" })).not.toBeInTheDocument();
    expect(mocks.messages).toHaveBeenCalledWith("session-one");
    await user.click(screen.getByRole("button", { name: "Session options" }));
    await user.click(screen.getByRole("button", { name: "Delete session history" }));
    expect(mocks.delete).not.toHaveBeenCalled();
    expect(
      screen.getByText(
        /Delete this session, its conversation and memories derived from it/,
      ),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(mocks.delete).not.toHaveBeenCalled();
  });

  it("pauses and resumes from snapshots without retaining old dialogue previews", async () => {
    const user = userEvent.setup();
    const initial = companionSnapshot();
    initial.session!.options.source = "luna";
    initial.source = {
      state: "receiving",
      lastLineAt: 1_700_000_000_000,
      lineCount: 1,
      preview: ["An observed line"],
    };
    acceptSnapshot(initial);
    const paused: CompanionSnapshot = {
      ...initial,
      sequence: 2,
      lifecycle: "paused",
      session: { ...initial.session!, state: "paused", reason: "user" },
      source: { state: "paused", lastLineAt: null, lineCount: 0, preview: [] },
    };
    mocks.control.mockResolvedValueOnce(paused).mockResolvedValueOnce({
      ...initial,
      sequence: 3,
      source: { state: "waiting", lastLineAt: null, lineCount: 0, preview: [] },
    });
    view(<CompanionScreen />);
    await screen.findByText("Luna: receiving text");
    await user.click(screen.getByRole("button", { name: "Session options" }));
    expect(screen.getByText("An observed line")).toBeVisible();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "Pause session" }));
    expect(await screen.findByText("Paused")).toBeVisible();
    expect(screen.queryByText("An observed line")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Resume with fresh dialogue" }));
    expect(await screen.findByText("Luna: waiting for text")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Session options" }));
    expect(screen.queryByText("An observed line")).not.toBeInTheDocument();
    expect(mocks.control.mock.calls.map((call) => call[1])).toEqual(["pause", "resume"]);
  });

  it("reconnects to a partial reply without duplicating it or cancelling on page loss", async () => {
    const message = companionMessage();
    acceptSnapshot({ ...companionSnapshot(), busy: true, live: message });
    mocks.messages.mockResolvedValue([message]);
    const first = view(<CompanionScreen />);
    expect(await screen.findAllByText("A quiet scene.")).toHaveLength(1);
    first.unmount();
    expect(mocks.control).not.toHaveBeenCalled();
    const next = view(<CompanionScreen />);
    expect(await screen.findAllByText("A quiet scene.")).toHaveLength(1);
    expect(next.container.querySelector(".companion__conversation")).toBeInTheDocument();
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("stops a current session before starting an earlier-save context", async () => {
    const user = userEvent.setup();
    acceptSnapshot(companionSnapshot());
    const ended = { ...companionSnapshot(2), lifecycle: "off" as const };
    mocks.control.mockResolvedValue(ended);
    const fresh = companionSnapshot(3);
    fresh.session = {
      ...companionSession(),
      id: "fresh-session",
      threadId: "fresh-thread",
      playthroughId: "fresh-reading",
    };
    mocks.start.mockResolvedValue(fresh);
    view(<CompanionScreen />, "/companion?game=game-one");
    await user.click(await screen.findByRole("button", { name: "Session options" }));
    await user.click(await screen.findByRole("button", { name: "New session" }));
    await user.click(screen.getByRole("button", { name: "End current and start fresh" }));
    await waitFor(() => expect(mocks.start).toHaveBeenCalledOnce());
    expect(mocks.control).toHaveBeenCalledWith("session-one", "end");
    expect(mocks.control.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.start.mock.invocationCallOrder[0]!,
    );
    await waitFor(() => expect(screen.queryByLabelText("Game")).not.toBeInTheDocument());
  });

  it("keeps session details out of the reading view and preserves focus while replies update", async () => {
    const user = userEvent.setup();
    const initial = companionSnapshot();
    acceptSnapshot(initial);
    useCompanionStore.setState({ draft: "Keep my question" });
    const result = view(<CompanionScreen />);
    await waitFor(() => expect(mocks.provider).toHaveBeenCalled());
    expect(screen.queryByRole("link", { name: "Open Settings" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Choose a session")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "End session" })).not.toBeInTheDocument();
    expect(
      result.container.querySelector(".companion__composer > .companion__send"),
    ).toBeInTheDocument();
    const trigger = screen.getByRole("button", { name: "Session options" });
    await user.click(trigger);
    const panel = screen.getByRole("dialog", { name: "Session options" });
    const picker = within(panel).getByLabelText("Choose a session");
    picker.focus();
    act(() =>
      acceptSnapshot({ ...initial, sequence: 2, busy: true, live: companionMessage() }),
    );
    expect(picker).toHaveFocus();
    expect(within(panel).getByRole("button", { name: "End session" })).toBeEnabled();
    expect(
      screen.queryByRole("button", { name: "Delete session history" }),
    ).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
    expect(screen.getByLabelText("Message")).toHaveValue("Keep my question");
    expect(screen.getByRole("button", { name: "Stop reply" })).toBeEnabled();
  });

  it("ends the shared session through its options panel", async () => {
    const user = userEvent.setup();
    acceptSnapshot(companionSnapshot());
    mocks.control.mockResolvedValue({
      ...companionSnapshot(2),
      lifecycle: "off",
      session: { ...companionSession(), state: "ended" },
    });
    view(<CompanionScreen />);
    await user.click(screen.getByRole("button", { name: "Session options" }));
    await user.click(screen.getByRole("button", { name: "End session" }));
    expect(mocks.control).toHaveBeenCalledWith("session-one", "end");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(await screen.findByText("Session ended")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Send" })).not.toBeInTheDocument();
  });

  it("keeps finished tool details with sources while leaving the answer visible", async () => {
    const user = userEvent.setup();
    const message = {
      ...companionMessage(),
      status: "complete" as const,
      tools: [
        {
          id: "call-one",
          name: "recent_game_text",
          arguments: "{}",
          status: "complete",
          result: null,
        },
      ],
      sources: [{ id: "S1", title: "Recent dialogue", gameId: null, url: null }],
    };
    acceptSnapshot(companionSnapshot());
    mocks.messages.mockResolvedValue([message]);
    view(<CompanionScreen />);
    expect(await screen.findByText("A quiet scene.")).toBeVisible();
    const tools = screen.getByLabelText("Tool activity");
    expect(tools).not.toBeVisible();
    await user.click(screen.getByText("Sources (1)"));
    expect(tools).toBeVisible();
    expect(tools.closest(".vnchat__sources")).toBeInTheDocument();
  });

  it("keeps the session bar available on another page and only removes its listener on unmount", async () => {
    acceptSnapshot(companionSnapshot());
    const unlisten = vi.fn();
    mocks.listen.mockResolvedValue(unlisten);
    const result = view(<CompanionBar />, "/");
    expect(screen.getByRole("link", { name: /Synthetic VN/ })).toHaveAttribute(
      "href",
      "/companion",
    );
    expect(result.container.querySelector(".companion-bar")).toBeInTheDocument();
    await act(async () => {
      await Promise.resolve();
    });
    result.unmount();
    expect(unlisten).toHaveBeenCalledOnce();
    expect(mocks.control).not.toHaveBeenCalled();
  });
});
