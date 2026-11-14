import { useCallback, useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Icon } from "@/components/Icon";
import { Sheet } from "@/components/Sheet";
import { api } from "@/lib/ipc/client";
import type { ScreenshotView } from "@/lib/ipc/types";

/**
 * Window-lifetime tile cache per `(vnId, imageId)` avoiding redundant IPC
 * round trips across journal saves and focus changes. Reveals obey `show`/`blur`/`hide`.
 */
const tileCache = new Map<string, Promise<ScreenshotView>>();

function loadTile(vnId: string, imageId: string): Promise<ScreenshotView> {
  const key = `${vnId}:${imageId}`;
  const cached = tileCache.get(key);
  if (cached) return cached;
  const pending = api.vndbScreenshotTile(vnId, imageId);
  tileCache.set(key, pending);
  // Evict rejected fetches so retries succeed while gallery batches finish downloading.
  pending.catch(() => tileCache.delete(key));
  return pending;
}

interface JournalPhotoProps {
  /** The game's VN id. A manual entry has none, so it has no gallery either. */
  vnId: string | null;
  /** VNDB image id, e.g. `sf83040`. */
  imageId: string;
  /** Game title, for the alt text and the lightbox label. */
  title: string;
}

export function JournalPhoto({ vnId, imageId, title }: JournalPhotoProps) {
  const { t } = useI18n();
  const nsfw = useAppStore((state) => state.prefs.nsfw);
  const [loaded, setLoaded] = useState<{
    key: string;
    view: ScreenshotView | null;
  } | null>(null);
  const [open, setOpen] = useState(false);
  const [full, setFull] = useState<ScreenshotView | null>(null);
  const [fullFailed, setFullFailed] = useState(false);

  const key = `${vnId ?? ""}:${imageId}`;
  const revealScope = `${key}:${nsfw}`;
  const [revealed, setRevealed] = useState<{ key: string; shown: boolean }>({
    key: revealScope,
    shown: false,
  });
  // A policy change re-conceals on the same render, the way `useArtwork` does.
  if (revealed.key !== revealScope) {
    setRevealed({ key: revealScope, shown: false });
  }
  const isRevealed = revealed.key === revealScope && revealed.shown;

  useEffect(() => {
    if (!vnId) return undefined;
    let cancelled = false;
    void loadTile(vnId, imageId).then(
      (view) => {
        if (!cancelled) setLoaded({ key, view });
      },
      () => {
        if (!cancelled) setLoaded({ key, view: null });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [vnId, imageId, key]);

  const closeLightbox = useCallback(() => {
    setOpen(false);
    setFull(null);
    setFullFailed(false);
  }, []);

  useEffect(() => {
    if (!open || !vnId) return undefined;
    let cancelled = false;
    void api.vndbScreenshotFetch(vnId, imageId).then(
      (view) => {
        if (!cancelled) setFull(view);
      },
      () => {
        if (!cancelled) setFullFailed(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [open, vnId, imageId]);

  const view = loaded?.key === key ? loaded.view : null;
  if (!view) {
    // A pinned photo whose bytes are gone (cache cleared, or the image is still
    // downloading) leaves a quiet gap rather than a broken image icon.
    return (
      <p className="journal-photo journal-photo--gap">
        {loaded?.key === key ? t("reading.photoMissing") : t("common.loading")}
      </p>
    );
  }

  const concealed = nsfw !== "show" && view.explicit && !isRevealed;
  const policy: "visible" | "blur" | "hide" = !concealed
    ? "visible"
    : nsfw === "blur" && view.blurred_src
      ? "blur"
      : "hide";
  const painted =
    policy === "blur" ? view.blurred_src : policy === "hide" ? null : view.src;

  return (
    <>
      <figure className="journal-photo">
        <button
          type="button"
          className="journal-photo__tile"
          aria-label={
            concealed ? t("reading.photoReveal") : t("reading.photoOpen", { title })
          }
          onClick={() => {
            // The first click on a concealed photo reveals it; opening straight
            // into a full-size explicit image is what "blur until revealed" forbids.
            if (concealed) {
              setRevealed({ key: revealScope, shown: true });
              return;
            }
            setOpen(true);
          }}
        >
          {painted ? (
            <img
              className={
                policy === "blur"
                  ? "journal-photo__img journal-photo__img--concealed"
                  : "journal-photo__img"
              }
              src={painted}
              alt={concealed ? "" : t("reading.photoAlt", { title })}
              aria-hidden={concealed ? "true" : undefined}
              loading="lazy"
              decoding="async"
              width={view.width ?? undefined}
              height={view.height ?? undefined}
            />
          ) : (
            <span className="journal-photo__neutral" aria-hidden="true" />
          )}
          {concealed && (
            <span className="journal-photo__scrim">
              <Icon name="eyeOff" size={16} />
            </span>
          )}
        </button>
      </figure>

      <Sheet open={open} title={t("reading.photoTitle", { title })} onClose={closeLightbox} wide>
        {fullFailed && <p className="body">{t("reading.photoMissing")}</p>}
        {!full && !fullFailed && <p role="status">{t("common.loading")}</p>}
        {full && (
          <img
            className="lightbox__img"
            src={
              nsfw !== "show" && full.explicit && full.blurred_src
                ? full.blurred_src
                : full.src
            }
            alt={t("reading.photoAlt", { title })}
          />
        )}
      </Sheet>
    </>
  );
}
