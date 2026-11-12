import { useEffect, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import { Icon } from "@/components/Icon";
import { api } from "@/lib/ipc/client";
import type { ScreenshotCard } from "@/lib/ipc/types";
import { ScreenshotButton } from "@/features/game/ScreenshotButton";
import { ScreenshotLightbox } from "@/features/screenshots/ScreenshotLightbox";

/**
 * The reader's own captures for one game.
 *
 * Separate from `ScreenshotGallery`, which shows VNDB's reference images for a
 * linked VN. These are the reader's pictures: the only copy, stored locally, and
 * nothing rebuilds them if they are lost.
 *
 * Thumbnails are fetched one at a time as data URLs and cached for the session,
 * because the webview has no filesystem access and a full-size JPEG per tile
 * would be several MB for a strip that renders at 160 px.
 */
export function ScreenshotLibrary({ gameId }: { gameId: string }) {
  const { t, locale } = useI18n();
  // The list is stored with the game it was fetched for, so switching games
  // shows an empty section until the new answer arrives instead of the previous
  // game's pictures. Nothing is reset from an effect body.
  const [listed, setListed] = useState<{
    gameId: string;
    shots: ScreenshotCard[];
  } | null>(null);
  const [open, setOpen] = useState<ScreenshotCard | null>(null);

  const shots = listed?.gameId === gameId ? listed.shots : [];
  const loaded = listed?.gameId === gameId;

  // Bumped to re-read the list after a capture or a delete. A token rather than
  // a callback so the fetch stays inside the effect, which is the shape the
  // project uses everywhere: nothing is written to state from an effect body.
  const [token, setToken] = useState(0);

  useEffect(() => {
    let active = true;
    void api
      .screenshotList(gameId)
      .then((next) => {
        if (active) setListed({ gameId, shots: next });
      })
      .catch(() => {
        // A missing list is not worth an error banner: the section simply stays
        // empty, and the capture button still works.
        if (active) setListed({ gameId, shots: [] });
      });
    return () => {
      active = false;
    };
  }, [gameId, token]);

  // Nothing captured and no way to capture here: an empty card explaining a
  // feature the reader cannot use is noise.
  if (loaded && shots.length === 0) {
    return (
      <Card>
        <CardHeader
          title={
            <span className="shots__heading">
              <Icon name="camera" size={18} />
              {t("screenshot.title")}
            </span>
          }
        />
        <p className="body">{t("screenshot.empty")}</p>
        <ScreenshotButton
          gameId={gameId}
          onCaptured={() => setToken((current) => current + 1)}
        />
      </Card>
    );
  }

  return (
    <>
      <Card>
        <CardHeader
          title={
            <span className="shots__heading">
              <Icon name="camera" size={18} />
              {t("screenshot.title")}
            </span>
          }
          aside={
            <span className="caption">
              {t("screenshot.count", { count: shots.length })}
            </span>
          }
        />
        <ScreenshotButton
          gameId={gameId}
          onCaptured={() => setToken((current) => current + 1)}
        />
        <ul className="shots">
          {shots.map((shot) => (
            <li key={shot.id}>
              <button
                type="button"
                className="shots__tile"
                onClick={() => setOpen(shot)}
                aria-label={t("screenshot.open", {
                  date: new Date(shot.capturedAt).toLocaleDateString(locale),
                })}
              >
                <Thumbnail id={shot.id} alt="" />
              </button>
            </li>
          ))}
        </ul>
      </Card>

      <ScreenshotLightbox
        shot={open}
        onClose={() => setOpen(null)}
        onDeleted={() => setToken((current) => current + 1)}
      />
    </>
  );
}

/**
 * One tile's image.
 *
 * Its own component so the fetch lives with the tile that needs it: a tile that
 * scrolls out and back re-mounts, and the browser's own cache answers the second
 * request without another IPC round trip.
 */
function Thumbnail({ id, alt }: { id: string; alt: string }) {
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void api
      .screenshotImage(id)
      .then((next) => {
        if (active) setSrc(next);
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, [id]);

  if (!src) return <span className="shots__placeholder" aria-hidden="true" />;
  return <img className="shots__img" src={src} alt={alt} loading="lazy" />;
}
