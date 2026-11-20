import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router";

import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import { ProgressRing } from "@/components/ProgressRing";
import { achievementsList, listenAchievementsUnlocked } from "@/lib/ipc/achievements";
import type { AchievementView } from "@/lib/ipc/types";

/**
 * Compact achievement summary for the Stats overview: unlocked count out of the
 * catalog, the most recent unlock, and a link to the full screen. It reads the
 * same `achievements_list` the screen does, so the two never disagree.
 */
export function AchievementStatsCard() {
  const { t } = useI18n();
  const [views, setViews] = useState<AchievementView[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [reloadTick, setReloadTick] = useState(0);

  useEffect(() => {
    let cancelled = false;
    void achievementsList()
      .then((next) => {
        if (!cancelled) {
          setViews(next);
          setFailed(false);
        }
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [reloadTick]);

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void listenAchievementsUnlocked(() => {
      if (!disposed) setReloadTick((tick) => tick + 1);
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stop = unlisten;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);

  const unlocked = useMemo(
    () =>
      (views ?? [])
        .filter((view) => view.unlocked)
        .sort((a, b) => (b.unlockedAt ?? 0) - (a.unlockedAt ?? 0)),
    [views],
  );
  const total = views?.length ?? 0;
  const latest = unlocked[0];
  const summary = t("achievements.statsProgress", { unlocked: unlocked.length, total });

  const link = (
    <Link className="ach-stats__link" to="/achievements">
      {t("achievements.statsLink")}
    </Link>
  );

  if (failed) {
    return (
      <Card className="ach-stats">
        <CardHeader title={t("achievements.statsTitle")} aside={link} />
        <p className="caption" role="alert">
          {t("achievements.errorTitle")}
        </p>
      </Card>
    );
  }

  if (!views) {
    return (
      <Card className="ach-stats">
        <CardHeader title={t("achievements.statsTitle")} />
        <div className="skeleton skeleton--row" />
      </Card>
    );
  }

  return (
    <Card className="ach-stats">
      <CardHeader title={t("achievements.statsTitle")} aside={link} />
      <div className="ach-stats__body">
        <ProgressRing
          progress={total > 0 ? unlocked.length / total : 0}
          size={40}
          strokeWidth={4}
          title={summary}
        />
        <div className="ach-stats__text">
          <p className="ach-stats__progress tnum">{summary}</p>
          <p className="caption">
            {latest
              ? t("achievements.statsLatest", { name: t(`achievements.names.${latest.slug}`) })
              : t("achievements.statsEmpty")}
          </p>
        </div>
      </div>
    </Card>
  );
}