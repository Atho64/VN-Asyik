import { useEffect, useState } from "react";
import { Link } from "react-router";

import { useI18n } from "@/app/useI18n";
import { Card } from "@/components/Card";
import { api } from "@/lib/ipc/client";
import type { ScreenshotCard } from "@/lib/ipc/types";
import { ScreenshotLightbox } from "@/features/screenshots/ScreenshotLightbox";

/** A capture with the game it belongs to. */
interface FeedShot extends ScreenshotCard {
  gameTitle: string;
}

/**
 * Game-grouped feed of the reader's local captures (16:9 uncropped), distinct
 * from the disposable VNDB reference gallery. Lives inside the Timeline screen tabs.
 */
export function ScreenshotsPanel() {
  const { t, locale } = useI18n();
  const [shots, setShots] = useState<FeedShot[] | null>(null);
  const [open, setOpen] = useState<FeedShot | null>(null);
  const [token, setToken] = useState(0);

  useEffect(() => {
    let active = true;
    void api
      .screenshotListAll()
      .then((next) => {
        if (active) setShots(next);
      })
      .catch(() => {
        if (active) setShots([]);
      });
    return () => {
      active = false;
    };
  }, [token]);

  if (shots === null) {
    return <p className="caption">{t("common.loading")}</p>;
  }

  if (shots.length === 0) {
    return (
      <Card>
        <p className="body">{t("screenshot.empty")}</p>
      </Card>
    );
  }

  // Newest game first: the order the feed arrives in, collapsed by game.
  const groups = new Map<string, { title: string; shots: FeedShot[] }>();
  for (const shot of shots) {
    const group = groups.get(shot.gameId);
    if (group) group.shots.push(shot);
    else groups.set(shot.gameId, { title: shot.gameTitle, shots: [shot] });
  }

  return (
    <>
      <p className="caption">
        {t("screenshot.libraryCount", { count: shots.length })}
      </p>

      {[...groups.entries()].map(([gameId, group]) => (
        <Card key={gameId}>
          <header className="card__header">
            <h2 className="h3">
              <Link className="shots__game" to={`/game/${gameId}`}>
                {group.title}
              </Link>
            </h2>
            <span className="caption">
              {t("screenshot.count", { count: group.shots.length })}
            </span>
          </header>
          <ul className="shots">
            {group.shots.map((shot) => (
              <li key={shot.id}>
                <button
                  type="button"
                  className="shots__tile"
                  onClick={() => setOpen(shot)}
                  aria-label={t("screenshot.open", {
                    date: new Date(shot.capturedAt).toLocaleDateString(locale),
                  })}
                >
                  <Thumbnail id={shot.id} />
                </button>
              </li>
            ))}
          </ul>
        </Card>
      ))}

      <ScreenshotLightbox
        shot={open}
        onClose={() => setOpen(null)}
        onDeleted={() => setToken((current) => current + 1)}
      />
    </>
  );
}

/** One tile's image, fetched on mount and cached by the browser afterwards. */
function Thumbnail({ id }: { id: string }) {
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
  return <img className="shots__img" src={src} alt="" loading="lazy" />;
}
