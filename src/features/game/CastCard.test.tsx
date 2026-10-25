import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import { CastCard } from "./CastCard";
import type { VndbCharacter } from "@/lib/ipc/types";

function character(overrides: Partial<VndbCharacter>): VndbCharacter {
  return {
    id: "c1",
    name: "Okabe Rintarou",
    original: "岡部 倫太郎",
    role: "main",
    image: {
      id: "cv1",
      url: "https://t.vndb.org/ch/1.jpg",
      dims: null,
      sexual: null,
      violence: null,
      votecount: null,
      thumbnail: null,
      thumbnail_dims: null,
    },
    ...overrides,
  };
}

function renderCast(characters: VndbCharacter[]) {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <CastCard characters={characters} />
    </AppContext.Provider>,
  );
}

describe("CastCard", () => {
  it("renders nothing at all when there is no cast", () => {
    // An empty "Characters" heading reads as a loading failure, which is why
    // every other VNDB card assembles only the rows it has.
    const { container } = renderCast([]);
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByText("Characters")).not.toBeInTheDocument();
  });

  it("shows the romanised name, the original script and the role", () => {
    renderCast([character({})]);
    expect(screen.getByText("Okabe Rintarou")).toBeInTheDocument();
    expect(screen.getByText("岡部 倫太郎")).toBeInTheDocument();
    expect(screen.getByText("Main")).toBeInTheDocument();
  });

  it("does not print the original when it duplicates the shown name", () => {
    // Some entries carry the same string in both fields; printing it twice looks
    // like a rendering bug.
    renderCast([character({ name: "Kurisu", original: "Kurisu" })]);
    expect(screen.getAllByText("Kurisu")).toHaveLength(1);
  });

  it("keeps the VNDB order rather than sorting by name", () => {
    // VNDB already puts the protagonist first; a local sort would bury them.
    renderCast([
      character({ id: "a", name: "Protagonist", role: "main" }),
      character({ id: "b", name: "Aardvark", role: "side" }),
    ]);
    const names = screen.getAllByText(/Protagonist|Aardvark/).map((n) => n.textContent);
    expect(names[0]).toBe("Protagonist");
  });

  it("collapses the tail behind a disclosure instead of an endless list", () => {
    const many = Array.from({ length: 20 }, (_, index) =>
      character({ id: `c${index}`, name: `Character ${index}` }),
    );
    renderCast(many);
    expect(screen.getByText("Character 0")).toBeInTheDocument();
    expect(screen.getByText("8 more")).toBeInTheDocument();
    // The tail is inside the collapsed <details>, so it is in the document but
    // not rendered. Asserting absence would be wrong: jsdom keeps collapsed
    // disclosure content in the DOM.
    const tail = screen.getByText("Character 19");
    const disclosure = tail.closest("details");
    expect(disclosure).not.toBeNull();
    expect(disclosure?.open).toBe(false);
  });

  it("skips an entry that has no name at all", () => {
    renderCast([character({ name: null, original: null, id: "empty" })]);
    // The card still renders because the list is non-empty; it just has no tile.
    expect(screen.queryByRole("listitem")).not.toBeInTheDocument();
  });
});
