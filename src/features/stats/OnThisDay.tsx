import { useEffect, useState } from "react";
import { Link } from "react-router";

import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import { Icon } from "@/components/Icon";
import { useArtwork } from "@/components/useArtwork";
import { api } from "@/lib/ipc/client";
import type { MemoryEvent } from "@/lib/ipc/types";

/** One memory row. Cover concealment goes through the shared artwork hook. */
function MemoryRow({ memory }: { memory: MemoryEvent }) {
  const { t } = useI18n();
  const art = useArtwork(memory.game);
  const { policy, imageUrl, fallback } = art;
  const concealed = policy !== "visible";
  const position = [memory.routeName, memory.chapterName].filter(Boolean).join(" · ");

  return (
    <li>
      <Link to={`/game/${memory.game.meta.id}`} className="memory__row">
        <div
          className="memory__thumb"
          style={
            imageUrl
              ? undefined
              : {
                  background: `linear-gradient(160deg, ${fallback.accent}44, var(--bg-elevated))`,
                }
          }
        >
          {imageUrl ? (
            <img
              className={concealed ? "cover__art--blurred" : undefined}
              src={imageUrl}
              alt=""
              width={44}
              height={66}
              loading="lazy"
              decoding="async"
              onError={concealed ? undefined : art.imageFailed}
              style={{ width: "100%", height: "100%", objectFit: "cover" }}
            />
          ) : concealed ? (
            <span className="memory__concealed" aria-hidden="true">
              <Icon name="eyeOff" size={18} />
            </span>
          ) : null}
        </div>
        <div className="memory__body">
          <span className="memory__when caption">
            {t("stats.onThisDayYears", { count: memory.yearsAgo })}
          </span>
          <span className="memory__title">{memory.game.title}</span>
          <span className="memory__event">
            {memory.kind === "ending"
              ? t("stats.onThisDayEnding", { name: memory.endingName ?? "" })
              : memory.note?.trim()
                ? memory.note
                : t("stats.onThisDayNote")}
          </span>
          {position ? <span className="memory__position caption">{position}</span> : null}
        </div>
      </Link>
    </li>
  );
}

/**
 * Past-year checkpoints and endings matching today's local date (excluding today).
 * Renders nothing when empty so no "nothing happened today" placeholder appears.
 */
export function OnThisDay() {
  const { t } = useI18n();
  const [memories, setMemories] = useState<MemoryEvent[] | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void api.onThisDay().then(
      (rows) => {
        if (!cancelled) setMemories(rows);
      },
      () => {
        if (!cancelled) setFailed(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  if (failed || !memories || memories.length === 0) return null;

  return (
    <Card className="memory-card">
      <CardHeader
        title={t("stats.onThisDay")}
        aside={
          <span className="caption tnum">
            {t("stats.onThisDayCount", { count: memories.length })}
          </span>
        }
      />
      <ul className="memory__list">
        {memories.map((memory) => (
          <MemoryRow key={`${memory.kind}:${memory.id}`} memory={memory} />
        ))}
      </ul>
    </Card>
  );
}
