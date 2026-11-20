import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router";

import { useAppStore, initialAchievementsBrowse } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { EmptyState } from "@/components/EmptyState";
import { Icon } from "@/components/Icon";
import { ProgressRing } from "@/components/ProgressRing";
import { IpcError } from "@/lib/ipc/client";
import { achievementsList, listenAchievementsUnlocked } from "@/lib/ipc/achievements";
import type { AchievementView } from "@/lib/ipc/types";
import { AchievementCard } from "./AchievementCard";

const STATUS_FILTERS = [
  ["all", "achievements.filterAll"],
  ["unlocked", "achievements.filterUnlocked"],
  ["locked", "achievements.filterLocked"],
] as const;

const TIER_FILTERS = [
  ["all", "achievements.tierAll"],
  ["bronze", "achievements.tier.bronze"],
  ["silver", "achievements.tier.silver"],
  ["gold", "achievements.tier.gold"],
] as const;

/**
 * The achievement catalog: one overall progress ring as the focal point, filter
 * chips and search, then a responsive grid of cards. Filters and search live in
 * the store so they survive navigation. Empty, loading, error and no-match states
 * are distinct screens, each naming its cause and the next action.
 */
export function AchievementsScreen() {
  const { t } = useI18n();
  const browse = useAppStore((state) => state.achievementsBrowse);
  const updateBrowse = useAppStore((state) => state.updateAchievementsBrowse);
  const [views, setViews] = useState<AchievementView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloadTick, setReloadTick] = useState(0);

  useEffect(() => {
    let cancelled = false;
    void achievementsList()
      .then((next) => {
        if (!cancelled) {
          setViews(next);
          setError(null);
        }
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setError(caught instanceof IpcError ? caught.message : t("common.error"));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [reloadTick, t]);

  // New unlocks land while the screen is open (a mutation elsewhere evaluated);
  // re-fetch so the grid and ring reflect them without a remount.
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

  const unlockedCount = useMemo(
    () => (views ? views.filter((view) => view.unlocked).length : 0),
    [views],
  );
  const total = views?.length ?? 0;
  const totalProgress = useMemo(
    () => (views ? views.reduce((sum, view) => sum + view.progress, 0) : 0),
    [views],
  );

  const filtered = useMemo(() => {
    if (!views) return [];
    const query = browse.query.trim().toLowerCase();
    const list = views.filter((view) => {
      if (browse.status === "unlocked" && !view.unlocked) return false;
      if (browse.status === "locked" && view.unlocked) return false;
      if (browse.tier !== "all" && view.tier !== browse.tier) return false;
      if (query) {
        const name = t(`achievements.names.${view.slug}`).toLowerCase();
        const desc = t(`achievements.desc.${view.slug}`).toLowerCase();
        if (!name.includes(query) && !desc.includes(query)) return false;
      }
      return true;
    });
    // Hierarchy: unlocked first (most recent first), then locked nearest to
    // completion, so the card a reader can act on next rises to the top.
    return [...list].sort((a, b) => {
      if (a.unlocked !== b.unlocked) return a.unlocked ? -1 : 1;
      if (a.unlocked && b.unlocked) return (b.unlockedAt ?? 0) - (a.unlockedAt ?? 0);
      const ratioA = a.target > 0 ? a.progress / a.target : 0;
      const ratioB = b.target > 0 ? b.progress / b.target : 0;
      return ratioB - ratioA;
    });
  }, [views, browse, t]);

  const retry = (): void => {
    setError(null);
    setReloadTick((tick) => tick + 1);
  };

  const ringRatio = total > 0 ? unlockedCount / total : 0;
  const summary = t("achievements.headerProgress", { unlocked: unlockedCount, total });

  return (
    <div className="ach-page">
      <header className="ach-head">
        <div className="ach-head__intro">
          <h1 className="page-title">{t("achievements.title")}</h1>
          <p className="ach-head__sub">{t("achievements.subtitle")}</p>
        </div>
        {views && !error ? (
          <div className="ach-head__summary">
            <ProgressRing progress={ringRatio} size={64} strokeWidth={5} title={summary} />
            <div className="ach-head__figure">
              <span className="ach-head__number tnum">
                {unlockedCount}
                <span className="ach-head__total">/{total}</span>
              </span>
              <span className="ach-head__caption">{t("achievements.unlocked")}</span>
            </div>
          </div>
        ) : null}
      </header>

      {error ? (
        <div className="ach-state" role="alert">
          <h2 className="ach-state__title">{t("achievements.errorTitle")}</h2>
          <p className="ach-state__body">{error}</p>
          <Button variant="secondary" onClick={retry}>
            <Icon name="sync" size={14} />
            {t("achievements.retry")}
          </Button>
        </div>
      ) : !views ? (
        <div className="ach-grid ach-grid--loading" aria-busy="true" aria-label={t("achievements.loading")}>
          {Array.from({ length: 8 }, (_, index) => (
            <div key={index} className="ach-card-skeleton skeleton" />
          ))}
        </div>
      ) : total === 0 || (unlockedCount === 0 && totalProgress === 0) ? (
        <EmptyState
          title={t("achievements.emptyTitle")}
          body={t("achievements.emptyBody")}
          action={
            <Link className="btn btn--secondary" to="/">
              {t("achievements.emptyAction")}
            </Link>
          }
        />
      ) : (
        <>
          <div className="ach-controls">
            <div className="ach-filters" role="group" aria-label={t("achievements.filterStatus")}>
              {STATUS_FILTERS.map(([value, key]) => (
                <Chip
                  key={value}
                  selected={browse.status === value}
                  onClick={() => updateBrowse({ status: value })}
                >
                  {t(key)}
                </Chip>
              ))}
            </div>
            <div className="ach-filters" role="group" aria-label={t("achievements.filterTier")}>
              {TIER_FILTERS.map(([value, key]) => (
                <Chip
                  key={value}
                  selected={browse.tier === value}
                  onClick={() => updateBrowse({ tier: value })}
                >
                  {t(key)}
                </Chip>
              ))}
            </div>
            <div className="search ach-search">
              <span className="search__icon">
                <Icon name="search" />
              </span>
              <label className="sr-only" htmlFor="ach-search">
                {t("achievements.searchLabel")}
              </label>
              <input
                id="ach-search"
                className="input"
                type="search"
                value={browse.query}
                onChange={(event) => updateBrowse({ query: event.target.value })}
                placeholder={t("achievements.searchPlaceholder")}
                autoComplete="off"
              />
            </div>
          </div>

          {filtered.length === 0 ? (
            <EmptyState
              title={t("achievements.resultsEmptyTitle")}
              body={t("achievements.resultsEmptyBody")}
              action={
                <Button variant="secondary" onClick={() => updateBrowse(initialAchievementsBrowse)}>
                  {t("achievements.clearFilters")}
                </Button>
              }
            />
          ) : (
            <>
              <p className="ach-count caption">{t("achievements.count", { count: filtered.length })}</p>
              <ul className="ach-grid">
                {filtered.map((view) => (
                  <AchievementCard key={view.slug} view={view} />
                ))}
              </ul>
            </>
          )}
        </>
      )}
    </div>
  );
}