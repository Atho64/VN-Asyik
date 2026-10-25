import { useCallback, useEffect, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import { api, IpcError } from "@/lib/ipc/client";
import type { StatsPayload } from "@/lib/ipc/types";
import { formatRating } from "@/lib/library";
import { formatDurationLong } from "@/lib/time";

const PERIODS = ["daily", "weekly", "monthly", "yearly"] as const;
type PeriodKey = (typeof PERIODS)[number];

/** The start of a bucket, as a short local date. */
function spanLabel(startMs: number | undefined, locale: string): string {
  if (!startMs) return "";
  return new Date(startMs).toLocaleDateString(locale === "id" ? "id-ID" : "en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

/**
 * Time played over time, plus the derived figures.
 *
 * The chart and the numbers come from one payload so they cannot disagree. The
 * period is a switch rather than four charts: the question is "how has my
 * reading gone lately", and one chart answering it beats four competing ones.
 */
export function TimeStats() {
  const { t, locale } = useI18n();
  const [period, setPeriod] = useState<PeriodKey>("weekly");
  const [payload, setPayload] = useState<StatsPayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async (next: PeriodKey) => {
    setBusy(true);
    setError(null);
    try {
      setPayload(await api.statsDerived(next));
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : String(caught));
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const next = await api.statsDerived("weekly");
        if (active) setPayload(next);
      } catch {
        // The screen has its own error state; a failure here must not throw.
      }
    })();
    return () => {
      active = false;
    };
  }, []);

  const summary = payload?.summary;
  const series = payload?.series ?? [];
  const peak = series.reduce((max, bucket) => Math.max(max, bucket.playtimeMs), 0);
  const hasTime = series.some((bucket) => bucket.playtimeMs > 0);

  return (
    <>
      <Card>
        <CardHeader
          title={t("time.title")}
          aside={
            <div className="time__tabs" role="tablist" aria-label={t("time.period")}>
              {PERIODS.map((key) => (
                <button
                  key={key}
                  type="button"
                  role="tab"
                  aria-selected={period === key}
                  className="time__tab"
                  data-active={period === key ? "true" : "false"}
                  onClick={() => {
                    setPeriod(key);
                    void load(key);
                  }}
                >
                  {t(`time.${key}`)}
                </button>
              ))}
            </div>
          }
        />
        {error && <p className="notice notice--danger">{error}</p>}
        {busy && !payload ? (
          <p className="caption">{t("common.loading")}</p>
        ) : hasTime ? (
          <div className="time__chart" aria-hidden="true">
            {series.map((bucket) => {
              // Height is relative to the busiest bucket in this window, so a
              // quiet week is still legible instead of being one flat pixel.
              const ratio = peak > 0 ? bucket.playtimeMs / peak : 0;
              return (
                <span
                  key={bucket.startMs}
                  className="time__bar"
                  data-empty={bucket.playtimeMs === 0 ? "true" : "false"}
                  style={{ height: `${Math.max(2, Math.round(ratio * 100))}%` }}
                  title={
                    bucket.playtimeMs > 0
                      ? formatDurationLong(bucket.playtimeMs, locale)
                      : undefined
                  }
                />
              );
            })}
          </div>
        ) : (
          <p className="body">{t("time.empty")}</p>
        )}
        {hasTime ? (
          <div className="time__footer">
            {/* Without the span, a bar chart of 30 unlabelled columns says
              nothing: the reader cannot tell whether the tall one is a day, a
              week or a year, or where in the window it sits. */}
            <span className="caption time__span">
              {spanLabel(series[0]?.startMs, locale)} –{" "}
              {spanLabel(series[series.length - 1]?.startMs, locale)}
            </span>
            <span className="caption">
              {t("time.peak", { duration: formatDurationLong(peak, locale) })}
            </span>
          </div>
        ) : null}
      </Card>

      {summary ? (
        <Card>
          <CardHeader title={t("time.figures")} />
          <dl className="figures">
            <Figure
              label={t("time.completion")}
              value={
                summary.completionRate === null
                  ? t("time.notRecorded")
                  : `${Math.round(summary.completionRate * 100)}%`
              }
            />
            <Figure
              label={t("time.averageRating")}
              value={
                summary.averageRating === null
                  ? t("time.notRecorded")
                  : formatRating(Math.round(summary.averageRating))
              }
            />
            <Figure
              label={t("time.longestSession")}
              value={
                summary.longestSessionMs === null
                  ? t("time.notRecorded")
                  : formatDurationLong(summary.longestSessionMs, locale)
              }
            />
            <Figure
              label={t("time.averageSession")}
              value={
                summary.averageSessionMs === null
                  ? t("time.notRecorded")
                  : formatDurationLong(summary.averageSessionMs, locale)
              }
            />
            <Figure label={t("time.sessions")} value={String(summary.sessionCount)} />
            {/* Named "longest route", not "favourite": time played says which
                route took longest, not which one the user liked. */}
            <Figure
              label={t("time.longestRoute")}
              value={summary.favouriteRoute?.name ?? t("time.notRecorded")}
              hint={
                summary.favouriteRoute
                  ? formatDurationLong(summary.favouriteRoute.playtimeMs, locale)
                  : undefined
              }
            />
            <Figure
              label={t("time.finishedThisYear")}
              value={
                summary.finishedThisYear === null
                  ? t("time.notRecorded")
                  : String(summary.finishedThisYear)
              }
            />
            <Figure
              label={t("time.streak")}
              value={
                summary.longestStreakDays === null
                  ? t("time.notRecorded")
                  : t("time.days", { count: summary.longestStreakDays })
              }
            />
          </dl>

          {/* Ratings as a small distribution: one row per score actually used. */}
          {summary.ratingSpread.some((count) => count > 0) ? (
            <div className="spread">
              {summary.ratingSpread.map((count, index) =>
                count > 0 ? (
                  <span key={index} className="spread__cell">
                    <span className="spread__label tnum">{index + 1}</span>
                    <span
                      className="spread__bar"
                      style={{
                        width: `${Math.round(
                          (count / Math.max(...summary.ratingSpread)) * 100,
                        )}%`,
                      }}
                    />
                    <span className="tnum caption">{count}</span>
                  </span>
                ) : null,
              )}
            </div>
          ) : null}
        </Card>
      ) : null}
    </>
  );
}

function Figure({
  label,
  value,
  hint,
}: {
  label: string;
  value: string;
  hint?: string | undefined;
}) {
  return (
    <div className="figures__item">
      <dt className="figures__label">{label}</dt>
      <dd className="figures__value tnum">{value}</dd>
      {hint && <dd className="figures__hint">{hint}</dd>}
    </div>
  );
}
