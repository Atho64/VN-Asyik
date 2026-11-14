import type { Locale } from "@/i18n";
import type { CharacterTrait, VndbCharacter } from "@/lib/ipc/types";

/** Maximum trait spoiler level shown unrevealed (levels 1-2 hold plot reveals). */
export const MAX_SAFE_SPOILER = 0;

/** Whether the character's per-VN entry carries a spoiler level >= 1 (`null` = safe). */
export function isSpoilerCharacter(character: VndbCharacter): boolean {
  return (character.spoiler ?? 0) >= 1;
}

/** Keeps safe characters ahead of spoiler-tagged entries so the initial cast slice stays spoiler-free. */
export function sortSpoilerLast(characters: VndbCharacter[]): VndbCharacter[] {
  const safe: VndbCharacter[] = [];
  const spoiled: VndbCharacter[] = [];
  for (const character of characters) {
    if (isSpoilerCharacter(character)) spoiled.push(character);
    else safe.push(character);
  }
  return [...safe, ...spoiled];
}

/** Whether a trait passes the spoiler ceiling (omitted levels default to 0/safe). */
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
