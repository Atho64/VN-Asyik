import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import type { PlaySession } from "@/lib/ipc/types";
import { SessionHistory } from "./SessionHistory";

const mocks = vi.hoisted(() => ({ listGameSessions: vi.fn() }));
vi.mock("@/lib/ipc/client", () => ({ api: mocks }));

const first: PlaySession[] = Array.from({ length: 50 }, (_, i) => ({
  meta: { id: `session-${i}`, updated_at: 1, device_id_device: "test", deleted: false },
  game_id: "game",
  chapter_id: null,
  source: "manual",
  started_at: 1_700_000_000_000 - i * 3_600_000,
  ended_at: 1_700_000_060_000 - i * 3_600_000,
  counted_ms: 60000,
  afk_ms: 0,
}));

function mount() {
  const select = vi.fn();
  render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <SessionHistory gameId="game" sessions={first} total={51} onSelect={select} />
    </AppContext.Provider>,
  );
  return select;
}

beforeEach(() => vi.resetAllMocks());

describe("session history", () => {
  it("opens a selected session by keyboard and bounds each page while reaching older sessions", async () => {
    const user = userEvent.setup();
    const select = mount();
    const rows = screen.getAllByRole("button", { name: /^View reading session:/ });
    expect(rows).toHaveLength(50);
    expect(rows[0]).toHaveClass("session-history__session");
    rows[0]!.focus();
    await user.keyboard("{Enter}");
    expect(select).toHaveBeenCalledWith("session-0");
    const older = { ...first[49]!, meta: { ...first[49]!.meta, id: "oldest" } };
    mocks.listGameSessions.mockResolvedValueOnce({
      sessions: [older],
      totalSessions: 51,
      notePreviews: { oldest: "An earlier choice" },
      countedMs: 3060000,
      afkMs: 0,
    });
    await user.click(screen.getByRole("button", { name: "Older sessions" }));
    expect(await screen.findByText(/Showing 1 of 51 sessions/)).toBeInTheDocument();
    expect(screen.getByText("An earlier choice")).toHaveClass("session-history__note");
    expect(
      screen.getByRole("button", { name: /^View reading session:/ }),
    ).toHaveAccessibleDescription("An earlier choice");
    expect(mocks.listGameSessions).toHaveBeenCalledWith("game", {
      startedAt: first[49]!.started_at,
      id: "session-49",
    });
    expect(
      screen.getAllByRole("button", { name: /^View reading session:/ }),
    ).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: /^View reading session:/ }));
    expect(select).toHaveBeenLastCalledWith("oldest");
    mocks.listGameSessions.mockResolvedValueOnce({
      sessions: first,
      totalSessions: 51,
      notePreviews: {},
      countedMs: 3060000,
      afkMs: 0,
    });
    await user.click(screen.getByRole("button", { name: "Newer sessions" }));
    expect(await screen.findByText(/Showing 50 of 51 sessions/)).toBeInTheDocument();
    expect(mocks.listGameSessions).toHaveBeenLastCalledWith("game", undefined);
  });

  it("preserves the current page on failure and permits a retry", async () => {
    const user = userEvent.setup();
    mount();
    mocks.listGameSessions.mockRejectedValueOnce(new Error("offline"));
    await user.click(screen.getByRole("button", { name: "Older sessions" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not load that page",
    );
    expect(
      screen.getAllByRole("button", { name: /^View reading session:/ }),
    ).toHaveLength(50);
    mocks.listGameSessions.mockResolvedValueOnce({
      sessions: [],
      totalSessions: 50,
      notePreviews: {},
      countedMs: 3000000,
      afkMs: 0,
    });
    await user.click(screen.getByRole("button", { name: "Older sessions" }));
    expect(await screen.findByText(/Showing 0 of 50 sessions/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Newer sessions" })).toBeEnabled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("refreshes edited and deleted previews on the current older page and ignores late replies", async () => {
    const user = userEvent.setup();
    const older = { ...first[49]!, meta: { ...first[49]!.meta, id: "oldest" } };
    const response = {
      sessions: [older],
      totalSessions: 51,
      countedMs: 3060000,
      afkMs: 0,
      notePreviews: { oldest: "Original note" },
    };
    const view = (noteVersion: number) => (
      <AppContext.Provider
        value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
      >
        <SessionHistory
          gameId="game"
          sessions={first}
          total={51}
          notePreviews={{ "session-0": "Latest session note" }}
          noteVersion={noteVersion}
          onSelect={vi.fn()}
        />
      </AppContext.Provider>
    );
    const mounted = render(view(0));
    expect(screen.getByText("Latest session note")).toHaveClass("session-history__note");
    expect(mocks.listGameSessions).not.toHaveBeenCalled();
    mocks.listGameSessions.mockResolvedValueOnce(response);
    await user.click(screen.getByRole("button", { name: "Older sessions" }));
    await screen.findByText("Original note");
    let finishOld: (value: unknown) => void = () => undefined;
    mocks.listGameSessions.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishOld = resolve;
        }),
    );
    mounted.rerender(view(1));
    expect(screen.queryByText("Original note")).not.toBeInTheDocument();
    mocks.listGameSessions.mockResolvedValueOnce({
      ...response,
      notePreviews: { oldest: "Edited note" },
    });
    mounted.rerender(view(2));
    await screen.findByText("Edited note");
    await act(async () => finishOld(response));
    expect(screen.queryByText("Original note")).not.toBeInTheDocument();
    expect(mocks.listGameSessions).toHaveBeenLastCalledWith("game", {
      startedAt: first[49]!.started_at,
      id: "session-49",
    });
    mocks.listGameSessions.mockRejectedValueOnce(new Error("offline"));
    mounted.rerender(view(3));
    await screen.findByRole("alert");
    expect(screen.queryByText("Edited note")).not.toBeInTheDocument();
    mocks.listGameSessions.mockResolvedValueOnce({ ...response, notePreviews: {} });
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
    expect(screen.getByText(/Showing 1 of 51 sessions/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /^View reading session:/ }),
    ).not.toHaveAttribute("aria-describedby");
  });
});
