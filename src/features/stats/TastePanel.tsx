import { useEffect, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { EmptyState } from "@/components/EmptyState";
import { api, IpcError } from "@/lib/ipc/client";
import type { ForYouSignal, TasteView } from "@/lib/ipc/types";

/**
 * Locally computed reading taste profile matching the For-you seed weights
 * (falls back to raw tag counts when no games are rated yet).
 */
export function TastePanel() {
  const { t } = useI18n();
  const [taste, setTaste] = useState<TasteView | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void api
      .aiTasteProfile()
      .then((next) => {
        if (!cancelled) setTaste(next);
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setError(caught instanceof IpcError ? caught.message : t("common.error"));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [t]);

  if (error) {
    return (
      <Card>
        <CardHeader title={t("taste.title")} />
        <p className="field__error" role="alert">
          {error}
        </p>
      </Card>
    );
  }

  if (!taste) {
    return (
      <Card>
        <CardHeader title={t("taste.title")} />
        <div className="skeleton skeleton--row" />
      </Card>
    );
  }

  const { profile, seeds } = taste;

  // Weighted seeds when ratings exist; the plain count otherwise. Either way
  // the section renders one list shape.
  const themeSignals: ForYouSignal[] =
    seeds.tags.length > 0
      ? seeds.tags
      : profile.themes.map((theme) => ({
          id: theme.name,
          name: theme.name,
          count: theme.count,
          examples: theme.examples,
        }));

  const creatorSeeds =
    seeds.developers.length + seeds.staff.length + seeds.voices.length;
  // Only claim missing data when the library is big enough to have any: a
  // two-game library legitimately has no recurring creator yet.
  const creatorsMissing =
    seeds.ready && seeds.considered > 0 && creatorSeeds === 0;

  const hasAnything =
    themeSignals.length > 0 ||
    seeds.topGames.length > 0 ||
    creatorSeeds > 0;

  return (
    <Card>
      <CardHeader title={t("taste.title")} />
      <p className="card__intro">{t("taste.intro")}</p>

      {seeds.considered > 0 && (
        <p className="caption">{t("taste.basedOn", { count: seeds.considered })}</p>
      )}

      {!hasAnything ? (
        <EmptyState title={t("taste.noThemes")} body={t("taste.noThemesBody")} />
      ) : (
        <>
          {seeds.topGames.length > 0 && (
            <div className="taste-section">
              <h3 className="taste-section__title">{t("taste.anchors")}</h3>
              <ul className="taste-tags">
                {seeds.topGames.map((title) => (
                  <li key={title}>
                    <Chip>{title}</Chip>
                  </li>
                ))}
              </ul>
            </div>
          )}

          {themeSignals.length > 0 && (
            <TasteSection title={t("taste.themes")} signals={themeSignals} />
          )}

          {seeds.developers.length > 0 && (
            <TasteSection title={t("taste.studios")} signals={seeds.developers} />
          )}
          {seeds.staff.length > 0 && (
            <TasteSection title={t("taste.staff")} signals={seeds.staff} />
          )}
          {seeds.voices.length > 0 && (
            <TasteSection title={t("taste.voices")} signals={seeds.voices} />
          )}

          {creatorsMissing && (
            <p
              className="notice"
              role="status"
              style={{ marginTop: "var(--space-3)" }}
            >
              {t("taste.hintRefresh")}
            </p>
          )}
        </>
      )}

      {/* Games with no tags are named rather than silently omitted: a profile of
          four out of ten games must not read as a profile of the library. */}
      {profile.undetermined.length > 0 ? (
        <p className="notice" role="status" style={{ marginTop: "var(--space-3)" }}>
          {t("taste.undetermined")
            .replace("{count}", String(profile.undetermined.length))
            .replace("{titles}", profile.undetermined.slice(0, 5).join(", "))}
        </p>
      ) : null}
    </Card>
  );
}

interface SectionProps {
  title: string;
  signals: ForYouSignal[];
}

/** One seed category: its heading and the chips carrying the evidence. */
function TasteSection({ title, signals }: SectionProps) {
  return (
    <div className="taste-section">
      <h3 className="taste-section__title">{title}</h3>
      <ul className="taste-tags">
        {signals.map((signal) => (
          <li key={signal.id}>
            {/* The games behind the count, so it can be checked. */}
            <Chip>
              <span title={signal.examples.join(", ")}>
                {signal.name} · {signal.count}
              </span>
            </Chip>
          </li>
        ))}
      </ul>
    </div>
  );
}
