import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import { companionSession, companionSnapshot } from "@/test/companionFixture";
import type { MemoryView } from "@/lib/ipc/companionMemory";
import { useCompanionStore } from "./store";

const mocks = vi.hoisted(() => ({
  view: vi.fn(),
  save: vi.fn(),
  settings: vi.fn(),
  sources: vi.fn(),
  forget: vi.fn(),
  suggest: vi.fn(),
  review: vi.fn(),
  resume: vi.fn(),
}));
vi.mock("@/lib/ipc/companionMemory", () => ({ memoryApi: mocks }));
const { CompanionMemory } = await import("./CompanionMemory");
let data: MemoryView;
beforeEach(() => {
  vi.resetAllMocks();
  useCompanionStore.setState(useCompanionStore.getInitialState(), true);
  data = {
    context: {
      companionId: "p",
      gameId: "game-one",
      playthroughId: "reading-one",
      knowledge: "observed",
    },
    settings: { revision: 3, retrievalEnabled: false, autoSummary: false, gap: null },
    items: [],
    jobs: [],
    replayExcluded: false,
    itemCount: 0,
    pinCount: 0,
    markerCount: 0,
  };
  mocks.view.mockImplementation(async () => structuredClone(data));
  mocks.save.mockResolvedValue(null);
  mocks.forget.mockResolvedValue(null);
  mocks.settings.mockResolvedValue(null);
});
function view() {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <CompanionMemory session={companionSession()} passage={null} onClose={vi.fn()} />
    </AppContext.Provider>,
  );
}
function addItem() {
  data.items = [
    {
      id: "m",
      version: 2,
      scope: "shared",
      class: "preference",
      companionId: null,
      gameId: null,
      playthroughId: null,
      knowledge: "metadata",
      body: "Short replies please",
      evidence: "reader",
      pinned: false,
      originSession: null,
      createdAt: 1,
      updatedAt: 1,
      expiresAt: null,
    },
  ];
  data.itemCount = 1;
}

describe("companion memory", () => {
  it("saves explicit text without enabling retrieval or requesting inference", async () => {
    const user = userEvent.setup();
    view();
    const use = await screen.findByRole("checkbox", { name: /Use saved memories/ });
    expect(use).not.toBeChecked();
    await user.click(screen.getByRole("button", { name: "Remember this" }));
    await user.type(screen.getByLabelText("Memory text"), "Short replies please");
    await user.click(screen.getByRole("button", { name: "Save memory" }));
    await waitFor(() =>
      expect(mocks.save).toHaveBeenCalledWith(
        expect.objectContaining({
          scope: "shared",
          class: "preference",
          body: "Short replies please",
          messageId: null,
          expectedRevision: 3,
        }),
      ),
    );
    expect(mocks.settings).not.toHaveBeenCalled();
    expect(mocks.suggest).not.toHaveBeenCalled();
  });
  it("requires a visible cost confirmation for automatic summaries", async () => {
    const user = userEvent.setup();
    view();
    await user.click(
      await screen.findByRole("checkbox", { name: /Automatic recap suggestions/ }),
    );
    expect(mocks.settings).not.toHaveBeenCalled();
    expect(screen.getByText(/Enable automatic model calls/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Confirm" }));
    await waitFor(() => expect(mocks.settings).toHaveBeenCalledWith(false, true, 3));
  });
  it("keeps the editor's original revision after a concurrent refresh", async () => {
    addItem();
    const user = userEvent.setup();
    view();
    await user.click(await screen.findByRole("button", { name: "Edit / pin" }));
    data.settings.revision = 4;
    data.items[0]!.version = 3;
    act(() => useCompanionStore.setState({ snapshot: companionSnapshot(4) }));
    await waitFor(() => expect(mocks.view).toHaveBeenCalledTimes(2));
    await user.clear(screen.getByLabelText("Memory text"));
    await user.type(screen.getByLabelText("Memory text"), "A correction");
    await user.click(screen.getByRole("button", { name: "Save memory" }));
    await waitFor(() =>
      expect(mocks.save).toHaveBeenCalledWith(
        expect.objectContaining({ id: "m", expectedVersion: 2, expectedRevision: 3 }),
      ),
    );
  });
  it("explains replay exclusion before Forget and keeps memory layout bounded", async () => {
    addItem();
    const user = userEvent.setup();
    const rendered = view();
    await user.click(await screen.findByRole("button", { name: "Forget" }));
    expect(mocks.forget).not.toHaveBeenCalled();
    expect(screen.getByText(/Every earlier companion transcript/)).toBeVisible();
    expect(screen.getByRole("alert")).toHaveFocus();
    expect(
      rendered.baseElement.querySelector(".companion-memory__item"),
    ).toBeInTheDocument();
    const buttons = screen.getAllByRole("button", { name: "Forget" });
    await user.click(buttons[buttons.length - 1]!);
    await waitFor(() =>
      expect(mocks.forget).toHaveBeenCalledWith({ kind: "item", id: "m", version: 2 }, 3),
    );
  });
  it("never allows a metadata-only session to save story material", async () => {
    const user = userEvent.setup();
    const session = companionSession();
    session.options.knowledge = "metadata";
    render(
      <AppContext.Provider
        value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
      >
        <CompanionMemory session={session} passage={null} onClose={vi.fn()} />
      </AppContext.Provider>,
    );
    await user.click(await screen.findByRole("button", { name: "Remember this" }));
    expect(
      within(screen.getByLabelText("Save for")).getByRole("option", {
        name: "This reading",
      }),
    ).toBeDisabled();
  });
});
