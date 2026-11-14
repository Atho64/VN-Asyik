/**
 * Pure formatters translating VNDB codes (BCP-47 tags, platform codes, dev status)
 * into localized display labels, passing unknown codes through verbatim.
 */

import type { VndbExtLink, VndbProducer } from "@/lib/ipc/types";

type Translate = (key: string) => string;

/** Resolves primary language subtag via i18n and keeps regional subtags as qualifiers (e.g. `zh-Hans`). */
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

/** Formats a partial or full VNDB release date (`YYYY`, `YYYY-MM`, `YYYY-MM-DD`, `TBA`, `unknown`). */
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

/** Orders developer credits with companies (in API rank order) ahead of individual or circle credits. */
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

/** Partitions VNDB `extlinks` into primary user links (official sites, stores, patches) and secondary reference links. */
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

