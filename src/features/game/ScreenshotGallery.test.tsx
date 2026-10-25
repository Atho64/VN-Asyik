import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import type { NsfwMode, ScreenshotGallery, ScreenshotView } from "@/lib/ipc/types";

const mocks = vi.hoisted(() => ({
  vndbScreenshots: vi.fn(),
  vndbScreenshotFetch: vi.fn(),
  openExternal: vi.fn(),
}));

vi.mock("@/lib/ipc/client", () => ({
  IpcError: class extends Error {},
  api: {
    vndbScreenshots: mocks.vndbScreenshots,
    vndbScreenshotFetch: mocks.vndbScreenshotFetch,
    openExternal: mocks.openExternal,
  },
}));

const { ScreenshotGallery: Gallery } = await import("./ScreenshotGallery");
const t = createTranslator("en");

const blurred = "data:image/jpeg;base64,Ymx1cg==";
const sharp = "data:image/jpeg;base64,c2hhcnA=";

function shot(overrides: Partial<ScreenshotView> = {}): ScreenshotView {
  return {
    image_id: "sf1",
    vndb_url: "https://vndb.org/sf1",
    src: sharp,
    blurred_src: blurred,
    sharp: true,
    explicit: false,
    width: 512,
    height: 384,
    ...overrides,
  };
}

function gallery(overrides: Partial<ScreenshotGallery> = {}): ScreenshotGallery {
  return {
    shots: [shot()],
    reported: 1,
    complete: true,
    pending: 0,
    ...overrides,
  };
}

function setPolicy(nsfw: NsfwMode) {
  useAppStore.setState({
    prefs: { theme: "dark", language: "en", nsfw, reduce_effects: false },
  });
}

function renderGallery(vnId: string | null = "v17", version = 0) {
  return render(
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      <Gallery vnId={vnId} title="A visual novel" version={version} />
    </AppContext.Provider>,
  );
}

function tiles(container: HTMLElement): HTMLImageElement[] {
  return [...container.querySelectorAll<HTMLImageElement>(".gallery__img")];
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.vndbScreenshots.mockResolvedValue(gallery());
  mocks.vndbScreenshotFetch.mockResolvedValue(shot());
  mocks.openExternal.mockResolvedValue(undefined);
  setPolicy("blur");
});

