import { useCallback, useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { Sheet } from "@/components/Sheet";
import { api, IpcError } from "@/lib/ipc/client";
import type { ScreenshotView } from "@/lib/ipc/types";

/**
 * How long the gallery waits before giving up and saying so.
 *
 * Generous rather than tight: the first visit downloads the whole gallery
 * sequentially at VNDB's rate limit, which for 30 screenshots is around 15-20
 * seconds. The point is not to be fast, it is to fail visibly instead of spinning.
 */
const GALLERY_TIMEOUT_MS = 120_000;

/**
 * How many download rounds to ask for before giving up on the rest.
 *
 * A 30-image gallery is five rounds of six. The cap is a guard against a backend that
 * keeps reporting work outstanding, not a real limit.
 */
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
 * The VN's screenshots, read from the local cache.
 *
 * Two decisions worth stating, because both are visible:
 *
 * * **Nothing downloads until this component mounts.** The gallery lives under the
 *   description, and a VN with thirty screenshots is a few megabytes. A screen that
 *   fetched them on open would delay the text the user came to read.
 * * **The content policy is applied by the backend, per image.** The `src` handed to
 *   a tile is already the pre-blurred file for an explicit screenshot, so the sharp
 *   bytes never reach the webview and no CSS class can accidentally reveal them.
 *   `explicit` is carried alongside so the tile can label itself and offer a reveal.
 */
export function ScreenshotGallery({ vnId, title, version = 0 }: ScreenshotGalleryProps) {
  const { t } = useI18n();
  const nsfw = useAppStore((state) => state.prefs.nsfw);
  const pushToast = useAppStore((state) => state.pushToast);

  // The gallery is keyed by everything that can change what it should show. A load
  // result is stored together with the key it was fetched for, so a stale response
  // arriving after the VN or the content setting changed is ignored instead of
  // painted over the newer state. Nothing is written to state from an effect body.
  const galleryKey = `${vnId ?? ""}:${version}`;
  const [loaded, setLoaded] = useState<{
    key: string;
    status: "ready" | "error";
    shots: ScreenshotView[];
    reported: number;
    complete: boolean;
    pending: number;
  } | null>(null);
  // A reveal is scoped to the policy value as well as to the gallery, so switching the
  // setting back to Blur re-conceals exactly like the cover reveal does. The payload is
  // policy-free, so the policy no longer belongs in the fetch key — only here.
  const revealScope = `${galleryKey}:${nsfw}`;
  const [revealed, setRevealed] = useState<{ key: string; ids: string[] }>({
    key: revealScope,
    ids: [],
  });
  const [openIndex, setOpenIndex] = useState<number | null>(null);
  // The full-size version of the current lightbox image, fetched lazily.
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

    // The first visit to a VN downloads up to 30 screenshots, so this call is far
    // slower than any other on the page and cannot be given the same budget. It is
    // still bounded: a request that never settles used to leave the placeholders
    // spinning forever, which reads as a broken screen rather than a slow one. On
    // timeout the section says it could not load and offers nothing it cannot back
    // up — the tiles already cached will paint on the next visit.
    const timeout = window.setTimeout(() => {
      settle({
        status: "error",
        shots: [],
        reported: 0,
        complete: true,
        pending: 0,
      });
    }, GALLERY_TIMEOUT_MS);

    // The backend downloads in batches so a 30-image gallery is not one minute of
    // blank screen. Keep asking while it reports work outstanding: each round paints
    // the new tiles, so the grid fills in visibly.
    const loadRound = async (): Promise<void> => {
      // Bounded so a backend that never settles cannot loop forever.
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

  // A reveal is per screenshot and is revoked when the policy changes, the same way
  // the cover reveal is. Keying a single object rather than resetting a map in an
  // effect means the revocation happens on the same render as the policy change, with
  // no intermediate frame in which the old reveals are still true.
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
            {/* Say what the wait is for. Three anonymous placeholders on a first
                visit that legitimately takes ~20 s read as a broken gallery; naming
                the download and promising it happens once is the honest version.
                The skeleton is also a full row rather than three tiles, because the
                count is unknown until the response arrives. */}
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
                // The same three-way policy a cover is drawn under, from the same
                // preference and the same rule: `show` paints the sharp file, `blur`
                // paints its own pre-blurred copy, and `hide` paints nothing and shows
                // the neutral surface. Revealing is per tile and expires with the
                // policy, exactly like `useArtwork`'s reveal.
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
            // The lightbox obeys the same policy as the tile that opened it, and reaches
            // for the sharp full-size file only once the image is genuinely in the
            // clear. A tile opened after its reveal paints sharp; one that somehow got
            // here under `blur` must not be promoted merely because the view is larger.
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
