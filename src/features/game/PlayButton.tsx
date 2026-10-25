import { useCallback, useEffect, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { useAppStore } from "@/app/store";
import { Icon } from "@/components/Icon";
import { api, IpcError } from "@/lib/ipc/client";
import type {
  GameExecutable,
  LaunchCapability,
  LaunchOutcome,
} from "@/lib/ipc/types";

/**
 * The one primary action on the game screen.
 *
 * Sized and coloured as the thing to press, not as another chip in a row: it is
 * the only control here that starts something. It names what will happen before
 * it happens -- a Japanese-locale badge means the game will start through Locale
 * Emulator, a runner badge names the emulator/runner, and no badge means direct.
 */
export function PlayButton({
  gameId,
  mappedExecutable,
}: {
  gameId: string;
  mappedExecutable?: GameExecutable | null;
}) {
  const { t } = useI18n();
  const isAndroid = useAppStore((state) => state.platform?.os === "android");
  const [capability, setCapability] = useState<LaunchCapability | null>(null);
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<LaunchOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    let active = true;
    void api
      .launchCapability()
      .then((next) => {
        if (active) setCapability(next);
      })
      .catch(() => {
        // Launching is optional; a failure here must not break the screen.
        if (active) setCapability(null);
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => load(), [load]);

  async function start() {
    setBusy(true);
    setError(null);
    setOutcome(null);
    try {
      setOutcome(await api.launchGame(gameId));
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  }

  // No launch support: show the reason rather than a button that cannot work.
  if (!capability || !capability.canLaunch) {
    return capability?.note ? <p className="settings__hint">{capability.note}</p> : null;
  }

  const runnerKind = mappedExecutable?.runner_kind;
  const isEmulatorRunner =
    runnerKind !== undefined &&
    runnerKind !== "native" &&
    runnerKind !== "custom" &&
    runnerKind !== "android_apk" &&
    runnerKind !== "native_apk";

  return (
    <div className="play">
      <button
        type="button"
        className="play__button"
        onClick={() => void start()}
        disabled={busy}
      >
        <span className="play__icon" aria-hidden="true">
          <Icon name="play" size={18} />
        </span>
        <span className="play__label">
          {busy
            ? t("play.starting")
            : isEmulatorRunner
              ? t("play.playVia", { runner: runnerKind.toUpperCase() })
              : t("play.play")}
        </span>
      </button>

      {isAndroid ? (
        <span className="settings__hint">{t("play.androidHint")}</span>
      ) : isEmulatorRunner ? (
        <span
          className="play__badge"
          title={mappedExecutable?.exe_path ?? undefined}
        >
          {runnerKind.toUpperCase()}
          {mappedExecutable?.launch_target
            ? ` · ${mappedExecutable.launch_target}`
            : ""}
        </span>
      ) : capability.emulator === "ready" ? (
        // The only state that actually starts a game in Japanese. Reading
        // "the launcher exists on disk" here showed the badge with no profile
        // chosen, where the game starts directly.
        <span className="play__badge" title={capability.emulatorPath ?? undefined}>
          {t("play.japaneseLocale")}
        </span>
      ) : capability.emulator === "noProfile" ? (
        <span className="settings__hint">{t("play.profileNone")}</span>
      ) : capability.emulator === "disabled" ? (
        <span className="settings__hint">{t("play.disabledHint")}</span>
      ) : (
        <span className="settings__hint">{t("play.noEmulator")}</span>
      )}

      {error && <p className="notice notice--danger">{error}</p>}
      {outcome && <p className="settings__hint">{outcome.explanation}</p>}
    </div>
  );
}
