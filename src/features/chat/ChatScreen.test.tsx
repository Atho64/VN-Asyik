import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { AppContext } from "@/app/AppProvider";
import { en } from "@/i18n/en";
import { ChatMessageView, safeSourceUrl } from "./ChatMessageView";
import { applyChatEvent } from "./ChatScreen";
import type { ChatEvent, ChatMessage, ChatTrace } from "@/lib/ipc/chat";

/** Minimal provider: these components only need `t` and `locale`. */
function renderMessage(message: ChatMessage) {
  const t = ((key: string, vars?: Record<string, string | number>) => {
    const text = (en as Record<string, string>)[key] ?? key;
    return vars
      ? Object.entries(vars).reduce(
          (out, [name, value]) => out.replaceAll(`{{${name}}}`, String(value)),
          text,
        )
      : text;
  }) as never;
  const value = { t, locale: "en" } as unknown as React.ContextType<typeof AppContext>;
  return render(
    <AppContext.Provider value={value}>
      <ChatMessageView message={message} />
    </AppContext.Provider>,
  );
}

const base: ChatMessage = {
  id: "m1",
  role: "assistant",
  content: "",
  status: "running",
  error: null,
  tools: [],
  sources: [],
  tokens: null,
  createdAt: 1_700_000_000_000,
};

describe("live chat events", () => {
  it("appends deltas in arrival order and replaces the running tool trace", () => {
    const running: ChatTrace = {
      id: "call_1",
      name: "library_stats",
      arguments: "{}",
      status: "running",
      result: null,
    };
    const events: ChatEvent[] = [
      { type: "started", message: base },
      { type: "delta", text: "You have " },
      { type: "delta", text: "12 hours." },
      { type: "tool", trace: running },
      {
        type: "tool",
        trace: { ...running, status: "complete", result: { playtime_ms: 1 } },
      },
    ];
    const message = events.reduce<ChatMessage | null>(applyChatEvent, null);
    expect(message?.content).toBe("You have 12 hours.");
    // The finished trace must replace the running one, not duplicate it.
    expect(message?.tools).toHaveLength(1);
    expect(message?.tools.at(0)?.status).toBe("complete");
  });

  it("takes the persisted citation list from the backend instead of guessing", () => {
    const started = applyChatEvent(null, { type: "started", message: base });
    const withSources = applyChatEvent(started, {
      type: "sources",
      sources: [
        { id: "S1", title: "v11", url: null, gameId: null },
        { id: "S2", title: "v999", url: null, gameId: null },
      ],
    });
    expect(withSources?.sources.map((source) => source.id)).toEqual(["S1", "S2"]);
  });

  it("ignores events that arrive before the started message", () => {
    expect(applyChatEvent(null, { type: "delta", text: "orphan" })).toBeNull();
  });
});

describe("interrupted answers", () => {
  it("marks a stopped answer with partial text as incomplete", () => {
    renderMessage({ ...base, status: "cancelled", content: "She heads to the" });
    expect(screen.getByText(/incomplete/i)).toBeInTheDocument();
  });

  it("says nothing was written when a stop lands before the first delta", () => {
    renderMessage({ ...base, status: "cancelled", content: "" });
    expect(screen.getByText(/before any answer/i)).toBeInTheDocument();
  });

  it("does not claim incompleteness for a finished answer", () => {
    renderMessage({ ...base, status: "complete", content: "Done." });
    expect(screen.queryByText(/incomplete/i)).toBeNull();
  });

  it("flags the partial text left behind by a failed request", () => {
    renderMessage({
      ...base,
      status: "error",
      content: "Partial answer",
      error: "provider_offline",
    });
    expect(screen.getByText(/incomplete/i)).toBeInTheDocument();
  });
});

describe("source links", () => {
  it("only renders http(s) links without embedded credentials", () => {
    expect(safeSourceUrl("https://vndb.org/v11")).toBe("https://vndb.org/v11");
    expect(safeSourceUrl("javascript:alert(1)")).toBeNull();
    expect(safeSourceUrl("https://user:pass@example.com")).toBeNull();
    expect(safeSourceUrl("file:///C:/secret.txt")).toBeNull();
    expect(safeSourceUrl(null)).toBeNull();
  });
});
