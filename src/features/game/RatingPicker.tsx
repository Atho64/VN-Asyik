import { useState } from "react";

import { useI18n } from "@/app/useI18n";
import { formatRating, RATING_MAX, RATING_MIN } from "@/lib/library";

/** What a typed string means. `Bad` is "not a rating", which is not the same as
 *  "no rating": empty clears, bad is rejected openly. */
type Parsed = { kind: "empty" } | { kind: "value"; decivotes: number } | { kind: "bad" };

/** Read typed text as decivotes. Accepts "7", "7.5", "7,5" and "10". */
export function parseRating(text: string): Parsed {
  const trimmed = text.trim().replace(",", ".");
  if (trimmed === "") return { kind: "empty" };
  // One optional decimal place, and no sign or exponent.
  if (!/^\d{1,2}(\.\d)?$/.test(trimmed)) return { kind: "bad" };
  const decivotes = Math.round(Number(trimmed) * 10);
  if (decivotes < RATING_MIN || decivotes > RATING_MAX) return { kind: "bad" };
  return { kind: "value", decivotes };
}

/**
 * Set a rating from the game screen, typed rather than clicked.
 *
 * A text field instead of a star row: VNDB's scale is 1.0 to 10.0 in tenths, so
 * a clickable version needs a hundred targets and still cannot reach 7.3
 * quickly. Typing "7.3" is one action and every value is reachable.
 *
 * The field keeps whatever is typed while editing and only commits text that
 * parses, so a half-typed "7." does not overwrite a stored 7.5. Empty clears the
 * rating; text that is out of range is reported and then reverted on blur, so a
 * typo never silently discards a rating the user had.
 */
export function RatingPicker({
  value,
  onChange,
  disabled,
}: {
  value: number | null;
  onChange: (rating: number | null) => void;
  disabled?: boolean;
}) {
  const { t } = useI18n();
  const stored = value === null ? "" : formatRating(value);
  // Keyed on the stored value, so a rating arriving from a VNDB sync remounts the
  // field with the new text. An effect that copied it across would be a
  // cascading render for the same result.
  const [draft, setDraft] = useState(stored);
  const [invalid, setInvalid] = useState(false);
  // Remount the field when the stored text changes from somewhere else (a VNDB
  // sync, or the screen's first load). Derived state during render, so no effect
  // and no cascading pass.
  const [seen, setSeen] = useState(stored);
  if (seen !== stored) {
    setSeen(stored);
    setDraft(stored);
    setInvalid(false);
  }

  function commit(text: string): void {
    setDraft(text);
    const parsed = parseRating(text);
    if (parsed.kind === "bad") {
      setInvalid(true);
      return;
    }
    setInvalid(false);
    onChange(parsed.kind === "empty" ? null : parsed.decivotes);
  }

  return (
    <div className="rating">
      <input
        className="input input--rating tnum"
        inputMode="decimal"
        value={draft}
        disabled={disabled}
        aria-label={t("detail.ratingLabel")}
        aria-invalid={invalid}
        placeholder={t("detail.ratingPlaceholder")}
        onChange={(event) => {
          commit(event.target.value);
        }}
        onBlur={() => {
          // Nothing is lost: a rejected draft reverts to what is stored.
          setDraft(stored);
          setInvalid(false);
        }}
      />
      <span className="rating__of">{t("detail.ratingOutOf")}</span>
      {invalid ? (
        <span className="rating__error" role="status">
          {t("detail.ratingRange")}
        </span>
      ) : null}
    </div>
  );
}
