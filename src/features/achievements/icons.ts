/**
 * The 65 achievement glyphs, inlined as raw SVG so `stroke="currentColor"`
 * follows the theme. An `<img src>` would resolve currentColor to black inside
 * the replaced element and disappear on the dark surface, so the markup is
 * injected into a sized, aria-hidden span instead (the name beside it is text).
 */

const modules = import.meta.glob<string>("../../assets/achievements/*.svg", {
  eager: true,
  query: "?raw",
  import: "default",
});

const bySlug: Record<string, string> = {};
for (const [path, raw] of Object.entries(modules)) {
  const slug = path.split("/").pop()?.replace(/\.svg$/, "");
  if (slug) bySlug[slug] = raw;
}

/** Raw SVG markup for a slug, or undefined when the asset is missing. */
export function achievementIcon(slug: string): string | undefined {
  return bySlug[slug];
}

/** Every slug that ships an icon, sorted; the catalog's source of truth in the UI. */
export const achievementSlugs: string[] = Object.keys(bySlug).sort();