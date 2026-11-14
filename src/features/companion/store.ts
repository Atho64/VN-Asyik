import { create } from "zustand";
import { IpcError } from "@/lib/ipc/client";
import { companionApi, type CompanionSnapshot } from "@/lib/ipc/companion";

interface CompanionUi {
  snapshot: CompanionSnapshot | null;
  connected: boolean;
  connectionError: boolean;
  actionPending: boolean;
  error: string | null;
  draft: string;
  submitted: { text: string; requestId: string; sessionId: string } | null;
}

export const useCompanionStore = create<CompanionUi>(() => ({
  snapshot: null,
  connected: false,
  connectionError: false,
  actionPending: false,
  error: null,
  draft: "",
  submitted: null,
}));

export function acceptSnapshot(next: CompanionSnapshot) {
  useCompanionStore.setState((previous) => {
    const old = previous.snapshot;
    if (old?.runtimeId === next.runtimeId && old.sequence > next.sequence)
      return previous;
    const begun =
      previous.submitted &&
      next.live &&
      next.requestId === previous.submitted.requestId &&
      next.session?.id === previous.submitted.sessionId;
    return {
      snapshot: next,
      connected: true,
      connectionError: false,
      draft: begun && previous.draft === previous.submitted?.text ? "" : previous.draft,
      submitted: begun || (!next.busy && next.error) ? null : previous.submitted,
    };
  });
}

export function failureCode(error: unknown): string {
  if (error instanceof IpcError) {
    return error.code === "companion" || error.code === "chat"
      ? error.message
      : error.code;
  }
  return "unknown";
}

export async function performAction(
  action: () => Promise<CompanionSnapshot>,
): Promise<boolean> {
  if (useCompanionStore.getState().actionPending) return false;
  useCompanionStore.setState({ actionPending: true, error: null });
  try {
    acceptSnapshot(await action());
    return true;
  } catch (error) {
    useCompanionStore.setState({ error: failureCode(error) });
    return false;
  } finally {
    useCompanionStore.setState({ actionPending: false });
  }
}

export async function sendQuestion(id: string): Promise<boolean> {
  const { draft, snapshot } = useCompanionStore.getState();
  const text = draft.trim();
  if (
    !text ||
    (snapshot?.busy && !snapshot.vision.automatic) ||
    useCompanionStore.getState().actionPending
  )
    return false;
  const requestId = crypto.randomUUID();
  useCompanionStore.setState({
    submitted: { text: draft, requestId, sessionId: id },
  });
  const accepted = await performAction(() => companionApi.send(id, requestId, text));
  if (!accepted) useCompanionStore.setState({ submitted: null });
  return accepted;
}

/**
 * Listen before requesting the first snapshot. Coalesce streaming invalidations;
 * an event arriving during a read schedules another read. Neither disconnect nor
 * cleanup invokes cancellation. Rust owns the session, including with zero views.
 */
export function connectCompanion(): () => void {
  let alive = true;
  let fetching = false;
  let dirty = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let unlisten: (() => void) | undefined;

  async function refresh() {
    if (!alive) return;
    if (fetching) {
      dirty = true;
      return;
    }
    fetching = true;
    try {
      const next = await companionApi.snapshot();
      if (alive) acceptSnapshot(next);
    } catch {
      if (alive) useCompanionStore.setState({ connected: false, connectionError: true });
    } finally {
      fetching = false;
      if (alive && dirty) {
        dirty = false;
        schedule();
      }
    }
  }

  function schedule() {
    if (!alive || timer) return;
    timer = setTimeout(() => {
      timer = undefined;
      void refresh();
    }, 80);
  }

  void (async () => {
    try {
      const dispose = await companionApi.listen((notice) => {
        const current = useCompanionStore.getState().snapshot;
        if (
          current?.runtimeId === notice.runtimeId &&
          current.sequence >= notice.sequence
        )
          return;
        schedule();
      });
      if (!alive) {
        dispose();
        return;
      }
      unlisten = dispose;
    } catch {
      // Snapshot polling remains usable if listener registration fails.
    }
    await refresh();
  })();
  // Also recover missed notifications after a suspended/minimized webview.
  const poll = setInterval(() => {
    if (document.visibilityState !== "hidden") void refresh();
  }, 2_000);
  const onReturn = () => {
    void refresh();
  };
  window.addEventListener("focus", onReturn);
  document.addEventListener("visibilitychange", onReturn);
  return () => {
    alive = false;
    clearInterval(poll);
    clearTimeout(timer);
    unlisten?.();
    window.removeEventListener("focus", onReturn);
    document.removeEventListener("visibilitychange", onReturn);
  };
}
