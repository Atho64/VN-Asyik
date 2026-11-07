import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, expect, it, vi } from "vitest";
import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import { companionSnapshot } from "@/test/companionFixture";
import { companionApi } from "@/lib/ipc/companion";
import type { VisionPreview } from "@/lib/ipc/companionVision";
import { useCompanionStore } from "./store";
const mocks = vi.hoisted(() => ({
  windows: vi.fn(),
  select: vi.fn(),
  preview: vi.fn(),
  probe: vi.fn(),
  control: vi.fn(),
  send: vi.fn(),
}));
vi.mock("@/lib/ipc/companionVision", () => ({ visionApi: mocks }));
const { CompanionVision } = await import("./CompanionVision");
let snap = companionSnapshot();
beforeEach(() => {
  vi.resetAllMocks();
  useCompanionStore.setState(useCompanionStore.getInitialState(), true);
  snap = companionSnapshot();
  snap.capabilities.vision = "available";
  mocks.windows.mockResolvedValue([{ id: "window-token", title: "Synthetic game" }]);
  mocks.control.mockImplementation(async () => snap);
  vi.spyOn(companionApi, "snapshot").mockImplementation(async () => snap);
});
function view() {
  useCompanionStore.setState({ snapshot: snap });
  function Surface() {
    const snapshot = useCompanionStore((s) => s.snapshot)!;
    return <CompanionVision snapshot={snapshot} overlay={false} />;
  }
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <MemoryRouter>
        <Surface />
      </MemoryRouter>
    </AppContext.Provider>,
  );
}
it("does not inspect windows, probe or send anything until explicitly requested", async () => {
  const user = userEvent.setup();
  view();
  await user.click(screen.getByRole("button", { name: "Vision" }));
  expect(screen.getByText(/0\/12 requests this hour; 0\/30 this session/)).toBeVisible();
  expect(mocks.windows).not.toHaveBeenCalled();
  expect(mocks.probe).not.toHaveBeenCalled();
  expect(mocks.send).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Find game windows" }));
  expect(mocks.windows).toHaveBeenCalledOnce();
  expect(
    screen.getByRole("button", { name: "Select window and preview" }),
  ).toBeDisabled();
});
it("shows the exact preview, endpoint and cost before enabling automatic sharing", async () => {
  const user = userEvent.setup();
  view();
  await user.click(screen.getByRole("button", { name: "Vision" }));
  await user.click(screen.getByRole("button", { name: "Find game windows" }));
  await user.selectOptions(screen.getByLabelText("Game window"), "window-token");
  const next = companionSnapshot(2);
  next.capabilities.vision = "available";
  next.vision = {
    ...next.vision,
    generation: 7,
    status: "preview",
    window: "Synthetic game",
    support: "accepted",
    provider: {
      provider: "openai",
      model: "Synthetic model",
      baseUrl: "http://127.0.0.1:1234",
    },
  };
  mocks.select.mockImplementation(async () => {
    snap = next;
    return {
      token: "immutable-frame",
      dataUrl: "data:image/jpeg;base64,/9j/",
      width: 32,
      height: 24,
      generation: 7,
      crop: { x: 0, y: 0, width: 1000, height: 1000 },
      view: next.vision,
    };
  });
  mocks.control.mockResolvedValue(next);
  await user.click(screen.getByRole("button", { name: "Select window and preview" }));
  expect(await screen.findByAltText("Selected game scene preview")).toHaveAttribute(
    "src",
    "data:image/jpeg;base64,/9j/",
  );
  expect(screen.getByText(/Image and text calls may cost money/)).toBeVisible();
  expect(screen.getByText(/http:\/\/127.0.0.1:1234/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Watch and react" })).toBeDisabled();
  expect(mocks.probe).not.toHaveBeenCalled();
  expect(mocks.control).not.toHaveBeenCalled();
  await user.click(screen.getByRole("checkbox"));
  await user.click(screen.getByRole("button", { name: "Watch and react" }));
  await waitFor(() =>
    expect(mocks.control).toHaveBeenCalledWith(
      "session-one",
      "watch",
      7,
      "immutable-frame",
    ),
  );
});
it("withholds selection in metadata-only sessions and reports unsupported platforms", async () => {
  const user = userEvent.setup();
  snap.session!.options.knowledge = "metadata";
  view();
  await user.click(screen.getByRole("button", { name: "Vision" }));
  expect(screen.getByText(/Start an Observed or All session/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Find game windows" })).toBeDisabled();
});

it.each(["close", "unmount"])(
  "dismisses a capture that finishes after %s",
  async (action) => {
    const user = userEvent.setup();
    const surface = view();
    await user.click(screen.getByRole("button", { name: "Vision" }));
    await user.click(screen.getByRole("button", { name: "Find game windows" }));
    await user.selectOptions(screen.getByLabelText("Game window"), "window-token");
    let finish!: (preview: VisionPreview) => void;
    mocks.select.mockReturnValue(
      new Promise<VisionPreview>((resolve) => {
        finish = resolve;
      }),
    );
    await user.click(screen.getByRole("button", { name: "Select window and preview" }));
    if (action === "close")
      await user.click(screen.getByRole("button", { name: "Close" }));
    else surface.unmount();
    await act(async () =>
      finish({
        token: "late-frame",
        dataUrl: "data:image/jpeg;base64,/9j/",
        width: 32,
        height: 24,
        generation: 7,
        crop: { x: 0, y: 0, width: 1000, height: 1000 },
        view: snap.vision,
      }),
    );
    await waitFor(() =>
      expect(mocks.control).toHaveBeenCalledWith(
        "session-one",
        "dismiss",
        7,
        "late-frame",
      ),
    );
    expect(screen.queryByAltText("Selected game scene preview")).not.toBeInTheDocument();
    expect(mocks.send).not.toHaveBeenCalled();
  },
);
