import { useI18n } from "@/app/useI18n";
import { Icon } from "@/components/Icon";
import type { AchievementView } from "@/lib/ipc/types";
import { achievementIcon } from "./icons";
import { achievementUnit, formatProgressValue } from "./progress";

/** Format an unlock timestamp as a local calendar date in the active locale. */
function unlockDate(ms: number, locale: string): string {
  return new Date(ms).toLocaleDateString(locale === "id" ? "id-ID" : "en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

/**
 * One catalog card. Unlocked cards carry a subtle accent material and the unlock
 * date; locked cards stay muted (still AA) and show a progress bar toward the
 * condition. Tier reads from its label plus a neutral surface step, never from
 * hue on the glyph, so one asset works across tiers and both themes.
 */
export function AchievementCard({ view }: { view: AchievementView }) {
  const { t, locale } = useI18n();
  const name = t(`achievements.names.${view.slug}`);
  const desc = t(`achievements.desc.${view.slug}`);
  const tierLabel = t(`achievements.tier.${view.tier}`);
  const groupName = t(`achievements.group.${view.group}`);
  const icon = achievementIcon(view.slug);
  const ratio = view.target > 0 ? Math.min(1, view.progress / view.target) : 0;
  // Time-based slugs carry milliseconds and streaks carry days; show them in
  // human units while the bar ratio keeps the raw numbers.
  const progressText =
    achievementUnit(view.slug) === "days"
      ? t("achievements.progressDays", { progress: view.progress, target: view.target })
      : t("achievements.progress", {
          progress: formatProgressValue(view.slug, view.progress, locale),
          target: formatProgressValue(view.slug, view.target, locale),
        });

  const stateLabel = view.unlocked ? t("achievements.unlocked") : t("achievements.locked");
  const ariaLabel = view.unlocked
    ? `${name}. ${tierLabel}. ${stateLabel}. ${groupName}`
    : `${name}. ${tierLabel}. ${stateLabel}, ${progressText}. ${groupName}`;

  return (
    <li className="ach-grid__item">
      <article
        className="ach-card"
        data-unlocked={view.unlocked ? "true" : "false"}
        data-tier={view.tier}
        tabIndex={0}
        aria-label={ariaLabel}
      >
        <span
          className="ach-card__icon"
          aria-hidden="true"
          {...(icon ? { dangerouslySetInnerHTML: { __html: icon } } : {})}
        />
        <div className="ach-card__body">
          <h3 className="ach-card__name">{name}</h3>
          <p className="ach-card__desc">{desc}</p>
          <div className="ach-card__foot">
            <span className="ach-card__tier" data-tier={view.tier}>
              {tierLabel}
            </span>
            {view.group === "E" ? (
              <span className="ach-card__platform">{t("achievements.platformBadge")}</span>
            ) : null}
            {view.unlocked ? (
              <span className="ach-card__date">
                <Icon name="check" size={13} />
                {view.unlockedAt !== null
                  ? t("achievements.unlockedAt", { date: unlockDate(view.unlockedAt, locale) })
                  : t("achievements.unlocked")}
              </span>
            ) : (
              <span className="ach-card__progress tnum">{progressText}</span>
            )}
          </div>
          {!view.unlocked ? (
            <span
              className="ach-card__bar"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={view.target}
              aria-valuenow={view.progress}
              aria-label={progressText}
            >
              <span
                className="ach-card__bar-fill"
                style={{ width: `${Math.round(ratio * 100)}%` }}
              />
            </span>
          ) : null}
        </div>
      </article>
    </li>
  );
}