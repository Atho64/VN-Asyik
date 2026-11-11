import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import { IpcError } from "@/lib/ipc/client";
import type * as ClientModule from "@/lib/ipc/client";
import type { ReadingContext, ReadingEntry } from "@/lib/ipc/reading";
import { ReadingCard } from "./ReadingCard";

const mocks = vi.hoisted(() => ({
  context: vi.fn(),
  session: vi.fn(),
  save: vi.fn(),
  remove: vi.fn(),
  journalExport: vi.fn(),
  screenshotTile: vi.fn(),
  screenshots: vi.fn(),
}));
vi.mock("@/lib/ipc/reading", () => ({ readingApi: mocks }));
// The export action and the pinned-photo tile go through the shared client, not
// through `readingApi`. Keep the real module (the component imports `IpcError`
// from it) and replace only the calls this screen makes.
vi.mock("@/lib/ipc/client", async (importOriginal) => ({
  ...(await importOriginal<typeof ClientModule>()),
  api: {
    journalExport: mocks.journalExport,
    vndbScreenshotTile: mocks.screenshotTile,
    vndbScreenshots: mocks.screenshots,
  },
}));

const entry: ReadingEntry = {
  meta: { id: "note-1", updated_at: 100, device_id_device: "test", deleted: false },
  game_id: "game-1",
  session_id: "session-1",
  created_at: 100,
  played_at: 90,
  note: "Before the choice",
  save_slot: "12",
  route_name: "Shizuku",
  chapter_name: "August 7",
  photo_image_id: "",
};
const initial: ReadingContext = {
  latestSession: {
    id: "session-1",
    startedAt: 1000,
    endedAt: 61000,
    countedMs: 60000,
    afkMs: 0,
    routeName: "Shizuku",
    chapterName: "August 7",
  },
  entries: [entry],
  hasMore: false,
};

function mount(sessionId: string | null = null, onEntriesChange = vi.fn()) {
  const guide = vi.fn();
  render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <ReadingCard
        gameId="game-1"
        vnId="v17"
        title="Symphonic Rain"
        onWalkthrough={guide}
        selectedSessionId={sessionId}
        onEntriesChange={onEntriesChange}
      />
    </AppContext.Provider>,
  );
  return guide;
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.context.mockResolvedValue(initial);
  mocks.save.mockResolvedValue(entry);
  mocks.remove.mockResolvedValue(null);
});

