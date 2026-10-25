import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Icon } from "@/components/Icon";
import { api, IpcError } from "@/lib/ipc/client";
import { formatRating } from "@/lib/library";
import { formatDurationLong } from "@/lib/time";
import type { DetailPayload } from "@/lib/ipc/types";
import {
  developerLabel,
  developmentStatus,
  formatReleaseDate,
  languageName,
  platformName,
  rankDevelopers,
  splitExtLinks,
  vndbVnUrl,
} from "@/features/game/vndbMetadata";

interface VndbFactsProps {
  payload: DetailPayload;
  /** The library row's own VN id, which is what the link must point at. */
  vnId: string;
  /** Local playtime, shown next to VNDB's estimate for the same reason both ratings are. */
  playtimeMs: number;
}

/**
 * Everything VNDB knows that is not prose: when it came out, who made it, what it
 * runs on and where to read more.
 *
 * The card is deliberately assembled from what the payload *has* rather than from a
 * fixed row count. A VN with no developer and no release date renders three honest
 * rows, not five rows with two blanks: an empty label looks like a loading bug.
 *
 * The link back to vndb.org is here rather than in the header because this is the
 * block that reads as "the reference entry", and it is the row a reader wants after
 * deciding they need the full page.
 */
export function VndbFacts({ payload, vnId, playtimeMs }: VndbFactsProps) {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);

  const openVndb = (url: string): void => {
    void api.openExternal(url).catch((caught: unknown) => {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    });
  };

  const released = formatReleaseDate(payload.released, t, (year, month) =>
    new Intl.DateTimeFormat(locale, { year: "numeric", month: "long" }).format(
      // Day 1 is a real date for a month-precision release: the sentence is "August
      // 2007", and the formatter needs a day to render a month at all.
      new Date(Date.UTC(year, month - 1, 1)),
    ),
  );
  const devStatus = developmentStatus(payload.devstatus, t);
  const developers = rankDevelopers(payload.developers)
    .map(developerLabel)
    .filter((name): name is string => name !== null);
  const languages = payload.languages.map((code) => languageName(code, t));
  const platforms = payload.platforms.map((code) => platformName(code, t));
  const links = splitExtLinks(payload.extlinks);

  const hasBody =
    released !== null ||
    devStatus !== null ||
    developers.length > 0 ||
    languages.length > 0 ||
    platforms.length > 0 ||
    payload.length_minutes !== null ||
    typeof payload.rating === "number" ||
    links.primary.length > 0;

  return (
    <Card>
      <CardHeader
        title={t("detail.facts")}
        aside={
          <Button
            variant="ghost"
            onClick={() => openVndb(vndbVnUrl(vnId))}
            title={t("detail.viewOnVndbHint", { vn: vnId })}
          >
            {t("detail.viewOnVndb")}
            <span aria-hidden="true" style={{ display: "inline-flex", marginLeft: 6 }}>
              <Icon name="spark" size={12} />
            </span>
          </Button>
        }
      />

      {hasBody ? (
        <dl className="facts">
          {released ? (
            <div className="facts__group">
              <dt className="facts__label">{t("detail.released")}</dt>
              <dd className="facts__value">{released}</dd>
            </div>
          ) : null}

          {devStatus ? (
            <div className="facts__group">
              <dt className="facts__label">{t("detail.status")}</dt>
              <dd className="facts__value">{devStatus}</dd>
            </div>
          ) : null}

          {developers.length > 0 ? (
            <div className="facts__group">
              <dt className="facts__label">
                {developers.length > 1 ? t("detail.developers") : t("detail.developer")}
              </dt>
              <dd className="facts__value">{developers.join(", ")}</dd>
            </div>
          ) : null}

          {/* The community score, with its vote count in the same block. "8.5" from
              six votes and "8.5" from nine thousand are not the same claim, and the
              hero chip cannot fit the caveat. */}
          {typeof payload.rating === "number" ? (
            <div className="facts__group">
              <dt className="facts__label">{t("detail.vndbScore")}</dt>
              <dd className="facts__value tnum">
                {/* VNDB returns both values in tenths (65 is 6.5), the same scale
                    the user's own rating is stored on, so they share one formatter. */}
                {formatRating(payload.rating)}
                <span className="facts__sub">
                  {t("detail.vndbVotes", { count: payload.votecount ?? 0 })}
                  {typeof payload.average === "number"
                    ? ` · ${t("detail.vndbAverage", { value: formatRating(payload.average) })}`
                    : ""}
                </span>
              </dd>
            </div>
          ) : null}

          {/* The estimate next to the tracked time, for the same reason two ratings
              sit side by side: the interesting number is the gap between them. */}
          {payload.length_minutes !== null ? (
            <div className="facts__group">
              <dt className="facts__label">{t("detail.vndbLength")}</dt>
              <dd className="facts__value tnum">
                {formatDurationLong(payload.length_minutes * 60_000, locale)}
                <span className="facts__sub">
                  {t("detail.vndbLengthVotes", { count: payload.length_votes ?? 0 })}
                  {playtimeMs > 0
                    ? ` · ${t("detail.active")}: ${formatDurationLong(playtimeMs, locale)}`
                    : ""}
                </span>
              </dd>
            </div>
          ) : null}

          {languages.length > 0 ? (
            <div className="facts__group">
              <dt className="facts__label">{t("detail.languages")}</dt>
              <dd className="facts__value">{languages.join(", ")}</dd>
            </div>
          ) : null}

          {platforms.length > 0 ? (
            <div className="facts__group">
              <dt className="facts__label">{t("detail.platforms")}</dt>
              <dd className="facts__value">{platforms.join(", ")}</dd>
            </div>
          ) : null}
        </dl>
      ) : (
        <p className="body">{t("detail.unknownDate")}</p>
      )}

      {links.primary.length > 0 ? (
        <div style={{ marginTop: "var(--space-3)" }}>
          <p className="facts__label">{t("detail.links")}</p>
          <ul className="facts__links">
            {links.primary.map((link) => (
              <li key={link.url ?? ""}>
                <button
                  type="button"
                  className="linkbtn"
                  onClick={() => {
                    if (link.url) openVndb(link.url);
                  }}
                >
                  {link.label ?? link.url}
                </button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {links.reference.length > 0 ? (
        <details className="facts__more">
          {/* Named for what is behind it. A bare "Links" summary under a "LINKS"
              heading read as a broken duplicate of the row above. */}
          <summary className="caption">
            {t("detail.linksMore")} ({links.reference.length})
          </summary>
          <ul className="facts__links">
            {links.reference.map((link) => (
              <li key={link.url ?? ""}>
                <button
                  type="button"
                  className="linkbtn"
                  onClick={() => {
                    if (link.url) openVndb(link.url);
                  }}
                >
                  {link.label ?? link.url}
                </button>
              </li>
            ))}
          </ul>
        </details>
      ) : null}
    </Card>
  );
}
