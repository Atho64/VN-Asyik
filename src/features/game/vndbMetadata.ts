/**
 * Turning VNDB's coded values into something a reader can use.
 *
 * The API reports languages as BCP-47-ish tags (`ja`, `zh-Hans`, `pt-BR`),
 * platforms as three-letter codes (`win`, `psp`) and development status as an
 * integer. None of those are display strings, and none of them should be pasted
 * into the UI raw: a facts row reading `win, psp, drc` is data, not information.
 *
 * Two rules here:
 *
 * 1. **Unknown codes pass through unchanged.** VNDB adds platforms, and a new code
 *    must degrade to its own abbreviation rather than vanish or throw. This is why
 *    every lookup takes the caller's translator instead of importing i18n: the
 *    module stays pure and testable.
 *
 * 2. **A region matters where it changes the reader's answer.** `zh-Hans` and
 *    `zh-Hant` are different releases with different availability, so the region is
 *    kept as a qualifier rather than flattened to "Chinese".
 */

import type { VndbExtLink, VndbProducer } from "@/lib/ipc/types";

type Translate = (key: string) => string;

/**
 * Prefix-based language naming.
 *
 * The tag is split on `-` and the primary subtag drives the name; the remainder is
 * appended as a parenthesised qualifier, so `zh-Hans` becomes "Chinese (Hans)" rather
 * than either an unknown string or a bare "Chinese" that hides a real distinction.
 */
