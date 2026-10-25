import type { Locale } from "@/i18n";
import type { CharacterTrait } from "@/lib/ipc/types";

/** Spoiler level above which traits are withheld by default.
 *
 * 0 means only safe traits show. VNDB marks levels 1 and 2 as plot reveals:
 * across CLANNAD's cast, 234 of 1 138 trait rows sit above level 0, and some of
 * them name the culprit in a mystery. The game page therefore conceals them the
 * way it conceals explicit art — behind a deliberate reveal, not on by default.
 */
export const MAX_SAFE_SPOILER = 0;

/** Whether a trait may be shown at the given spoiler ceiling.
 *
 * An absent level counts as 0: VNDB omits the field rather than sending 0, and
 * treating "missing" as unsafe would hide most of a character's hair and eyes.
 */
export function isTraitVisible(trait: CharacterTrait, ceiling: number): boolean {
  return (trait.spoiler ?? 0) <= ceiling;
}

/** Traits grouped by their parent category, safe ones only unless raised.
 *
 * Order follows the source data rather than a local ranking: VNDB already emits
 * traits in a stable, meaningful order per group, and re-sorting would put
 * "Blue" before "Hair" style names for no reader's benefit.
 */
export function groupTraits(
  traits: CharacterTrait[],
  ceiling: number = MAX_SAFE_SPOILER,
): Array<[string, string[]]> {
  const groups = new Map<string, string[]>();
  for (const trait of traits) {
    if (!isTraitVisible(trait, ceiling)) continue;
    const name = trait.name?.trim();
    if (!name) continue;
    // A trait with no group is still worth showing; `Other` keeps it from being
    // silently dropped, which is what a null key would do.
    const group = trait.group_name?.trim() || "Other";
    const bucket = groups.get(group);
    if (bucket) {
      if (!bucket.includes(name)) bucket.push(name);
    } else {
      groups.set(group, [name]);
    }
  }
  return [...groups.entries()];
}

/** A birthday as a readable date, or null when VNDB has none.
 *
 * Uses a fixed month table rather than `Intl` on a Date: the value is a
 * month/day pair with no year, and constructing a Date for it would invent one
 * (and shift the day in some timezones).
 */
export function formatBirthday(value: number[] | null, locale: Locale): string | null {
  if (!value || value.length < 2) return null;
  const [month, day] = value;
  if (!month || month < 1 || month > 12 || !day || day < 1 || day > 31) return null;
  const en = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
  ];
  const id = [
    "Januari",
    "Februari",
    "Maret",
    "April",
    "Mei",
    "Juni",
    "Juli",
    "Agustus",
    "September",
    "Oktober",
    "November",
    "Desember",
  ];
  return `${(locale === "id" ? id : en)[month - 1]} ${day}`;
}

/** Blood type as the label people expect, or null. */
export function formatBloodType(value: string | null): string | null {
  const text = value?.trim();
  if (!text) return null;
  return text.toUpperCase();
}
