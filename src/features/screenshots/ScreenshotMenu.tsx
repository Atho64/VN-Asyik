import { useEffect, useRef } from "react";
import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import type { MenuItem } from "@/components/ContextMenu";
import { api } from "@/lib/ipc/client";

/** Delegation reaches portal lightboxes; only tagged screenshot images opt in. */
export function ScreenshotMenu({
  openMenu,
}: {
  openMenu: (position: { clientX: number; clientY: number }, items: MenuItem[]) => void;
}) {
  const { t } = useI18n();
  const platform = useAppStore((state) => state.platform);
  const pushToast = useAppStore((state) => state.pushToast);
  const busy = useRef(false);
  useEffect(() => {
    const findImage = (target: EventTarget | null): HTMLImageElement | null => {
      if (!(target instanceof Element)) return null;
      return (
        target.closest<HTMLImageElement>("img[data-screenshot-filename]") ??
        target
          .closest("button")
          ?.querySelector<HTMLImageElement>("img[data-screenshot-filename]") ??
        null
      );
    };
    const open = (image: HTMLImageElement, clientX: number, clientY: number) => {
      const src = image.currentSrc || image.src;
      if (!src.startsWith("data:image/")) return;
      const filename = image.dataset.screenshotFilename ?? "screenshot.png";
      (image.closest("button") ?? image).focus();
      const perform = async (action: "copy" | "save") => {
        // Policy changes and navigation invalidate a formerly revealed source.
        if (busy.current || !image.isConnected || (image.currentSrc || image.src) !== src)
          return;
        busy.current = true;
        try {
          if (action === "copy") {
            await api.screenshotCopy(src);
            pushToast("success", t("screenshot.copied"));
          } else if (await api.screenshotSave(src, filename)) {
            pushToast("success", t("screenshot.saved"));
          }
        } catch {
          pushToast(
            "error",
            t(action === "copy" ? "screenshot.copyFailed" : "screenshot.saveFailed"),
          );
        } finally {
          busy.current = false;
        }
      };
      openMenu({ clientX, clientY }, [
        {
          label: t(
            platform?.kind === "mobile" ? "screenshot.copyDesktop" : "screenshot.copy",
          ),
          disabled: platform?.kind !== "desktop" || busy.current,
          onSelect: () => {
            void perform("copy");
          },
        },
        {
          label: t("screenshot.save"),
          disabled: busy.current,
          onSelect: () => {
            void perform("save");
          },
        },
      ]);
    };
    const context = (event: MouseEvent) => {
      const image = findImage(event.target);
      if (!image) return;
      event.preventDefault();
      event.stopPropagation();
      open(image, event.clientX, event.clientY);
    };
    const key = (event: KeyboardEvent) => {
      if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
      const image = findImage(event.target);
      if (!image) return;
      event.preventDefault();
      event.stopPropagation();
      const box = image.getBoundingClientRect();
      open(image, box.left + box.width / 2, box.top + box.height / 2);
    };
    document.addEventListener("contextmenu", context, true);
    document.addEventListener("keydown", key, true);
    return () => {
      document.removeEventListener("contextmenu", context, true);
      document.removeEventListener("keydown", key, true);
    };
  }, [openMenu, platform?.kind, pushToast, t]);
  return null;
}
