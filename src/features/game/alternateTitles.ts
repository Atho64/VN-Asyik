import type { TitleVariant } from "@/lib/ipc/types";

/**
 * Which languages of a VN's name are worth showing.
 *
 * Japanese because it is what the cover and save files say; English because it is
 * what most people search for. VNDB also publishes Russian, Chinese, Korean,
 * German and more, and a row of five scripts under the title is noise when the
 * reader wants to match a name rather than survey localisations.
 */
const SHOWN_LANGUAGES = new Set(["ja", "en"]);

/**
 * Every other name this VN goes by, in the order they are worth reading.
 *
 * Filtered against the main title and against the names already kept, because VNDB
 * repeats itself freely: CLANNAD stores "CLANNAD" as its ja, en *and* zh-Hans
 * title, and `alttitle` often duplicates `title`, so the header used to print the
 * same word twice.
 *
 * `original` is the library row's own name for the VN, so it is passed in rather
 * than guessed at here.
 */
export function alternateTitles(
  primary: string,
  original: string | null,
  titles: TitleVariant[],
  aliases: string[] = [],
): string[] {
  const seen = new Set<string>([primary.trim().toLowerCase()]);
  if (original) seen.add(original.trim().toLowerCase());
  const out: string[] = [];
  const push = (value: string | null | undefined): void => {
    const text = value?.trim();
    if (!text) return;
    const key = text.toLowerCase();
    if (seen.has(key)) return;
    seen.add(key);
    out.push(text);
  };

  // Original script before translation: the Japanese name is the identity. The
  // `official` flag is not used because VNDB leaves it null on every title in
  // practice, so sorting on it would be sorting on nothing.
  const kept = titles.filter(
    (entry) => entry.lang !== null && SHOWN_LANGUAGES.has(entry.lang),
  );
  const sorted = [...kept].sort((a, b) => {
    const rank = (entry: TitleVariant): number => (entry.lang === "ja" ? 0 : 1);
    return rank(a) - rank(b);
  });
  for (const entry of sorted) {
    push(entry.title);
  }
  // Romanised forms for the same languages, which is how a Japanese-only release
  // gets a Latin name without an official English title existing.
  for (const entry of sorted) {
    push(entry.latin);
  }
  if (original) push(original);
  for (const alias of aliases) {
    push(alias);
  }
  return out;
}
