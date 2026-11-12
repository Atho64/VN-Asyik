import { useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { Sheet } from "@/components/Sheet";
import { api, IpcError } from "@/lib/ipc/client";
import type { ScreenshotCard } from "@/lib/ipc/types";

/**
 * Full-size view of one capture, with delete.
 *
 * Shared by the per-game section and the cross-game library so the two cannot
 * drift: a shot opened from either place behaves the same, and deleting from one
 * refreshes the other through the caller's callback.
 *
 * The image is fetched on open rather than with the list, because a full-size
 * JPEG per row would be several MB for a grid that renders at a few hundred px.
 */
export function ScreenshotLightbox({
  shot,
  onClose,
  onDeleted,
}: {
  shot: ScreenshotCard | null;
  onClose: () => void;
  onDeleted?: () => void;
}) {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const [preview, setPreview] = useState<{ id: string; src: string } | null>(null);
  const [busy, setBusy] = useState(false);

  // Keyed by id so a shot opened after another never shows the previous one's
  // picture while its own request is in flight.
  const image = shot && preview?.id === shot.id ? preview.src : null;

  useEffect(() => {
    if (!shot) return undefined;
    let active = true;
    void api
      .screenshotImage(shot.id)
      .then((next) => {
        if (active) setPreview({ id: shot.id, src: next });
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, [shot]);

  const remove = async (target: ScreenshotCard): Promise<void> => {
    setBusy(true);
    try {
      await api.screenshotDelete(target.id);
      onClose();
      onDeleted?.();
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("common.error"),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet
      open={shot !== null}
      title={t("screenshot.title")}
      onClose={onClose}
      wide
      footer={
        shot ? (
          <Button variant="danger" onClick={() => void remove(shot)} disabled={busy}>
            <Icon name="trash" size={16} />
            {t("screenshot.delete")}
          </Button>
        ) : null
      }
    >
      <figure className="lightbox">
        {image ? (
          <img
            className="lightbox__img"
            src={image}
            alt={t("screenshot.alt", {
              date: shot ? new Date(shot.capturedAt).toLocaleDateString(locale) : "",
            })}
            width={shot?.width}
            height={shot?.height}
          />
        ) : null}
        {shot ? (
          <figcaption className="caption lightbox__caption">
            {t("screenshot.capturedAt", {
              date: new Date(shot.capturedAt).toLocaleString(locale),
            })}
            {" · "}
            {t(`screenshot.source.${shot.source}`)}
          </figcaption>
        ) : null}
      </figure>
    </Sheet>
  );
}
