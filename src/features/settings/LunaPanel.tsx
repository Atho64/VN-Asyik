import { useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { lunaApi } from "@/lib/ipc/luna";
import { hookApi } from "@/lib/ipc/hook";
import type { HookStatusView } from "@/lib/ipc/hook";
import { IpcError } from "@/lib/ipc/client";
import { formatDateTime } from "@/lib/time";
import type { LunaSettings, LunaStatus } from "@/lib/ipc/luna";

/** How many captured lines to preview. Enough to prove text is flowing. */
const PREVIEW_LINES = 3;

/**
 * The `LunaTranslator` live-text source.
 *
 * Feeding the companion chat what the player is reading needs no hook of ours:
 * `LunaTranslator` already extracts text, and its network service republishes
 * it on a local websocket. This card only points at that address. The panel
 * says what to enable in `LunaTranslator` itself, because the honest setup step
 * ("turn on 网络服务 there") is the one thing this app cannot do for the user.
 */
export function LunaPanel() {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);

  const [settings, setSettings] = useState<LunaSettings | null>(null);
  const [baseUrl, setBaseUrl] = useState("");
  const [status, setStatus] = useState<LunaStatus | null>(null);
  // The live ring the companion chat reads, whichever source feeds it. Showing
  // it here is the difference between "Connected" and knowing text arrives.
  const [ring, setRing] = useState<HookStatusView | null>(null);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [probe, setProbe] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void lunaApi
      .settings()
      .then((next) => {
        if (!active) return;
        setSettings(next);
        setBaseUrl(next.baseUrl);
      })
      .catch((caught: unknown) => {
        if (active) {
          setError(caught instanceof IpcError ? caught.message : t("common.error"));
        }
      });
    const refresh = (): void => {
      void lunaApi
        .status()
        .then((next) => {
          if (active) setStatus(next);
        })
        .catch(() => undefined);
      void hookApi
        .status()
        .then((next) => {
          if (active) setRing(next);
        })
        .catch(() => undefined);
    };
    refresh();
    const interval = window.setInterval(refresh, 5000);
    return () => {
      active = false;
      window.clearInterval(interval);
    };
  }, [t]);

  if (!settings) return null;

  const save = async (next: LunaSettings): Promise<boolean> => {
    setBusy(true);
    setError(null);
    try {
      const saved = await lunaApi.save(next);
      setSettings(saved);
      setBaseUrl(saved.baseUrl);
      return true;
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
      return false;
    } finally {
      setBusy(false);
    }
  };

  // The toggle saves the draft address along with the switch, so enabling a
  // freshly typed port does not connect to the previously saved one.
  const toggle = async (): Promise<void> => {
    const saved = await save({ enabled: !settings.enabled, baseUrl: baseUrl.trim() });
    if (saved) pushToast("success", t("luna.saved"));
  };

  const test = async (): Promise<void> => {
    setTesting(true);
    setProbe(null);
    try {
      const candidate = baseUrl.trim();
      const result = await lunaApi.test(candidate.length > 0 ? candidate : undefined);
      setProbe(result.ok ? t("luna.testOk") : t("luna.testFail", { detail: result.detail }));
    } catch (caught) {
      setProbe(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setTesting(false);
    }
  };

  const statusLabel = !status || !status.enabled ? t("luna.statusDisabled") : status.connected ? t("luna.statusConnected") : t("luna.statusDisconnected");

  return (
    <Card>
      <CardHeader title={t("luna.title")} />
      <p className="card__intro">{t("luna.intro")}</p>

      <div className="field">
        <span className="field__label">{t("luna.enable")}</span>
        <div className="settings__control">
          <Chip
            selected={settings.enabled}
            onClick={() => void toggle()}
            ariaLabel={t("luna.enable")}
          >
            {settings.enabled ? t("common.enabled") : t("common.disabled")}
          </Chip>
          <span className="settings__hint" role="status">
            {statusLabel}
          </span>
        </div>
      </div>

      <label className="field" style={{ marginTop: "var(--space-3)" }}>
        <span className="field__label">{t("luna.baseUrl")}</span>
        <input
          className="input"
          value={baseUrl}
          onChange={(event) => setBaseUrl(event.target.value)}
          spellCheck={false}
          placeholder="http://127.0.0.1:2333"
        />
        <span className="field__hint">{t("luna.baseUrlHint")}</span>
      </label>

      <div style={{ display: "flex", gap: "var(--space-2)", marginTop: "var(--space-3)" }}>
        <Button
          variant="primary"
          disabled={busy}
          onClick={() => {
            void (async () => {
              if (await save({ enabled: settings.enabled, baseUrl: baseUrl.trim() })) {
                pushToast("success", t("luna.saved"));
              }
            })();
          }}
        >
          {busy ? t("chat.saving") : t("luna.save")}
        </Button>
        <Button disabled={testing} onClick={() => void test()}>
          {testing ? t("luna.testing") : t("luna.test")}
        </Button>
      </div>

      {probe ? (
        <p className="field__hint" role="status">
          {probe}
        </p>
      ) : null}

      {status?.lastLineAt ? (
        <p className="field__hint">
          {t("luna.lastText", { time: formatDateTime(status.lastLineAt, locale) })}
        </p>
      ) : null}
      {status?.lastError ? (
        <p className="field__hint">{t("luna.lastError", { error: status.lastError })}</p>
      ) : null}

      {ring && ring.lines.length > 0 ? (
        <div className="luna-preview">
          <p className="field__hint">{t("luna.recent")}</p>
          <ul className="luna-preview__lines">
            {ring.lines.slice(-PREVIEW_LINES).map((line, index) => (
              <li key={`${line.at}-${index}`} className="luna-preview__line">
                {line.text}
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {error ? (
        <p className="field__error" role="alert">
          {error}
        </p>
      ) : null}
    </Card>
  );
}
