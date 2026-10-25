import { useEffect, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { EmptyState } from "@/components/EmptyState";
import { api, IpcError } from "@/lib/ipc/client";
import type { TasteView } from "@/lib/ipc/types";

/**
 * What the library says about its owner's taste.
 *
 * A plain count of the VNDB tags the games carry most — no model, no network,
 * nothing sent anywhere. Every tag names the games behind it on hover, so a
 * claim can always be checked against the shelf.
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

  const { profile } = taste;

  return (
    <Card>
      <CardHeader title={t("taste.title")} />
      <p className="card__intro">{t("taste.intro")}</p>

      {profile.themes.length === 0 ? (
        <EmptyState
          title={t("taste.noThemes")}
          body={t("taste.noThemesBody")}
        />
      ) : (
        <ul className="taste-tags">
          {profile.themes.map((theme) => (
            <li key={theme.name}>
              {/* The games behind the count, so it can be checked. */}
              <Chip>
                <span title={theme.examples.join(", ")}>
                  {theme.name} · {theme.count}
                </span>
              </Chip>
            </li>
          ))}
        </ul>
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
