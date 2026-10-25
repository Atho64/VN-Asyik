/**
 * Plain-text rendering of VNDB description text.
 *
 * The API returns descriptions containing VNDB formatting codes such as
 * `[url=https://x]text[/url]`, `[b]` and `[spoiler]`. This app has no rich-text
 * renderer yet, so codes are stripped rather than shown raw. Mirrors
 * `vndb::import::strip_formatting` so both sides agree.
 */
export function stripFormatting(text: string): string {
  let depth = 0;
  let out = "";
  for (const ch of text) {
    if (ch === "[") {
      depth += 1;
      continue;
    }
    if (ch === "]" && depth > 0) {
      depth -= 1;
      continue;
    }
    if (depth === 0) out += ch;
  }
  return out.trim();
}
