import { useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { api, IpcError } from "@/lib/ipc/client";

/**
 * One line above the library asking to install a newer build.
 *
 * It appears only when a check found something newer, and only on a platform that
 * can install it: a button that downloads a file it cannot run is worse than no
 * button. The Settings card below is the same answer with the detail, for anyone
 * who wants to read the version numbers or check on demand.
 */
export function UpdateBanner() {
  const { t } = useI18n();
  const update = useAppStore((state) => state.update);
  const pushToast = useAppStore((state) => state.pushToast);
  const [busy, setBusy] = useState(false);

  if (!update?.available || !update.canInstall) return null;

  const install = async (): Promise<void> => {
    setBusy(true);
    try {
      const path = await api.updateInstall();
      pushToast("success", t("update.started", { path }));
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("common.error"),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="update-banner" role="status">
      <span>
        {t("update.available", { version: update.latest ?? "" })}
      </span>
      <Button variant="primary" onClick={() => void install()} disabled={busy}>
        {busy ? t("update.downloading") : t("update.install")}
      </Button>
    </div>
  );
}

/**
 * The update section of Settings: the version this build is, the newest published
 * one, and a way to fetch it.
 */
export function UpdatePanel() {
  const { t } = useI18n();
  const update = useAppStore((state) => state.update);
  const setUpdate = useAppStore((state) => state.setUpdate);
  const pushToast = useAppStore((state) => state.pushToast);
  const [busy, setBusy] = useState(false);

  const check = async (force: boolean): Promise<void> => {
    setBusy(true);
    try {
      setUpdate(await api.updateCheck(force));
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("common.error"),
      );
    } finally {
      setBusy(false);
    }
  };

  const install = async (): Promise<void> => {
    setBusy(true);
    try {
      const path = await api.updateInstall();
      pushToast("success", t("update.started", { path }));
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("common.error"),
      );
    } finally {
      setBusy(false);
    }
  };

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
            {busy ? t("update.checking") : t("update.check")}
          </Button>
          {update?.available && update.canInstall ? (
            <Button variant="primary" onClick={() => void install()} disabled={busy}>
              {t("update.install")}
            </Button>
          ) : null}
          {update?.available && !update.canInstall && update.url ? (
            <Button
              onClick={() => void api.openExternal(update.url ?? "")}
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
