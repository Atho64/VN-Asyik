/** Strips VNDB formatting tags for plain-text display, matching `vndb::import::strip_formatting`. */
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