export function languageName(tag: string, t: Translate): string {
  const trimmed = tag.trim();
  if (trimmed.length === 0) return t("common.unknown");
  const [primary = "", ...rest] = trimmed.split("-");
  const capitalised = `${primary.charAt(0).toUpperCase()}${primary.slice(1).toLowerCase()}`;
  const key = `detail.languageName${capitalised}`;
  const name = t(key);
  // `t` returns the key itself when it is missing, which is the signal that this
  // language has no display name yet.
  const known = name !== key;
  if (!known) return rest.length > 0 ? `${primary}-${rest.join("-")}` : primary;
  if (rest.length === 0) return name;

  // Regions that have their own name rather than a qualifier. "Portuguese (br)"
  // reads as a database key; "Brazilian Portuguese" is what the release is called,
  // and it is the distinction a reader is actually looking for.
  const region = rest
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1).toLowerCase()}`)
    .join("");
  const combinedKey = `detail.languageName${capitalised}${region}`;
  const combined = t(combinedKey);
  if (combined !== combinedKey) return combined;

  return `${name} (${rest.join("-")})`;
}

/** Platform codes VNDB uses, mapped onto the keys in the message catalogue. */
export function platformName(code: string, t: Translate): string {
  const trimmed = code.trim();
  if (trimmed.length === 0) return t("common.unknown");
  const normalised =
    trimmed.charAt(0).toUpperCase() + trimmed.slice(1).toLowerCase();
  const key = `detail.platformName${normalised}`;
  const name = t(key);
  // An unrecognised platform shows its own code. Inventing a label would be worse
  // than showing `win9x`, and hiding it would misreport the entry.
  return name === key ? trimmed : name;
}

/** VNDB's development status integer, as a display string. */
export function developmentStatus(status: number | null, t: Translate): string | null {
  switch (status) {
    case 0:
      return t("detail.finished");
    case 1:
      return t("detail.inDevelopment");
    case 2:
      return t("detail.cancelled");
    default:
      return null;
  }
}

/**
 * Format a VNDB release date for display.
 *
 * VNDB dates are intentionally partial: `2007`, `2007-08` and `2007-08-17` all mean
 * different amounts of precision, and padding a year-only date to January 1st would
 * claim a day nobody knows. So the string is reformatted only as far as it is known.
 * `TBA` and `unknown` are real values the API documents.
 */
export function formatReleaseDate(
  value: string | null,
  t: Translate,
  formatMonth: (year: number, month: number) => string,
): string | null {
  if (!value) return null;
  const trimmed = value.trim();
  if (trimmed.length === 0) return null;
  if (/^tba$/i.test(trimmed)) return t("detail.unreleased");
  if (/^unknown$/i.test(trimmed)) return t("detail.unknownDate");

  const match = /^(\d{4})(?:-(\d{2}))?(?:-(\d{2}))?$/.exec(trimmed);
  if (!match) {
    // Not a shape this function knows: show what VNDB said rather than nothing.
    return trimmed;
  }
  const [, year = "", month = "", day = ""] = match;
  if (day.length === 2) return `${year}-${month}-${day}`;
  const monthNumber = Number.parseInt(month, 10);
  if (month.length === 2 && Number.isFinite(monthNumber) && monthNumber >= 1 && monthNumber <= 12) {
    return formatMonth(Number.parseInt(year, 10), monthNumber);
  }
  return year;
}

/**
 * Order developers so the ones worth reading come first.
 *
 * VNDB returns them as it likes. A company that developed the VN is the answer to
 * "who made this"; an amateur group or an individual is usually a credit rather than
 * a publisher, and a long list of both buries the one name a reader wants. Companies
 * keep their API order among themselves, which is the site's own ranking.
 */
export function rankDevelopers(developers: VndbProducer[]): VndbProducer[] {
  const named = developers.filter(
    (entry) => (entry.name ?? "").trim().length > 0 || (entry.original ?? "").trim().length > 0,
  );
  return [...named].sort((a, b) => rank(a) - rank(b));
}

function rank(producer: VndbProducer): number {
  // `co` is a company, `in` an individual, `ng` an amateur group. The field keeps
  // VNDB's own name (`type`) because that is what the API returns.
  switch (producer.type) {
    case "co":
      return 0;
    case "ng":
      return 1;
    case "in":
      return 2;
    default:
      return 3;
  }
}

/** Developer name, preferring the readable one and keeping the original script. */
export function developerLabel(producer: VndbProducer): string | null {
  const name = (producer.name ?? "").trim();
  const original = (producer.original ?? "").trim();
  if (name.length > 0 && original.length > 0 && name !== original) {
    return `${name} · ${original}`;
  }
  if (name.length > 0) return name;
  return original.length > 0 ? original : null;
}

/** Sites that are a database or encyclopaedia rather than an official page. */
const REFERENCE_SITES = new Set([
  "enwiki",
  "jawiki",
  "wikidata",
  "mobygames_game",
  "gamefaqs_game",
  "igdb_game",
  "pcgamingwiki",
  "wine",
  "howlongtobeat",
  "renai",
  "erogamescape",
  "vndb",
]);

/**
 * Split external links into the ones people actually click and the rest.
 *
 * VNDB's `extlinks` for one VN routinely holds a dozen entries, most of them
 * databases. The official site, a Steam or GOG page and a translation patch are what
 * a reader wants; the fourth GameFAQs mirror is not. Both lists are returned because
 * the reference links are still worth having behind a disclosure, just not in the
 * first row.
 */
export function splitExtLinks(links: VndbExtLink[]): {
  primary: VndbExtLink[];
  reference: VndbExtLink[];
} {
  const usable = links.filter((link) => {
    const url = (link.url ?? "").trim();
    // Only http(s) links are accepted. A `javascript:` URL from an API response must
    // never reach an anchor's `href`, so the scheme is checked here rather than
    // trusting the payload.
    return /^https?:\/\//i.test(url);
  });
  const seen = new Set<string>();
  const unique = usable.filter((link) => {
    const url = (link.url ?? "").trim();
    if (seen.has(url)) return false;
    seen.add(url);
    return true;
  });

  const primary: VndbExtLink[] = [];
  const reference: VndbExtLink[] = [];
  for (const link of unique) {
    if (REFERENCE_SITES.has((link.name ?? "").toLowerCase())) {
      reference.push(link);
    } else {
      primary.push(link);
    }
  }
  return { primary, reference };
}

/** The vndb.org page for a VN id. */
export function vndbVnUrl(vnId: string): string {
  return `https://vndb.org/${encodeURIComponent(vnId.trim())}`;
}

