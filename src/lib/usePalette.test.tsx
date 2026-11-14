import { act, renderHook, waitFor } from "@testing-library/react";
import { StrictMode, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { fallbackPalette } from "./palette";
import { usePalette } from "./usePalette";

const fetchImage = vi.fn();
const decode = vi.fn();
const close = vi.fn();
const pixels = new Uint8ClampedArray([64, 180, 128, 255]);

beforeEach(() => {
  vi.clearAllMocks();
  fetchImage.mockResolvedValue({ ok: true, blob: async () => new Blob() });
  decode.mockResolvedValue({ close });
  vi.stubGlobal("fetch", fetchImage);
  vi.stubGlobal("createImageBitmap", decode);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
    drawImage: vi.fn(),
    getImageData: () => ({ data: pixels }),
  } as unknown as CanvasRenderingContext2D);
});

afterEach(() => vi.unstubAllGlobals());

describe("cover palette work", () => {
  it("shares one decode across concurrent consumers and StrictMode remounts", async () => {
    const wrapper = ({ children }: { children: ReactNode }) => (
      <StrictMode>{children}</StrictMode>
    );
    const first = renderHook(() => usePalette("shared-cover", null, "First"), {
      wrapper,
    });
    const second = renderHook(() => usePalette("shared-cover", null, "Second"), {
      wrapper,
    });
    await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
    expect(fetchImage).toHaveBeenCalledTimes(1);
    expect(decode).toHaveBeenCalledTimes(1);
    expect(first.result.current).toEqual(second.result.current);
    first.unmount();
    second.unmount();
    renderHook(() => usePalette("shared-cover", null, "First"));
    expect(fetchImage).toHaveBeenCalledTimes(1);
  });

  it("retries a failed sample on the next visit instead of caching a title fallback", async () => {
    fetchImage.mockRejectedValueOnce(new Error("offline"));
    const first = renderHook(() => usePalette("retry-cover", null, "First"));
    await act(async () => {});
    expect(first.result.current).toEqual(fallbackPalette("First"));
    first.unmount();
    const second = renderHook(() => usePalette("retry-cover", null, "Second"));
    await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
    expect(fetchImage).toHaveBeenCalledTimes(2);
    expect(second.result.current).not.toEqual(fallbackPalette("First"));
  });

  it.each(["missing", "throws"])(
    "releases the bitmap when the canvas %s",
    async (failure) => {
      vi.mocked(HTMLCanvasElement.prototype.getContext).mockImplementation(() => {
        if (failure === "throws") throw new Error("canvas unavailable");
        return null;
      });
      renderHook(() => usePalette(`canvas-${failure}`, null, "First"));
      await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
    },
  );

  it("evicts old palette keys while browsing a large library", async () => {
    const hook = renderHook(({ url }) => usePalette(url, null, "First"), {
      initialProps: { url: "evict-first" },
    });
    await act(async () => {});
    for (let index = 0; index < 64; index += 1) {
      await act(async () => hook.rerender({ url: `evict-${index}` }));
    }
    const before = fetchImage.mock.calls.length;
    await act(async () => hook.rerender({ url: "evict-first" }));
    expect(fetchImage.mock.calls.length).toBe(before + 1);
  });

  it("does not retain an oversized encoded cover as a palette cache key", async () => {
    const url = `data:image/jpeg;base64,${"A".repeat(2 * 1024 * 1024)}`;
    const first = renderHook(() => usePalette(url, null, "First"));
    await act(async () => {});
    first.unmount();
    renderHook(() => usePalette(url, null, "First"));
    await waitFor(() => expect(decode).toHaveBeenCalledTimes(2));
  });

  it("uses the stored accent without fetching or decoding the cover", () => {
    const { result } = renderHook(() => usePalette("stored-cover", "#65d8c5", "First"));
    expect(result.current.accent).toBe("#65d8c5");
    expect(fetchImage).not.toHaveBeenCalled();
    expect(decode).not.toHaveBeenCalled();
  });
});
