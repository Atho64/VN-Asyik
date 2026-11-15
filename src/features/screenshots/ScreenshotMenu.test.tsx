import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppContext } from "@/app/AppProvider";
import { ContextMenuProvider } from "@/app/ContextMenuProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { Sheet } from "@/components/Sheet";

const mocks = vi.hoisted(() => ({ copy: vi.fn(), save: vi.fn() }));
vi.mock("@/lib/ipc/client", () => ({
  api: { screenshotCopy: mocks.copy, screenshotSave: mocks.save },
}));

const source = "data:image/jpeg;base64,painted-image";
function mount(src = source, modalClose?: () => void) {
  const image = (
    <img src={src} alt="Screenshot" data-screenshot-filename="screenshot-sf123.png" />
  );
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <ContextMenuProvider>
        {modalClose ? (
          <Sheet open title="Screenshot preview" onClose={modalClose}>
            {image}
          </Sheet>
        ) : (
          <button type="button" aria-label="Open screenshot">
            {image}
          </button>
        )}
      </ContextMenuProvider>
    </AppContext.Provider>,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.copy.mockResolvedValue(undefined);
  mocks.save.mockResolvedValue(true);
  useAppStore.setState({
    toasts: [],
    platform: {
      os: "windows",
      kind: "desktop",
      first_class: true,
      process_tracking: "experimental",
      chapter_detection: "experimental",
      discord_presence: "supported",
      cloud_sync: "unavailable",
    },
  });
});

describe("screenshot context actions", () => {
  it("copies the painted image without fetching another source", async () => {
    mount();
    fireEvent.contextMenu(screen.getByRole("img", { name: "Screenshot" }), {
      clientX: 40,
      clientY: 60,
    });
    await userEvent.click(screen.getByRole("menuitem", { name: "Copy image" }));
    expect(mocks.copy).toHaveBeenCalledWith(source);
    await waitFor(() =>
      expect(useAppStore.getState().toasts.at(-1)?.message).toBe("Image copied"),
    );
  });

  it("saves with a filename and treats picker cancellation as a normal outcome", async () => {
    mocks.save.mockResolvedValue(false);
    mount();
    fireEvent.contextMenu(screen.getByRole("button", { name: "Open screenshot" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Save image…" }));
    expect(mocks.save).toHaveBeenCalledWith(source, "screenshot-sf123.png");
    await waitFor(() => expect(mocks.save).toHaveResolved());
    expect(useAppStore.getState().toasts).toEqual([]);
  });

  it("opens by keyboard, moves focus, closes without activating, and restores focus", async () => {
    mount();
    const tile = screen.getByRole("button", { name: "Open screenshot" });
    tile.focus();
    fireEvent.keyDown(tile, { key: "F10", shiftKey: true });
    expect(screen.getByRole("menuitem", { name: "Copy image" })).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(screen.getByRole("menuitem", { name: "Save image…" })).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(tile).toHaveFocus();
    expect(mocks.copy).not.toHaveBeenCalled();
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it("exports a concealed image and refuses a source changed after opening the menu", async () => {
    const { rerender } = mount("data:image/jpeg;base64,blurred");
    fireEvent.contextMenu(screen.getByRole("img"));
    await userEvent.click(screen.getByRole("menuitem", { name: "Copy image" }));
    expect(mocks.copy).toHaveBeenCalledWith("data:image/jpeg;base64,blurred");
    await waitFor(() => expect(useAppStore.getState().toasts).toHaveLength(1));
    fireEvent.contextMenu(screen.getByRole("img"));
    screen.getByRole("img").setAttribute("src", source);
    await userEvent.click(screen.getByRole("menuitem", { name: "Copy image" }));
    expect(mocks.copy).toHaveBeenCalledTimes(1);
    // Leaving the screen also makes the retained source ineligible.
    rerender(<div />);
  });

  it("keeps the menu inside the lightbox and Escape dismisses only the menu", () => {
    const modalClose = vi.fn();
    mount(source, modalClose);
    fireEvent.contextMenu(screen.getByRole("img"));
    expect(screen.getByRole("dialog")).toContainElement(screen.getByRole("menu"));
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(modalClose).not.toHaveBeenCalled();
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(modalClose).toHaveBeenCalledOnce();
  });

  it("shows a copy failure and keeps saving available on Android", async () => {
    mocks.copy.mockRejectedValue(new Error("clipboard unavailable"));
    mount();
    fireEvent.contextMenu(screen.getByRole("img"));
    await userEvent.click(screen.getByRole("menuitem", { name: "Copy image" }));
    await waitFor(() => expect(useAppStore.getState().toasts.at(-1)?.kind).toBe("error"));
    act(() => {
      useAppStore.setState({
        platform: { ...useAppStore.getState().platform!, os: "android", kind: "mobile" },
      });
    });
    fireEvent.contextMenu(screen.getByRole("img"));
    expect(
      screen.getByRole("menuitem", { name: "Copy image (desktop only)" }),
    ).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: "Save image…" })).toBeEnabled();
  });
});
