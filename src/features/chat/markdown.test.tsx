import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

const mocks = { openExternal: vi.fn() };
vi.mock("../../lib/ipc/client", () => ({
  api: {
    openExternal: (url: string) => mocks.openExternal(url),
  },
}));

const { renderMarkdown, safeHref } = await import("./markdown");

beforeEach(() => {
  mocks.openExternal.mockReset();
  mocks.openExternal.mockResolvedValue(undefined);
});

const show = (source: string) => render(<div>{renderMarkdown(source)}</div>);

describe("inline formatting", () => {
  it("renders **bold** as bold instead of literal asterisks", () => {
    show("The route is **hidden** after chapter 3.");
    const strong = screen.getByText("hidden");
    expect(strong.tagName).toBe("STRONG");
    expect(screen.queryByText(/\*\*/)).toBeNull();
  });

  it("renders *italic* and _italic_", () => {
    show("A *quiet* and _gentle_ story.");
    expect(screen.getByText("quiet").tagName).toBe("EM");
    expect(screen.getByText("gentle").tagName).toBe("EM");
  });

  it("keeps backtick content literal so it cannot be bolded or linked", () => {
    show("Call `search_library` with **exact** ids.");
    expect(screen.getByText("search_library").tagName).toBe("CODE");
    expect(screen.getByText("exact").tagName).toBe("STRONG");
  });

  it("treats an unterminated ** as text rather than swallowing the reply", () => {
    show("This answer was cut **off mid");
    expect(screen.getByText(/This answer was cut/)).toBeInTheDocument();
    expect(screen.getByText(/\*\*off mid/)).toBeInTheDocument();
  });
});

describe("links", () => {
  it("routes an http(s) link through open_external instead of an href", () => {
    // A model reply is third-party text, so the URL goes to Rust's scheme
    // allowlist rather than into an href the webview can navigate to.
    show("See [VNDB](https://vndb.org/v11) for details.");
    const link = screen.getByRole("button", { name: "VNDB" });
    expect(link).not.toHaveAttribute("href");
    link.click();
    expect(mocks.openExternal).toHaveBeenCalledWith("https://vndb.org/v11");
  });

  it("refuses javascript: and data: targets", () => {
    show("Click [here](javascript:alert(1)) now.");
    expect(screen.queryByRole("button", { name: "here" })).toBeNull();
    expect(screen.getByText(/\[here\]\(javascript:alert\(1\)\)/)).toBeInTheDocument();
  });

  it("refuses credentialed URLs", () => {
    expect(safeHref("https://user:pass@example.com")).toBeNull();
    expect(safeHref("https://vndb.org/v11")).toBe("https://vndb.org/v11");
  });
});

describe("blocks", () => {
  it("renders headings, lists and quotes", () => {
    show(
      ["## Library", "", "- CLANNAD", "- Rewrite", "", "> A quiet read.", ""].join("\n"),
    );
    expect(screen.getByRole("heading", { name: "Library" })).toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
    expect(screen.getByText("A quiet read.")).toBeInTheDocument();
  });

  it("keeps numbered lists starting at the number the model wrote", () => {
    show("3. third\n4. fourth");
    const list = screen.getByRole("list");
    expect(list.tagName).toBe("OL");
    expect(list).toHaveAttribute("start", "3");
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });

  it("starts a new paragraph per blank line but joins wrapped lines", () => {
    show("first line\nstill first\n\nsecond para");
    expect(screen.getByText("first line still first")).toBeInTheDocument();
    expect(screen.getByText("second para")).toBeInTheDocument();
  });
});

describe("untrusted output", () => {
  it("never interprets raw HTML from the model", () => {
    show('<img src=x onerror="alert(1)"><script>alert(2)</script>');
    expect(document.querySelector("img")).toBeNull();
    expect(document.querySelector("script")).toBeNull();
    expect(screen.getByText(/<img/)).toBeInTheDocument();
  });

  it("renders an empty answer as nothing rather than a stray empty block", () => {
    const { container } = render(<div>{renderMarkdown("")}</div>);
    expect(container.querySelector(".md-p")).toBeNull();
  });
});