describe("reading checkpoints", () => {
  it("ignores a late response after switching the selected session", async () => {
    let finishOld: (value: unknown) => void = () => undefined;
    mocks.session.mockImplementation((_game: string, sessionId: string) =>
      sessionId === "slow-session"
        ? new Promise((resolve) => {
            finishOld = resolve;
          })
        : Promise.resolve({
            session: {
              ...initial.latestSession,
              id: "new-session",
              routeName: "New route",
            },
            entries: [],
            hasMore: false,
          }),
    );
    const view = (sessionId: string) => (
      <AppContext.Provider
        value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
      >
        <ReadingCard
          gameId="game-1"
          vnId="v17"
          title="Symphonic Rain"
          onWalkthrough={() => undefined}
          selectedSessionId={sessionId}
        />
      </AppContext.Provider>
    );
    const mounted = render(view("slow-session"));
    mounted.rerender(view("new-session"));
    const details = screen.getByRole("dialog", { name: "Reading session" });
    expect(await within(details).findByText("New route")).toBeInTheDocument();
    await act(async () =>
      finishOld({
        session: {
          ...initial.latestSession,
          id: "slow-session",
          routeName: "Stale route",
        },
        entries: [],
        hasMore: false,
      }),
    );
    expect(within(details).getByText("New route")).toBeInTheDocument();
    expect(within(details).queryByText("Stale route")).not.toBeInTheDocument();
  });

  it("adds to the selected older session and keeps that association across refreshes", async () => {
    const older = {
      ...initial.latestSession!,
      id: "old-session",
      routeName: "Old route",
      chapterName: "Old chapter",
      afkMs: 120000,
    };
    mocks.session.mockResolvedValue({ session: older, entries: [], hasMore: false });
    const user = userEvent.setup();
    const changed = vi.fn();
    mount("old-session", changed);
    const details = await screen.findByRole("dialog", { name: "Reading session" });
    expect(await within(details).findByText("Old route")).toBeInTheDocument();
    expect(within(details).getByText("2 min")).toBeInTheDocument();
    expect(within(details).queryByText("Before the choice")).not.toBeInTheDocument();
    expect(details.querySelector(".reading__session-facts")).toBeInTheDocument();
    await user.click(within(details).getByRole("button", { name: "Add checkpoint" }));
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(screen.getByLabelText("Route")).toHaveValue("Old route");
    await user.type(screen.getByLabelText("Session note"), "Earlier choice");
    mocks.context.mockResolvedValue({
      ...initial,
      latestSession: { ...initial.latestSession, id: "newest-session" },
    });
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(mocks.context).toHaveBeenCalledTimes(2));
    mocks.save.mockRejectedValueOnce(new Error("disk full"));
    await user.click(screen.getByRole("button", { name: "Save checkpoint" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Your text is still here");
    expect(changed).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Session note")).toHaveValue("Earlier choice");
    const saved = {
      ...entry,
      session_id: "old-session",
      note: "Earlier choice",
      route_name: "Old route",
    };
    mocks.session.mockResolvedValue({ session: older, entries: [saved], hasMore: false });
    await user.click(screen.getByRole("button", { name: "Save checkpoint" }));
    expect(mocks.save).toHaveBeenLastCalledWith(
      expect.objectContaining({
        session_id: "old-session",
        route_name: "Old route",
        chapter_name: "Old chapter",
        note: "Earlier choice",
      }),
    );
    const reopened = await screen.findByRole("dialog", { name: "Reading session" });
    expect(await within(reopened).findByText("Earlier choice")).toBeInTheDocument();
    expect(changed).toHaveBeenCalledOnce();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
  });

  it("pages only the selected session's notes, then edits and deletes with revision checks", async () => {
    const older = { ...initial.latestSession!, id: "old-session" };
    const notes = Array.from({ length: 10 }, (_, i) => ({
      ...entry,
      meta: { ...entry.meta, id: `old-${i}` },
      session_id: "old-session",
      note: `Old checkpoint ${i}`,
    }));
    mocks.session.mockResolvedValue({ session: older, entries: notes, hasMore: true });
    const user = userEvent.setup();
    const changed = vi.fn();
    mount("old-session", changed);
    const details = await screen.findByRole("dialog", { name: "Reading session" });
    await within(details).findByText("Old checkpoint 0");
    mocks.session.mockResolvedValueOnce({
      session: older,
      entries: [
        { ...notes[0], meta: { ...entry.meta, id: "old-10" }, note: "Old checkpoint 10" },
      ],
      hasMore: false,
    });
    await user.click(within(details).getByRole("button", { name: "Show older entries" }));
    expect(await within(details).findByText("Old checkpoint 10")).toBeInTheDocument();
    expect(mocks.session).toHaveBeenLastCalledWith("game-1", "old-session", 10);
    await user.click(
      within(details).getAllByRole("button", { name: "Edit checkpoint" })[0]!,
    );
    await user.clear(screen.getByLabelText("Save slot"));
    await user.type(screen.getByLabelText("Save slot"), "21");
    await user.click(screen.getByRole("button", { name: "Save checkpoint" }));
    await screen.findByRole("dialog", { name: "Reading session" });
    expect(mocks.save).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "old-0",
        expected_updated_at: 100,
        session_id: "old-session",
        save_slot: "21",
      }),
    );
    const updatedDetails = screen.getByRole("dialog", { name: "Reading session" });
    await within(updatedDetails).findByText("Old checkpoint 0");
    await user.click(
      within(updatedDetails).getAllByRole("button", { name: "Delete checkpoint" })[0]!,
    );
    expect(mocks.remove).not.toHaveBeenCalled();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    await user.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Delete checkpoint",
      }),
    );
    await screen.findByRole("dialog", { name: "Reading session" });
    expect(mocks.remove).toHaveBeenCalledWith("game-1", notes[0]);
    expect(changed).toHaveBeenCalledTimes(2);
  });

  it("shows a recoverable session error without offering an unlinked checkpoint", async () => {
    mocks.session.mockRejectedValueOnce(new IpcError("not_found", "gone"));
    const user = userEvent.setup();
    mount("missing-session");
    const details = await screen.findByRole("dialog", { name: "Reading session" });
    expect(await within(details).findByRole("alert")).toHaveTextContent(
      "Could not refresh this session",
    );
    expect(
      within(details).queryByRole("button", { name: "Add checkpoint" }),
    ).not.toBeInTheDocument();
    mocks.session.mockResolvedValue({
      session: { ...initial.latestSession, id: "missing-session" },
      entries: [],
      hasMore: false,
    });
    await user.click(within(details).getByRole("button", { name: "Reload journal" }));
    expect(
      await within(details).findByRole("button", { name: "Add checkpoint" }),
    ).toBeEnabled();
  });

  it("offers a manual checkpoint without inventing a session or chapter", async () => {
    mocks.context.mockResolvedValue({ latestSession: null, entries: [], hasMore: false });
    const user = userEvent.setup();
    const guide = mount();
    expect(await screen.findByText(/No reading session recorded/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open walkthrough" }));
    expect(guide).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: "Add checkpoint" }));
    expect(screen.getByRole("button", { name: "Save checkpoint" })).toBeDisabled();
    await user.type(screen.getByLabelText("Save slot"), "12");
    await user.click(screen.getByRole("button", { name: "Save checkpoint" }));
    expect(mocks.save).toHaveBeenCalledWith(
      expect.objectContaining({
        game_id: "game-1",
        session_id: null,
        save_slot: "12",
        note: "",
        route_name: "",
        chapter_name: "",
      }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("keeps a historical note distinct from an unattributed latest session", async () => {
    mocks.context.mockResolvedValue({
      ...initial,
      latestSession: { ...initial.latestSession, routeName: null, chapterName: null },
    });
    mount();
    expect(await screen.findByText("Before the choice")).toBeInTheDocument();
    const session = screen
      .getByText("Last recorded reading session")
      .closest(".reading__session") as HTMLElement;
    expect(
      within(session).getByText("No route or chapter recorded for this session."),
    ).toBeInTheDocument();
    expect(within(session).queryByText("Shizuku")).not.toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Where did I leave off?" }).closest("section"),
    ).toHaveClass("reading-card");
    expect(
      screen.getByText("Last checkpoint").closest(".reading__overview"),
    ).toBeInTheDocument();
  });

  it("captures the form's session and preserves text when a save fails", async () => {
    const user = userEvent.setup();
    mocks.save.mockRejectedValueOnce(new Error("disk full"));
    mount();
    await screen.findByText("Before the choice");
    await user.click(screen.getByRole("button", { name: "Add checkpoint" }));
    await user.type(screen.getByLabelText("Session note"), "Continue tomorrow");
    expect(screen.getByLabelText("Route")).toHaveValue("Shizuku");
    mocks.context.mockResolvedValue({
      ...initial,
      latestSession: {
        ...initial.latestSession,
        id: "new-session",
        routeName: "New route",
      },
    });
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(mocks.context).toHaveBeenCalledTimes(2));
    await user.click(screen.getByRole("button", { name: "Save checkpoint" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Your text is still here");
    expect(screen.getByLabelText("Session note")).toHaveValue("Continue tomorrow");
    expect(mocks.save).toHaveBeenCalledWith(
      expect.objectContaining({ session_id: "session-1", route_name: "Shizuku" }),
    );
  });

  it("submits once while pending and does not dismiss the form before saving", async () => {
    let finish: (value: ReadingEntry) => void = () => undefined;
    mocks.save.mockImplementation(
      () =>
        new Promise<ReadingEntry>((resolve) => {
          finish = resolve;
        }),
    );
    const user = userEvent.setup();
    mount();
    await screen.findByText("Before the choice");
    await user.click(screen.getByRole("button", { name: "Edit checkpoint" }));
    await user.dblClick(screen.getByRole("button", { name: "Save checkpoint" }));
    expect(mocks.save).toHaveBeenCalledOnce();
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await act(async () => finish(entry));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("uses revision checks for editing and requires confirmation for deletion", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("Before the choice");
    await user.click(screen.getByRole("button", { name: "Edit checkpoint" }));
    await user.clear(screen.getByLabelText("Save slot"));
    await user.type(screen.getByLabelText("Save slot"), "13");
    mocks.save.mockRejectedValueOnce(new IpcError("reading_conflict", "changed"));
    await user.click(screen.getByRole("button", { name: "Save checkpoint" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("changed or was deleted");
    expect(mocks.save).toHaveBeenCalledWith(
      expect.objectContaining({
        id: entry.meta.id,
        expected_updated_at: 100,
        save_slot: "13",
      }),
    );
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(screen.getByRole("button", { name: "Delete checkpoint" }));
    expect(mocks.remove).not.toHaveBeenCalled();
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete checkpoint" }));
    expect(mocks.remove).toHaveBeenCalledWith("game-1", entry);
  });

  it("loads older notes on demand and can recover from an initial read error", async () => {
    mocks.context.mockRejectedValueOnce(new Error("offline"));
    const user = userEvent.setup();
    mount();
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not refresh");
    const entries = Array.from({ length: 10 }, (_, i) => ({
      ...entry,
      meta: { ...entry.meta, id: `note-${i}` },
      note: `Checkpoint ${i}`,
    }));
    mocks.context.mockResolvedValueOnce({ ...initial, entries, hasMore: true });
    await user.click(screen.getByRole("button", { name: "Reload journal" }));
    await screen.findByText("Checkpoint 0");
    expect(screen.queryByText("Checkpoint 1")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reading journal" }));
    expect(screen.getByText("Checkpoint 1")).toBeInTheDocument();
    mocks.context.mockResolvedValueOnce({
      ...initial,
      entries: [{ ...entry, meta: { ...entry.meta, id: "older" }, note: "Older note" }],
      hasMore: false,
    });
    await user.click(screen.getByRole("button", { name: "Show older entries" }));
    expect(await screen.findByText("Older note")).toBeInTheDocument();
    expect(mocks.context).toHaveBeenLastCalledWith("game-1", 10);
    expect(
      screen.queryByRole("button", { name: "Show older entries" }),
    ).not.toBeInTheDocument();
  });
});

const shot = {
  image_id: "sf83040",
  vndb_url: "https://vndb.org/sf83040",
  src: "data:image/jpeg;base64,AAAA",
  blurred_src: "data:image/jpeg;base64,BBBB",
  sharp: true,
  explicit: false,
  width: 640,
  height: 480,
};

describe("journal photos and export", () => {
  beforeEach(() => {
    mocks.screenshots.mockResolvedValue({
      shots: [shot],
      reported: 1,
      complete: true,
      pending: 0,
    });
    mocks.screenshotTile.mockResolvedValue(shot);
    mocks.journalExport.mockResolvedValue({
      saved: true,
      filename: "Symphonic-Rain-journal.md",
      entries: 1,
      photos: 1,
      concealed: 0,
    });
  });

  it("pins a screenshot to a checkpoint and saves its id", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(await screen.findByRole("button", { name: "Add checkpoint" }));
    await user.click(screen.getByRole("button", { name: "Pin a screenshot" }));

    const picker = await screen.findByRole("dialog", { name: "Pin a screenshot" });
    await user.click(
      await within(picker).findByRole("button", {
        name: "Pin this screenshot to Symphonic Rain",
      }),
    );

    // The chosen photo is painted in the form, and it alone is enough to save:
    // a photo diary entry does not need a caption.
    const form = screen.getByRole("dialog", { name: "Add checkpoint" });
    expect(await within(form).findByAltText(/Screenshot pinned to a journal/)).toBeInTheDocument();
    const save = within(form).getByRole("button", { name: "Save checkpoint" });
    expect(save).toBeEnabled();
    await user.click(save);
    expect(mocks.save).toHaveBeenCalledWith(
      expect.objectContaining({ photo_image_id: "sf83040" }),
    );
  });

  it("renders a pinned photo on an existing checkpoint, fetching each image once", async () => {
    // A distinct id: the tile cache is module-level and survives between tests,
    // which is the behaviour being pinned here — one fetch per image, not one per
    // render.
    const pinned = { ...shot, image_id: "sf99999" };
    mocks.screenshotTile.mockResolvedValue(pinned);
    mocks.context.mockResolvedValue({
      ...initial,
      entries: [{ ...entry, photo_image_id: "sf99999" }],
    });
    mount();
    expect(
      await screen.findByAltText(/Screenshot pinned to a journal/),
    ).toBeInTheDocument();
    expect(mocks.screenshotTile).toHaveBeenCalledWith("v17", "sf99999");
    // The same image on a second checkpoint must not be fetched again.
    mocks.screenshotTile.mockClear();
    mocks.context.mockResolvedValue({
      ...initial,
      entries: [
        { ...entry, photo_image_id: "sf99999" },
        { ...entry, meta: { ...entry.meta, id: "note-2" }, photo_image_id: "sf99999" },
      ],
    });
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(mocks.context).toHaveBeenCalledTimes(2));
    expect(mocks.screenshotTile).not.toHaveBeenCalled();
  });

  it("exports the journal in the format the reader picks", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(await screen.findByRole("button", { name: "Export journal" }));
    const sheet = await screen.findByRole("dialog", { name: "Export journal" });
    await user.click(within(sheet).getByRole("button", { name: "Markdown (.md)" }));
    await waitFor(() =>
      expect(mocks.journalExport).toHaveBeenCalledWith("game-1", "markdown"),
    );
  });
});
