import { useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { api, IpcError } from "@/lib/ipc/client";

type BusyAction = "check" | "install" | "open" | null;

function useUpdateActions() {
  const { t } = useI18n();
  const update = useAppStore((state) => state.update);
  const setUpdate = useAppStore((state) => state.setUpdate);
  const pushToast = useAppStore((state) => state.pushToast);
  const [busyAction, setBusyAction] = useState<BusyAction>(null);

  const runAction = async (
    action: Exclude<BusyAction, null>,
    work: () => Promise<void>,
  ): Promise<void> => {
    setBusyAction(action);
    try {
      await work();
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("common.error"),
      );
    } finally {
      setBusyAction(null);
    }
  };

  const check = (force: boolean): Promise<void> =>
    runAction("check", async () => {
      setUpdate(await api.updateCheck(force));
    });

  const install = (): Promise<void> =>
    runAction("install", async () => {
      const path = await api.updateInstall();
      pushToast("success", t("update.started", { path }));
    });

  const openPage = (url: string): Promise<void> =>
    runAction("open", async () => {
      await api.openExternal(url);
    });

  return { t, update, busyAction, busy: busyAction !== null, check, install, openPage };
}

/**
 * One line above the library asking to update when a newer build exists.
 *
 * Self-installing builds (Windows) download and launch the setup executable
 * directly; other platforms (Android, Linux, macOS) open the GitHub release
 * page in the user's browser instead of downloading in-app.
 */
export function UpdateBanner() {
  const { t, update, busyAction, busy, install, openPage } = useUpdateActions();

  if (!update?.available) return null;
  const releaseUrl = update.url;
  if (!update.canInstall && !releaseUrl) return null;

  const canSelfInstall = update.canInstall;
  const message = canSelfInstall
    ? t("update.available", { version: update.latest ?? "" })
    : t("update.behind", { version: update.latest ?? "" });
  const label = canSelfInstall
    ? busyAction === "install"
      ? t("update.downloading")
      : t("update.install")
    : t("update.openPage");

  const handleAction = (): void => {
    if (canSelfInstall) {
      void install();
    } else if (releaseUrl) {
      void openPage(releaseUrl);
    }
  };

  return (
    <div className="update-banner" role="status">
      <span>{message}</span>
      <Button variant="primary" onClick={handleAction} disabled={busy}>
        {label}
      </Button>
    </div>
  );
}

/**
 * The update section of Settings: the version this build is, the newest published
 * one, and a way to fetch it.
 */
export function UpdatePanel() {
  const { t, update, busyAction, busy, check, install, openPage } = useUpdateActions();

  /** One sentence for the row: what the last answer said. */
  const status = (): string => {
    if (!update) return t("update.checking");
    if (update.error) {
      const key = `update.error.${update.error}`;
      const translated = t(key);
      return translated === key ? t("update.error.unknown") : translated;
    }
    if (update.available) return t("update.behind", { version: update.latest ?? "" });
    return t("update.current");
  };

  const releaseUrl = update?.url;

  return (
    <Card>
      <CardHeader title={t("update.title")} />
      <p className="card__intro">{t("update.intro")}</p>

      <div className="settings__row">
        <div className="settings__label">
          <span>{t("update.installed")}</span>
          <span className="settings__hint">{status()}</span>
        </div>
        <div className="settings__control">
          <Button onClick={() => void check(true)} disabled={busy}>
            {busyAction === "check" ? t("update.checking") : t("update.check")}
          </Button>
          {update?.available && update.canInstall ? (
            <Button variant="primary" onClick={() => void install()} disabled={busy}>
              {busyAction === "install" ? t("update.downloading") : t("update.install")}
            </Button>
          ) : null}
          {update?.available && !update.canInstall && releaseUrl ? (
            <Button
              variant="primary"
              onClick={() => void openPage(releaseUrl)}
              disabled={busy}
            >
              {t("update.openPage")}
            </Button>
          ) : null}
        </div>
      </div>

      {update?.latest ? (
        <p className="settings__hint">
          {t("update.versions", { current: update.current, latest: update.latest })}
        </p>
      ) : null}
    </Card>
  );
}
