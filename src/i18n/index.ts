/**
 * Translation lookup with `{{name}}` interpolation and plural selection.
 *
 * A tiny implementation on purpose: two locales, one interpolation form, no ICU
 * runtime. The Rust side never renders text, so this is the only formatter and it
 * is fully testable.
 */

import { en, type MessageKey, type Messages } from "./en";
import { id } from "./id";

export type Locale = "en" | "id";

export const locales: Locale[] = ["en", "id"];

const dictionaries: Record<Locale, Messages> = { en, id };

export type TranslateVars = Record<string, string | number>;

export interface PluralArgs {
  /** The count that decides singular vs plural. */
  count: number;
  /** Extra interpolation variables. */
  vars?: TranslateVars;
}

/** Resolve a locale tag to a supported one, defaulting to English. */
export function resolveLocale(tag: string | null | undefined): Locale {
  if (!tag) return "en";
  const base = tag.toLowerCase().split("-")[0];
  return base === "id" ? "id" : "en";
}

function isPluralArgs(args: PluralArgs | TranslateVars): args is PluralArgs {
  return typeof (args as PluralArgs).count === "number";
}

function interpolate(template: string, vars?: TranslateVars): string {
  if (!vars) return template;
  return template.replace(/\{\{(\w+)\}\}/g, (whole, name: string) => {
    const value = vars[name];
    return value === undefined ? whole : String(value);
  });
}

/**
 * Look up a message.
 *
 * Plural keys use the `_one` / `_other` suffix. A missing translation falls back
 * to English, and an entirely missing key returns the key itself so a bug is
 * visible in the UI instead of rendering an empty string.
 */
export function translate(
  locale: Locale,
  key: MessageKey | string,
  args?: PluralArgs | TranslateVars,
): string {
  const dict = dictionaries[locale] ?? en;

  if (args && isPluralArgs(args)) {
    const suffix = args.count === 1 ? "_one" : "_other";
    const pluralKey = `${key}${suffix}`;
    const template =
      (dict as Record<string, string>)[pluralKey] ??
      (en as Record<string, string>)[pluralKey];
    if (template) {
      return interpolate(template, { ...(args.vars ?? {}), count: args.count });
    }
  }

  const template =
    (dict as Record<string, string>)[key] ?? (en as Record<string, string>)[key] ?? key;
  return interpolate(template, args as TranslateVars | undefined);
}

export type Translator = (
  key: MessageKey | string,
  args?: PluralArgs | TranslateVars,
) => string;

export function createTranslator(locale: Locale): Translator {
  return (key, args) => translate(locale, key, args);
}
