import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { companionSnapshot, companionMessage } from "@/test/companionFixture";
import type { CompanionNotice, CompanionSnapshot } from "@/lib/ipc/companion";

const mocks = vi.hoisted(() => ({
  snapshot: vi.fn(),
  listen: vi.fn(),
  send: vi.fn(),
  control: vi.fn(),
}));
vi.mock("@/lib/ipc/companion", async () => ({
  ...(await vi.importActual("@/lib/ipc/companion")),
  companionApi: mocks,
}));
const { acceptSnapshot, connectCompanion, sendQuestion, useCompanionStore } =
  await import("./store");

beforeEach(() => {
  vi.resetAllMocks();
  useCompanionStore.setState(useCompanionStore.getInitialState(), true);
});
afterEach(() => {
  vi.useRealTimers();
});

describe("ordered companion snapshots", () => {
  it("rejects an old read or acknowledgement after a newer event", () => {
    const current = { ...companionSnapshot(8), live: companionMessage(), busy: true };
    acceptSnapshot(current);
    acceptSnapshot(companionSnapshot(3));
    expect(useCompanionStore.getState().snapshot).toEqual(current);
    expect(useCompanionStore.getState().connected).toBe(true);
  });

  it("does not clear a draft until the backend actually begins its message", async () => {
    acceptSnapshot(companionSnapshot());
    useCompanionStore.setState({ draft: "My question" });
    mocks.send.mockResolvedValue({ ...companionSnapshot(2), busy: true });
    await sendQuestion("session-one");
    const requestId = mocks.send.mock.calls[0]![1] as string;
    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(useCompanionStore.getState().draft).toBe("My question");
    acceptSnapshot({
      ...companionSnapshot(3),
      requestId,
      busy: true,
      live: companionMessage(),
    });
    expect(useCompanionStore.getState().draft).toBe("");
    // A later edit is never eaten by an old Started event.
    useCompanionStore.setState({
      submitted: { text: "Old question", requestId, sessionId: "session-one" },
      draft: "Next question",
    });
    acceptSnapshot({
      ...companionSnapshot(4),
      requestId,
      busy: true,
      live: companionMessage(),
    });
    expect(useCompanionStore.getState().draft).toBe("Next question");
  });

  it("preserves the losing window's draft when another window starts a reply", async () => {
    acceptSnapshot(companionSnapshot());
    useCompanionStore.setState({ draft: "Question from this window" });
    let reject: ((error: Error) => void) | undefined;
    mocks.send.mockReturnValue(
      new Promise((_, fail) => {
        reject = fail;
      }),
    );
    const sending = sendQuestion("session-one");
    acceptSnapshot({
      ...companionSnapshot(3),
      requestId: "another-window-request",
      busy: true,
      live: companionMessage(),
    });
    expect(useCompanionStore.getState().draft).toBe("Question from this window");
    reject?.(new Error("chat_busy"));
    expect(await sending).toBe(false);
    expect(useCompanionStore.getState().draft).toBe("Question from this window");
    expect(useCompanionStore.getState().submitted).toBeNull();
  });

  it("leaves an unsent draft intact when preparation fails", async () => {
    acceptSnapshot(companionSnapshot());
    useCompanionStore.setState({ draft: "Keep this question" });
    mocks.send.mockRejectedValue(new Error("offline"));
    expect(await sendQuestion("session-one")).toBe(false);
    expect(useCompanionStore.getState().draft).toBe("Keep this question");
  });
});

describe("subscriber lifetime", () => {
  it("subscribes before reading, catches changes during a read, and never cancels on cleanup", async () => {
    vi.useFakeTimers();
    let receive: ((notice: CompanionNotice) => void) | undefined;
    const unlisten = vi.fn();
    mocks.listen.mockImplementation(async (callback: typeof receive) => {
      receive = callback;
      return unlisten;
    });
    let release: ((snapshot: CompanionSnapshot) => void) | undefined;
    mocks.snapshot.mockImplementationOnce(
      () =>
        new Promise<CompanionSnapshot>((resolve) => {
          release = resolve;
        }),
    );
    mocks.snapshot.mockResolvedValue(companionSnapshot(9));
    const disconnect = connectCompanion();
    await vi.advanceTimersByTimeAsync(0);
    expect(mocks.listen.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.snapshot.mock.invocationCallOrder[0]!,
    );
    receive?.({
      runtimeId: "runtime-one",
      sequence: 9,
      epoch: 2,
      sessionId: "session-one",
    });
    await vi.advanceTimersByTimeAsync(80);
    release?.(companionSnapshot(1));
    await vi.advanceTimersByTimeAsync(80);
    expect(useCompanionStore.getState().snapshot?.sequence).toBe(9);
    disconnect();
    expect(unlisten).toHaveBeenCalledOnce();
    expect(mocks.control).not.toHaveBeenCalled();
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("cleans up a listener that resolves after unmount", async () => {
    let register: ((dispose: () => void) => void) | undefined;
    mocks.listen.mockReturnValue(
      new Promise<() => void>((resolve) => {
        register = resolve;
      }),
    );
    const disconnect = connectCompanion();
    disconnect();
    const unlisten = vi.fn();
    register?.(unlisten);
    await Promise.resolve();
    await Promise.resolve();
    expect(unlisten).toHaveBeenCalledOnce();
    expect(mocks.snapshot).not.toHaveBeenCalled();
    expect(mocks.control).not.toHaveBeenCalled();
  });
});
