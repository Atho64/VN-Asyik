/**
 * Shell-level achievement events: one unlock-toast subscriber and one coalesced
 * refresh listener. Both live here so the wiring is testable without the shell,
 * and a burst of mutations evaluates the catalog once, not once per edit.
 */

import { useEffect } from "react";

import { useAppStore, type ToastKind } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import type { Translator } from "@/i18n";
import {
  ACHIEVEMENTS_REFRESH_EVENT,
  achievementsEvaluate,
  listenAchievementsUnlocked,
} from "@/lib/ipc/achievements";

/**
 * Build one toast for a batch of newly unlocked slugs. A single unlock names the
 * achievement; a batch (the retroactive first scan can unlock many at once) gets
 * one summary naming a few, never one toast per slug.
 */
export function buildUnlockToast(
  slugs: string[],
  t: Translator,
): { kind: ToastKind; message: string } {
  if (slugs.length === 1) {
    return {
      kind: "success",
      message: t("achievements.toastUnlocked", {
        name: t(`achievements.names.${slugs[0]}`),
      }),
    };
  }
  const names = slugs
    .slice(0, 3)
    .map((slug) => t(`achievements.names.${slug}`))
    .join(", ");
  return {
    kind: "success",
    message: t("achievements.toastUnlockedMany", { count: slugs.length, names }),
  };
}

/** Push a batched success toast whenever the backend reports new unlocks. */
export function useAchievementUnlockToasts(): void {
  const { t } = useI18n();
  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void listenAchievementsUnlocked((slugs) => {
      const { kind, message } = buildUnlockToast(slugs, t);
      useAppStore.getState().pushToast(kind, message);
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stop = unlisten;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stop?.();
    };
  }, [t]);
}

/**
 * Evaluate the catalog once per burst of `vn:achievements-refresh` events. The
 * timeout coalesces dispatches that land in the same tick; evaluate is never
 * called per render, only when a mutation asks for it.
 */
export function useAchievementsRefresh(): void {
  useEffect(() => {
    let pending = false;
    let timer = 0;
    const onRefresh = (): void => {
      if (pending) return;
      pending = true;
      timer = window.setTimeout(() => {
        pending = false;
        void achievementsEvaluate().catch(() => undefined);
      }, 0);
    };
    window.addEventListener(ACHIEVEMENTS_REFRESH_EVENT, onRefresh);
    return () => {
      window.removeEventListener(ACHIEVEMENTS_REFRESH_EVENT, onRefresh);
      window.clearTimeout(timer);
    };
  }, []);
}