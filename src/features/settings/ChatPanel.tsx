import { useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { api, IpcError } from "@/lib/ipc/client";
import type { ChatSettings } from "@/lib/ipc/types";

/**
 * The optional chat provider.
 *
 * Optional is the operative word: the taste profile works without this. The panel
 * says so, because a settings screen that implies the feature needs a key would
 * push users toward a paid API for something that runs locally for free.
 */
export function ChatPanel() {
  const { t } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);

  const [settings, setSettings] = useState<ChatSettings | null>(null);
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const next = await api.aiChatSettings();
        if (cancelled) return;
        setSettings(next);
        setBaseUrl(next.baseUrl);
        setModel(next.model);
      } catch (caught) {
        if (!cancelled) {
          setError(caught instanceof IpcError ? caught.message : t("common.error"));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [t]);

  const save = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      // An empty box means "leave the stored key alone", so changing the model
      // does not require re-entering it. Omitted rather than set to undefined,
      // because the payload type does not admit an explicit undefined.
      const trimmedKey = key.trim();
      const next = await api.aiSaveChatSettings(
        trimmedKey.length > 0
          ? { baseUrl, model, key: trimmedKey }
          : { baseUrl, model },
      );
      setSettings(next);
      setKey("");
      pushToast("success", t("chat.saved"));
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  const clearKey = async (): Promise<void> => {
    setBusy(true);
    try {
      setSettings(await api.aiClearChatKey());
      pushToast("success", t("chat.keyRemoved"));
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card>
      <CardHeader title={t("chat.title")} />
      <p className="card__intro">{t("chat.intro")}</p>

      {!settings?.supported ? (
        <p className="notice" role="status">
          {t("chat.unsupported")}
        </p>
      ) : (
        <>
          <label className="field">
            <span className="field__label">{t("chat.baseUrl")}</span>
            <input
              className="input"
              value={baseUrl}
              onChange={(event) => setBaseUrl(event.target.value)}
              spellCheck={false}
              placeholder="https://api.openai.com/v1"
            />
            <span className="field__hint">{t("chat.baseUrlHint")}</span>
          </label>

          <label className="field" style={{ marginTop: "var(--space-3)" }}>
            <span className="field__label">{t("chat.model")}</span>
            <input
              className="input"
              value={model}
              onChange={(event) => setModel(event.target.value)}
              spellCheck={false}
            />
          </label>

          <label className="field" style={{ marginTop: "var(--space-3)" }}>
            <span className="field__label">{t("chat.key")}</span>
            <input
              className="input"
              type="password"
              value={key}
              onChange={(event) => setKey(event.target.value)}
              spellCheck={false}
              autoComplete="off"
              placeholder={
                settings?.hasKey
                  ? (settings.keyHint ?? t("chat.keyStored"))
                  : t("chat.keyPlaceholder")
              }
            />
            <span className="field__hint">{t("chat.keyHint")}</span>
          </label>

          <div style={{ display: "flex", gap: "var(--space-2)", marginTop: "var(--space-3)" }}>
            <Button variant="primary" onClick={() => void save()} disabled={busy}>
              {busy ? t("chat.saving") : t("chat.save")}
            </Button>
            {settings?.hasKey ? (
              <Button onClick={() => void clearKey()} disabled={busy}>
                {t("chat.removeKey")}
              </Button>
            ) : null}
          </div>

          <div style={{ marginTop: "var(--space-2)" }}>
            <Chip selected={settings.hasKey}>
              {settings.hasKey ? t("chat.keySet") : t("chat.noKey")}
            </Chip>
          </div>
        </>
      )}

      {error ? (
        <p className="field__error" role="alert">
          {error}
        </p>
      ) : null}
    </Card>
  );
}
