import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { createTranslator } from "@/i18n";
import { RatingPicker, parseRating } from "./RatingPicker";

vi.mock("@/app/useI18n", () => ({
  useI18n: () => ({ t: createTranslator("en"), locale: "en" }),
}));

/** What the game screen does: the committed value comes back as the prop. */
function Harness({ onChange }: { onChange: (rating: number | null) => void }) {
  const [value, setValue] = useState<number | null>(null);
  return (
    <RatingPicker
      value={value}
      onChange={(next) => {
        onChange(next);
        setValue(next);
      }}
    />
  );
}

function field(): HTMLInputElement {
  return screen.getByRole("textbox");
}

describe("RatingPicker typing", () => {
  it("keeps a decimal being typed instead of rewriting it", () => {
    // Typing "7.5" used to be impossible: the first keystroke committed 7, the
    // stored value came back as "7.0" and overwrote the field, so the next
    // character landed on top of it ("7.0." and then an error).
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);

    fireEvent.focus(field());
    fireEvent.change(field(), { target: { value: "7" } });
    expect(field()).toHaveValue("7");

    fireEvent.change(field(), { target: { value: "7." } });
    expect(field()).toHaveValue("7.");
    expect(field()).not.toHaveAttribute("aria-invalid", "true");

    fireEvent.change(field(), { target: { value: "7.5" } });
    expect(field()).toHaveValue("7.5");
    expect(onChange).toHaveBeenLastCalledWith(75);

    fireEvent.blur(field());
    expect(field()).toHaveValue("7.5");
  });

  it("reports a value outside the scale, and reverts it on blur", () => {
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);

    fireEvent.focus(field());
    fireEvent.change(field(), { target: { value: "11" } });
    expect(field()).toHaveAttribute("aria-invalid", "true");
    expect(onChange).not.toHaveBeenCalledWith(110);

    fireEvent.blur(field());
    expect(field()).not.toHaveAttribute("aria-invalid", "true");
    expect(field()).toHaveValue("");
  });

  it("clears the rating when the field is emptied", () => {
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);

    fireEvent.focus(field());
    fireEvent.change(field(), { target: { value: "8" } });
    fireEvent.change(field(), { target: { value: "" } });
    expect(onChange).toHaveBeenLastCalledWith(null);
  });

  it("accepts a comma as the decimal separator while typing", () => {
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);

    fireEvent.focus(field());
    fireEvent.change(field(), { target: { value: "7," } });
    expect(field()).toHaveValue("7,");
    fireEvent.change(field(), { target: { value: "7,5" } });
    expect(onChange).toHaveBeenLastCalledWith(75);
  });

  it("adopts a rating that changes from outside when it is not being typed in", () => {
    // A VNDB sync can land while the field is idle; it has to show up.
    function Outer({ value }: { value: number | null }) {
      return <RatingPicker value={value} onChange={() => {}} />;
    }
    const { rerender } = render(<Outer value={null} />);
    expect(field()).toHaveValue("");
    rerender(<Outer value={73} />);
    expect(field()).toHaveValue("7.3");
  });
});

describe("parseRating", () => {
  it("reads a whole number as its star value", () => {
    expect(parseRating("7")).toEqual({ kind: "value", decivotes: 70 });
    expect(parseRating("10")).toEqual({ kind: "value", decivotes: 100 });
    expect(parseRating("1")).toEqual({ kind: "value", decivotes: 10 });
  });

  it("reads one decimal place", () => {
    expect(parseRating("7.5")).toEqual({ kind: "value", decivotes: 75 });
    expect(parseRating("7.3")).toEqual({ kind: "value", decivotes: 73 });
    expect(parseRating("10.0")).toEqual({ kind: "value", decivotes: 100 });
  });

  it("accepts a comma, because some keyboards only offer one", () => {
    expect(parseRating("7,5")).toEqual({ kind: "value", decivotes: 75 });
  });

  it("trims surrounding space", () => {
    expect(parseRating("  8.5 ")).toEqual({ kind: "value", decivotes: 85 });
  });

  it("treats empty as no rating, which is different from a bad one", () => {
    // Empty clears the rating; invalid must not, or a typo would silently
    // delete a rating the user had already set.
    expect(parseRating("")).toEqual({ kind: "empty" });
    expect(parseRating("   ")).toEqual({ kind: "empty" });
  });

  it("rejects values outside the scale", () => {
    expect(parseRating("0")).toEqual({ kind: "bad" });
    expect(parseRating("0.9")).toEqual({ kind: "bad" });
    expect(parseRating("10.1")).toEqual({ kind: "bad" });
    expect(parseRating("11")).toEqual({ kind: "bad" });
    expect(parseRating("99")).toEqual({ kind: "bad" });
  });

  it("rejects more than one decimal place", () => {
    expect(parseRating("7.55")).toEqual({ kind: "bad" });
  });

  it("rejects anything that is not a number", () => {
    for (const text of ["abc", "7.", ".5", "-7", "1e1", "7 5", "+7"]) {
      expect(parseRating(text), text).toEqual({ kind: "bad" });
    }
  });
});
