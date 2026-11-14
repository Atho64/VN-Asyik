import { useCallback, useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { Sheet } from "@/components/Sheet";
import { api, IpcError } from "@/lib/ipc/client";
import type { ScreenshotView } from "@/lib/ipc/types";

/** Generous timeout for sequential rate-limited download of up to 30 screenshots. */
const GALLERY_TIMEOUT_MS = 120_000;

/** Cap polling rounds to prevent infinite loops if pending count stops progressing. */
const MAX_ROUNDS = 8;

interface ScreenshotGalleryProps {
  /** VN id, e.g. `v17`. Absent for a manual entry, which has no gallery. */
  vnId: string | null;
  /** Game title, used in alt text so a screen reader says which VN it is. */
  title: string;
  /** Changes when the metadata is refreshed, so the gallery re-reads the list. */
  version?: number;
}

/**
 * Cached VNDB screenshot gallery that defers downloads until mount and receives
 * pre-blurred copies directly from Rust for explicit images.
 */
export function ScreenshotGallery({ vnId, title, version = 0 }: ScreenshotGalleryProps) {
  const { t } = useI18n();
  const nsfw = useAppStore((state) => state.prefs.nsfw);
  const pushToast = useAppStore((state) => state.pushToast);

  // Store results with the query key so stale async loads cannot overwrite newer game/policy state.
  const galleryKey = `${vnId ?? ""}:${version}`;
  const [loaded, setLoaded] = useState<{
    key: string;
    status: "ready" | "error";
    shots: ScreenshotView[];
    reported: number;
    complete: boolean;
    pending: number;
  } | null>(null);
  // Policy is not part of the fetch key because the backend returns both sharp and pre-blurred URLs;
  // scoping reveal state by `${galleryKey}:${nsfw}` ensures toggling back to blur re-conceals tiles.
  const revealScope = `${galleryKey}:${nsfw}`;
  const [revealed, setRevealed] = useState<{ key: string; ids: string[] }>({
    key: revealScope,
    ids: [],
  });
  const [openIndex, setOpenIndex] = useState<number | null>(null);
  const [fullView, setFullView] = useState<ScreenshotView | null>(null);
  const [fullFailed, setFullFailed] = useState(false);

  useEffect(() => {
    if (!vnId) return undefined;
    let cancelled = false;
    const settle = (next: {
      status: "ready" | "error";
      shots: ScreenshotView[];
      reported: number;
      complete: boolean;
      pending: number;
    }): void => {
      if (!cancelled) setLoaded({ key: galleryKey, ...next });
    };

    // Bound first-visit batched download (up to 30 screenshots) so stalled requests fail visibly.
    const timeout = window.setTimeout(() => {
      settle({
        status: "error",
        shots: [],
        reported: 0,
        complete: true,
        pending: 0,
      });
    }, GALLERY_TIMEOUT_MS);

    // Poll batched downloads progressively so tiles appear without waiting for the whole set.
    const loadRound = async (): Promise<void> => {
      // Cap polling rounds if the backend continues reporting pending screenshots.
      for (let round = 0; round < MAX_ROUNDS; round += 1) {
        const gallery = await api.vndbScreenshots(vnId);
        if (cancelled) return;
        settle({
          status: "ready",
          shots: gallery.shots,
          reported: gallery.reported,
          complete: gallery.complete,
          pending: gallery.pending,
        });
        if (gallery.pending === 0) return;
      }
    };

    void loadRound().then(
      () => window.clearTimeout(timeout),
      () => {
        window.clearTimeout(timeout);
        if (cancelled) return;
        // A gallery that fails to load must not take the page with it: the rest of
        // the detail screen is still readable. Already-painted rounds are kept, so a
        // failure on the third batch does not blank the first two.
        setLoaded((current) =>
          current && current.key === galleryKey && current.shots.length > 0
            ? current
            : {
                key: galleryKey,
                status: "error",
                shots: [],
                reported: 0,
                complete: true,
                pending: 0,
              },
        );
      },
    );
    return () => {
      cancelled = true;
      window.clearTimeout(timeout);
    };
  }, [vnId, galleryKey]);

  // Key reveals by policy value so policy changes revoke per-tile reveals on the same render.
  const revealedIds = revealed.key === revealScope ? revealed.ids : [];
  const remote = loaded?.key === galleryKey ? loaded : null;
  const shots = remote?.shots ?? [];
  const reported = remote?.reported ?? 0;
  const complete = remote?.complete ?? true;
  const pending = remote?.pending ?? 0;
  const state: "loading" | "ready" | "error" = !vnId
    ? "ready"
    : !remote
      ? "loading"
      : remote.status;

  const addReveal = useCallback((imageId: string) => {
    setRevealed((current) => ({
      key: revealScope,
      ids: current.key === revealScope ? [...current.ids, imageId] : [imageId],
    }));
  }, [revealScope]);

  const closeLightbox = useCallback(() => {
    setOpenIndex(null);
    setFullView(null);
    setFullFailed(false);
  }, []);

  const openAt = useCallback((index: number) => {
    setOpenIndex(index);
    setFullView(null);
    setFullFailed(false);
  }, []);

  const step = useCallback(
    (delta: number) => {
      setOpenIndex((current) => {
        if (current === null || shots.length === 0) return current;
        return (current + delta + shots.length) % shots.length;
      });
      setFullView(null);
      setFullFailed(false);
    },
    [shots.length],
  );

  const current = openIndex === null ? null : (shots[openIndex] ?? null);

  useEffect(() => {
    if (!vnId || !current) return undefined;
    let cancelled = false;
    void api.vndbScreenshotFetch(vnId, current.image_id).then(
      (view) => {
        if (!cancelled) setFullView(view);
      },
      () => {
        if (!cancelled) setFullFailed(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [vnId, current]);

  useEffect(() => {
    if (openIndex === null) return undefined;
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "ArrowRight") {
        event.preventDefault();
        step(1);
      } else if (event.key === "ArrowLeft") {
        event.preventDefault();
        step(-1);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [openIndex, step]);

  // A manual entry has no VN, and an entry VNDB knows with no screenshots has nothing
  // to show. Both are stated rather than hidden: an absent section reads as a bug.
  const total = reported > 0 ? reported : shots.length;

  return (
    <>
      <Card>
        <CardHeader
          title={t("detail.screenshots")}
          aside={
            total > 0 ? (
              <span className="caption tnum">
                {t("detail.screenshotsCount", { count: total })}
              </span>
            ) : null
          }
        />

        {/* A gallery-local busy label, not the generic `common.loading`: the page
            shell uses that label for "the whole screen is still arriving", and a
            section that borrows it makes the page look like it never finished. */}
        {state === "loading" ? (
          <div aria-busy="true" aria-label={t("detail.screenshots")}>
            {/* Show loading text and a full-row skeleton so initial fetch latency does not look broken. */}
            <p className="body" style={{ marginBottom: "var(--space-2)" }}>
              {t("detail.screenshotsLoading")}
            </p>
            <div className="gallery">
              {Array.from({ length: 6 }, (_, index) => (
                <div key={index} className="gallery__tile skeleton" />
              ))}
            </div>
          </div>
        ) : null}

        {state === "error" ? (
          <p className="body">{t("detail.screenshotsEmpty")}</p>
        ) : null}

        {state === "ready" && shots.length === 0 ? (
          <p className="body">{t("detail.screenshotsEmpty")}</p>
        ) : null}

        {state === "ready" && shots.length > 0 ? (
          <>
            <ul className="gallery">
              {shots.map((shot, index) => {
                // Apply the shared three-way NSFW policy with reveals keyed by policy value.
                const explicit = shot.explicit;
                const revealed = revealedIds.includes(shot.image_id);
                const concealed = nsfw !== "show" && explicit && !revealed;
                const policy: "visible" | "blur" | "hide" = !concealed
                  ? "visible"
                  : nsfw === "blur" && shot.blurred_src
                    ? "blur"
                    : "hide";
                const painted =
                  policy === "blur" ? shot.blurred_src : policy === "hide" ? null : shot.src;
                return (
                  <li key={shot.image_id} className="gallery__item">
                    <button
                      type="button"
                      className="gallery__tile"
                      aria-label={
                        concealed
                          ? t("detail.screenshotReveal")
                          : t("detail.screenshotOpen", {
                              index: index + 1,
                              total: shots.length,
                            })
                      }
                      onClick={() => {
                        // The first click on a concealed tile reveals it, the second
                        // opens it. Opening straight into a full-size explicit image
                        // is the one thing a "blur until revealed" setting forbids.
                        if (concealed) {
                          addReveal(shot.image_id);
                          return;
                        }
                        openAt(index);
                      }}
                    >
                      {painted ? (
                        <img
                          className={
                            policy === "blur"
                              ? "gallery__img gallery__img--concealed"
                              : "gallery__img"
                          }
                          src={painted}
                          alt={
                            concealed
                              ? ""
                              : t("detail.screenshotAlt", {
                                  title,
                                  index: index + 1,
                                  total: shots.length,
                                })
                          }
                          aria-hidden={concealed ? "true" : undefined}
                          loading="lazy"
                          decoding="async"
                          width={shot.width ?? undefined}
                          height={shot.height ?? undefined}
                        />
                      ) : (
                        /* `hide` leaves the surface empty rather than painting the
                           blur: that is the whole difference between the two modes,
                           and for two phases this control behaved exactly like blur
                           because it always painted the soft file. */
                        <span className="gallery__neutral" aria-hidden="true" />
                      )}
                      {concealed ? (
                        <span className="gallery__scrim">
                          <Icon name="eyeOff" size={16} />
                          <span>{t("detail.screenshotHidden")}</span>
                        </span>
                      ) : null}
                    </button>
                  </li>
                );
              })}
            </ul>
            {/* While batches are still arriving this is progress, not a shortfall:
                reporting "7 of 12 could not be downloaded" mid-download would be a
                lie about work that has not been attempted yet. */}
            {pending > 0 ? (
              <p className="caption" style={{ marginTop: "var(--space-2)" }}>
                {t("detail.screenshotsLoading")} ({shots.length}/{total})
              </p>
            ) : !complete ? (
              <p className="caption" style={{ marginTop: "var(--space-2)" }}>
                {t("detail.screenshotsPartial", {
                  shown: shots.length,
                  total,
                })}
              </p>
            ) : null}
          </>
        ) : null}
      </Card>

      <Sheet
        open={current !== null}
        title={
          // The sheet title is the position, which is the one thing that helps when
          // stepping through a set: "3 of 14".
          current
            ? t("detail.screenshotAlt", {
                title,
                index: (openIndex ?? 0) + 1,
                total: shots.length,
              })
            : title
        }
        onClose={closeLightbox}
        wide
        footer={
          <>
            <Button
              variant="ghost"
              onClick={() => {
                if (!current) return;
                void api.openExternal(current.vndb_url).catch((caught: unknown) => {
                  pushToast(
                    "error",
                    caught instanceof IpcError ? caught.message : t("common.error"),
                  );
                });
              }}
            >
              {t("detail.screenshotOnVndb")}
            </Button>
            <span style={{ flex: 1 }} />
            <Button
              variant="secondary"
              onClick={() => step(-1)}
              disabled={shots.length < 2}
              aria-label={t("detail.screenshotPrevious")}
            >
              <Icon name="chevronLeft" />
            </Button>
            <Button
              variant="secondary"
              onClick={() => step(1)}
              disabled={shots.length < 2}
              aria-label={t("detail.screenshotNext")}
            >
              <span style={{ display: "inline-flex", transform: "rotate(180deg)" }}>
                <Icon name="chevronLeft" />
              </span>
            </Button>
          </>
        }
      >
        {current ? (
          (() => {
            // Gate sharp full-size image loads behind `show` or an active per-tile reveal.
            const revealed = revealedIds.includes(current.image_id);
            const concealed = nsfw !== "show" && current.explicit && !revealed;
            const blurSrc = fullView?.blurred_src ?? current.blurred_src;
            const policy: "visible" | "blur" | "hide" = !concealed
              ? "visible"
              : nsfw === "blur" && blurSrc
                ? "blur"
                : "hide";
            const painted =
              policy === "blur" ? blurSrc : policy === "hide" ? null : (fullView?.src ?? current.src);
            const full = policy === "visible" ? fullView : null;
            return (
              <figure className="lightbox">
                {fullFailed ? (
                  <p className="body">{t("detail.screenshotFailed")}</p>
                ) : painted ? (
                  <img
                    className="lightbox__img"
                    src={painted}
                    alt={
                      concealed
                        ? ""
                        : t("detail.screenshotAlt", {
                            title,
                            index: (openIndex ?? 0) + 1,
                            total: shots.length,
                          })
                    }
                    aria-hidden={concealed ? "true" : undefined}
                    width={full?.width ?? current.width ?? undefined}
                    height={full?.height ?? current.height ?? undefined}
                  />
                ) : null}
                {concealed ? (
                  <figcaption className="caption lightbox__caption">
                    {t("detail.screenshotHidden")}
                  </figcaption>
                ) : null}
              </figure>
            );
          })()
        ) : null}
      </Sheet>
    </>
  );
}
