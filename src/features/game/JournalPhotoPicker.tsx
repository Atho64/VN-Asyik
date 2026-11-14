import { useCallback, useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { api } from "@/lib/ipc/client";
import type { ScreenshotView } from "@/lib/ipc/types";

interface JournalPhotoPickerProps {
  /** The game's VN id. A manual entry has none, so it has no gallery either. */
  vnId: string | null;
  title: string;
  /** Currently pinned image id, so the chosen tile can be marked. */
  selected: string;
  onSelect: (imageId: string) => void;
}

/**
 * Pinned-checkpoint screenshot picker reading already-cached thumbnails so opening
 * the journal form never triggers an unsolicited batch download.
 */
export function JournalPhotoPicker({
  vnId,
  title,
  selected,
  onSelect,
}: JournalPhotoPickerProps) {
  const { t } = useI18n();
  const nsfw = useAppStore((state) => state.prefs.nsfw);
  const [state, setState] = useState<{
    status: "loading" | "ready" | "error";
    shots: ScreenshotView[];
    pending: number;
  }>({ status: "loading", shots: [], pending: 0 });
  const [busy, setBusy] = useState(false);

  // Initial load happens in the effect; explicit "download more" reloads run from the click handler.
  useEffect(() => {
    if (!vnId) return undefined;
    let cancelled = false;
    void api.vndbScreenshots(vnId).then(
      (gallery) => {
        if (cancelled) return;
        setState({
          status: "ready",
          shots: gallery.shots,
          pending: gallery.pending,
        });
      },
      () => {
        if (!cancelled) setState({ status: "error", shots: [], pending: 0 });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [vnId]);

  const loadMore = useCallback(() => {
    if (!vnId || busy) return;
    setBusy(true);
    void api.vndbScreenshots(vnId).then(
      (gallery) => {
        setState({
          status: "ready",
          shots: gallery.shots,
          pending: gallery.pending,
        });
        setBusy(false);
      },
      () => {
        setBusy(false);
      },
    );
  }, [vnId, busy]);

  if (!vnId) {
    return <p className="body">{t("reading.photoNoGallery")}</p>;
  }
  if (state.status === "loading") {
    return (
      <div aria-busy="true" aria-label={t("reading.photoPick")}>
        <div className="gallery">
          {Array.from({ length: 6 }, (_, index) => (
            <div key={index} className="gallery__tile skeleton" />
          ))}
        </div>
      </div>
    );
  }
  if (state.status === "error") {
    return <p className="body">{t("detail.screenshotsEmpty")}</p>;
  }
  if (state.shots.length === 0) {
    return <p className="body">{t("detail.screenshotsEmpty")}</p>;
  }

  return (
    <>
      <ul className="gallery gallery--picker">
        {state.shots.map((shot) => {
          // Apply the shared three-way NSFW rule without maintaining a separate reveal state in the picker.
          const concealed = nsfw !== "show" && shot.explicit;
          const painted = !concealed
            ? shot.src
            : nsfw === "blur" && shot.blurred_src
              ? shot.blurred_src
              : null;
          const isSelected = shot.image_id === selected;
          return (
            <li key={shot.image_id} className="gallery__item">
              <button
                type="button"
                className={
                  isSelected
                    ? "gallery__tile gallery__tile--selected"
                    : "gallery__tile"
                }
                aria-pressed={isSelected}
                aria-label={t("reading.photoChoose", { title })}
                onClick={() => onSelect(shot.image_id)}
              >
                {painted ? (
                  <img
                    className={
                      concealed
                        ? "gallery__img gallery__img--concealed"
                        : "gallery__img"
                    }
                    src={painted}
                    alt=""
                    aria-hidden="true"
                    loading="lazy"
                    decoding="async"
                    width={shot.width ?? undefined}
                    height={shot.height ?? undefined}
                  />
                ) : (
                  <span className="gallery__neutral" aria-hidden="true" />
                )}
                {isSelected && (
                  <span className="gallery__scrim gallery__scrim--selected">
                    <Icon name="check" size={16} />
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
      {state.pending > 0 && (
        <div className="reading__picker-more">
          <p className="caption">
            {t("reading.photoPending", { count: state.pending })}
          </p>
          <Button variant="secondary" disabled={busy} onClick={loadMore}>
            {busy ? t("common.loading") : t("reading.photoLoadMore")}
          </Button>
        </div>
      )}
    </>
  );
}
