import type { TitleVariant } from "@/lib/ipc/types";

/** A name this VN goes by, with what kind of name it is.
 *
 * The old header emitted a bare list of strings, so a reader saw
 * "智代アフター ～It's a Wonderful Life～" and "Tomoyo After" with no way to know
 * which was the Japanese release name, which was the romaji, and which was the
 * official English title. Labelling each one is the whole point.
 */
export type TitleRole =
  | "english"
  | "romaji"
  | "japanese"
  | "original"
  | "chinese"
  | "korean"
  | "alias";

/** Which script a string is written in.
 *
 * VNDB aliases carry no language field — "团子大家族" is listed beside
 * "クラナド" with nothing to tell them apart — so the script is detected rather
 * than guessed at. Han characters are reported as Chinese: Japanese names with
 * kanji also use them, but a name made *only* of han is not distinguishable from
 * Chinese without a dictionary, and calling it Japanese would be a worse guess.
 */
function scriptOf(text: string): "kana" | "han" | "hangul" | "latin" | "mixed" {
  const seen = new Set<string>();
  for (const character of text) {
    if (/\s/.test(character)) continue;
    const code = character.codePointAt(0) ?? 0;
    if (code >= 0x3040 && code <= 0x30ff) seen.add("kana");
    else if (code >= 0x4e00 && code <= 0x9fff) seen.add("han");
    else if (code >= 0xac00 && code <= 0xd7af) seen.add("hangul");
    else if (code > 0x2000) seen.add("other");
    else seen.add("latin");
  }
  if (seen.has("kana")) return "kana";
  if (seen.has("hangul")) return "hangul";
  // Han plus punctuation is Chinese; han plus Latin is a romanised form.
  if (seen.has("han")) return seen.has("latin") ? "mixed" : "han";
  return "latin";
}

export interface ShownTitle {
  role: TitleRole;
  text: string;
}

/** Picks one entry per script, in the order a reader looks for them.
 *
 * - `english`: VNDB's own English title, when it differs from what is shown.
 * - `romaji`: the Latin transliteration of the Japanese title.
 * - `japanese`: the original-script title.
 *
 * Korean and Chinese names are deliberately not surfaced here. They exist on
 * many entries, and a header listing four scripts was the noise the previous
 * implementation was written to avoid — but they were being dropped silently
 * *along with* the useful ones, which is the bug this replaces.
 */
export function titledNames(
  primary: string,
  original: string | null,
  titles: TitleVariant[],
  aliases: string[] = [],
): ShownTitle[] {
  const seen = new Set<string>();
  const remember = (value: string | null | undefined): string | null => {
    const text = value?.trim();
    if (!text) return null;
    const key = text.toLowerCase();
    if (seen.has(key)) return null;
    seen.add(key);
    return text;
  };

  // The displayed title always wins the dedupe, so it is never repeated below.
  remember(primary);

  const out: ShownTitle[] = [];
  const push = (role: TitleRole, value: string | null | undefined): void => {
    const text = remember(value);
    if (text) out.push({ role, text });
  };

  const byLang = (lang: string): TitleVariant | undefined =>
    titles.find((entry) => entry.lang === lang);

  const ja = byLang("ja");
  const en = byLang("en");

  // English first: for a reader browsing in English it is the name they are most
  // likely to recognise, and VNDB only carries it when it differs.
  push("english", en?.title);
  // Then the romaji, which is what somebody types into a search box.
  push("romaji", ja?.latin);
  // Then the Japanese itself, which is what the cover and save files say.
  push("japanese", ja?.title);

  // The library row's own original title, which survives independently of
  // `titles` and is what a manually added game carries.
  push("original", original);

  for (const alias of aliases) {
    // A name is only worth showing if it says something the ones above do not.
    // Labelling by script is what stops a Chinese alias appearing as a second
    // unlabelled "also called" beside the Japanese one.
    const script = scriptOf(alias);
    const role: TitleRole =
      script === "han"
        ? "chinese"
        : script === "hangul"
          ? "korean"
          : script === "kana"
            ? "japanese"
            : "alias";
    push(role, alias);
  }
  return out;
}