describe("screenshot gallery", () => {
  it("does not request anything for a hand-added game with no VN", async () => {
    renderGallery(null);
    expect(mocks.vndbScreenshots).not.toHaveBeenCalled();
    expect(await screen.findByText("VNDB has no screenshots for this visual novel.")).toBeTruthy();
  });

  it("says so when VNDB has no screenshots", async () => {
    mocks.vndbScreenshots.mockResolvedValue(gallery({ shots: [], reported: 0 }));
    renderGallery();
    expect(await screen.findByText("VNDB has no screenshots for this visual novel.")).toBeTruthy();
  });

  it("paints what the backend resolved, without second-guessing the policy", async () => {
    // The backend hands over both copies and states the rating; the decision between
    // them is the surface's, exactly as it is for a cover.
    mocks.vndbScreenshots.mockResolvedValue(
      gallery({ shots: [shot({ explicit: true })] }),
    );
    const { container } = renderGallery();
    await waitFor(() => expect(tiles(container)).toHaveLength(1));
    // Under `blur` an explicit tile paints its own pre-blurred copy, not the sharp one.
    expect(tiles(container)[0]).toHaveAttribute("src", blurred);
  });

  it("paints the sharp copy of a harmless tile even though a blur exists", async () => {
    // `blurred_src` is present for every cached image, so a missing policy check would
    // blur the whole gallery.
    const { container } = renderGallery();
    await waitFor(() => expect(tiles(container)).toHaveLength(1));
    expect(tiles(container)[0]).toHaveAttribute("src", sharp);
  });

  it("paints nothing at all under the hide policy, matching a hidden cover", async () => {
    // The one visible difference between Blur and Hide. For two phases both modes
    // painted the soft copy, so the setting did nothing a user could see.
    setPolicy("hide");
    mocks.vndbScreenshots.mockResolvedValue(
      gallery({ shots: [shot({ explicit: true })] }),
    );
    const { container } = renderGallery();
    await waitFor(() => expect(screen.getByText("Explicit screenshot hidden")).toBeTruthy());
    expect(tiles(container)).toHaveLength(0);
    expect(container.querySelector(".gallery__neutral")).toBeTruthy();
  });

  it("shows the sharp copy under the show policy", async () => {
    setPolicy("show");
    mocks.vndbScreenshots.mockResolvedValue(
      gallery({ shots: [shot({ explicit: true })] }),
    );
    const { container } = renderGallery();
    await waitFor(() => expect(tiles(container)).toHaveLength(1));
    expect(tiles(container)[0]).toHaveAttribute("src", sharp);
    expect(screen.queryByText("Explicit screenshot hidden")).toBeNull();
  });

  it("reveals to the sharp copy without re-fetching the gallery", async () => {
    // The cover reveal swaps a URL it already holds; the gallery must do the same, or a
    // reveal would cost a round trip and the policy would be split across two sides
    // again.
    mocks.vndbScreenshots.mockResolvedValue(
      gallery({ shots: [shot({ explicit: true })] }),
    );
    const { container } = renderGallery();
    await waitFor(() => expect(tiles(container)[0]).toHaveAttribute("src", blurred));
    await userEvent.click(
      screen.getByRole("button", { name: "Show this screenshot" }),
    );
    await waitFor(() => expect(tiles(container)[0]).toHaveAttribute("src", sharp));
    expect(mocks.vndbScreenshots).toHaveBeenCalledTimes(1);
  });

  it("labels an explicit tile and hides it from assistive tech", async () => {
    mocks.vndbScreenshots.mockResolvedValue(
      gallery({ shots: [shot({ explicit: true })] }),
    );
    const { container } = renderGallery();
    await waitFor(() => expect(tiles(container)).toHaveLength(1));
    expect(screen.getByText("Explicit screenshot hidden")).toBeTruthy();
    // The alt text would describe art the user has not agreed to see.
    expect(tiles(container)[0]).toHaveAttribute("aria-hidden", "true");
    expect(tiles(container)[0]).toHaveAttribute("alt", "");
  });

  it("reveals a concealed tile on the first click instead of opening it", async () => {
    // Opening straight into a full-size explicit image is exactly what "blur until
    // revealed" forbids, so the first activation only uncovers the tile.
    mocks.vndbScreenshots.mockResolvedValue(
      gallery({ shots: [shot({ explicit: true })] }),
    );
    renderGallery();
    const button = await screen.findByRole("button", {
      name: "Show this screenshot",
    });
    await userEvent.click(button);

    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Open screenshot 1 of 1" }),
      ).toBeTruthy(),
    );
  });

  it("opens the lightbox on a harmless screenshot and fetches the full size", async () => {
    renderGallery();
    await userEvent.click(
      await screen.findByRole("button", { name: "Open screenshot 1 of 1" }),
    );
    expect(await screen.findByRole("dialog")).toBeTruthy();
    expect(mocks.vndbScreenshotFetch).toHaveBeenCalledWith("v17", "sf1");
  });

  it("reports a shortfall instead of silently showing fewer images", async () => {
    mocks.vndbScreenshots.mockResolvedValue(
      gallery({ shots: [shot()], reported: 12, complete: false }),
    );
    renderGallery();
    expect(
      await screen.findByText(
        "Showing 1 of 12: the rest could not be downloaded right now.",
      ),
    ).toBeTruthy();
  });

  it("keeps the page usable when the gallery fails to load", async () => {
    // The rest of the detail screen is still readable, so this must not throw.
    mocks.vndbScreenshots.mockRejectedValue(new Error("offline"));
    renderGallery();
    expect(await screen.findByText("VNDB has no screenshots for this visual novel.")).toBeTruthy();
  });

  it("re-reads the list when the payload is refreshed", async () => {
    // A refresh can change which screenshots exist, and the VN id does not change.
    const first = renderGallery("v17", 0);
    await waitFor(() => expect(mocks.vndbScreenshots).toHaveBeenCalledTimes(1));
    first.unmount();
    renderGallery("v17", 1);
    await waitFor(() => expect(mocks.vndbScreenshots).toHaveBeenCalledTimes(2));
  });

  it("opens the image's VNDB page through the validated command", async () => {
    // Not an anchor: the URL is opened by Rust, which checks the scheme first.
    renderGallery();
    await userEvent.click(
      await screen.findByRole("button", { name: "Open screenshot 1 of 1" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Open this screenshot on VNDB" }));
    expect(mocks.openExternal).toHaveBeenCalledWith("https://vndb.org/sf1");
  });
});
