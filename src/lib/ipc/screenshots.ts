import { listen } from "@tauri-apps/api/event";

/** Outcome of a capture started by the global shortcut. */
export interface ShortcutCapture {
  ok: boolean;
  id?: string;
  code?: string;
}

/**
 * The global shortcut's result.
 *
 * A shortcut that fails silently is indistinguishable from one that is not
 * bound, so the backend reports every attempt and the UI says what happened.
 */
export const listenScreenshotShortcut = (onResult: (result: ShortcutCapture) => void) =>
  listen<ShortcutCapture>("vn:screenshot-captured", ({ payload }) => {
    if (payload && typeof payload.ok === "boolean") onResult(payload);
  });
