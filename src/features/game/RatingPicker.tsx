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
/** A trailing separator is a step in typing "7.5", not a mistake yet. */
const INCOMPLETE = /^\d{1,2}[.,]$/;

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
  // Whether the field is being typed in. Committing a keystroke changes the
  // stored value, and copying that back into the field rewrote "7" as "7.0" in
  // the middle of a word: "7.5" could never be typed, only pasted.
  const [focused, setFocused] = useState(false);
  // Adopt a value that changed from somewhere else (a VNDB sync, the first load)
  // as long as the user is not in the middle of typing one. Derived state during
  // render, so no effect and no cascading pass.
  const [seen, setSeen] = useState(stored);
  if (!focused && seen !== stored) {
    setSeen(stored);
    setDraft(stored);
    setInvalid(false);
  }

  function commit(text: string): void {
    setDraft(text);
    const parsed = parseRating(text);
    if (parsed.kind === "bad") {
      // Only the separator case stays quiet; anything else is worth reporting
      // while it is still being typed.
      setInvalid(!INCOMPLETE.test(text));
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
        onFocus={(event) => {
          setFocused(true);
          // Selecting the current value makes a new rating replace it; without
          // this, typing into a rated game appended to "10.0".
          event.currentTarget.select();
        }}
        onBlur={() => {
          // Nothing is lost: a rejected draft reverts to what is stored, and a
          // committed one lands in its normalised form.
          setFocused(false);
          setDraft(stored);
          setSeen(stored);
          setInvalid(false);
        }}
        onKeyDown={(event) => {
          // Enter means "done": blur commits and normalises the same way a tap
          // elsewhere does.
          if (event.key === "Enter") event.currentTarget.blur();
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
