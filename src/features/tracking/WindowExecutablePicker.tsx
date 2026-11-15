import { useEffect, useRef, useState } from "react";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { api, IpcError } from "@/lib/ipc/client";
import type { ExecutableWindow } from "@/lib/ipc/types";

export function WindowExecutablePicker({
  gameId,
  onMapped,
}: {
  gameId: string;
  onMapped: (name: string) => Promise<void>;
}) {
  const { t } = useI18n();
  const [windows, setWindows] = useState<ExecutableWindow[]>([]);
  const [selected, setSelected] = useState("");
  const [loading, setLoading] = useState(true);
  const [linking, setLinking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const active = useRef(false);

  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void api
      .trackingExecutableWindows()
      .then((result) => {
        if (!cancelled) setWindows(result);
      })
      .catch((caught: unknown) => {
        if (!cancelled)
          setError(caught instanceof IpcError ? caught.message : t("common.error"));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [revision, t]);

  async function link() {
    if (!gameId || !selected) return;
    setLinking(true);
    setError(null);
    try {
      const mapping = await api.trackingMapWindow(gameId, selected);
      if (active.current) await onMapped(mapping.exe_name);
    } catch (caught) {
      if (active.current)
        setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      if (active.current) setLinking(false);
    }
  }

  const candidate = windows.find((window) => window.key === selected);
  return (
    <div className="map-exe__windows" aria-busy={loading || linking}>
      <p className="field__hint">{t("tracking.windowHint")}</p>
      <label className="field">
        <span className="field__label">{t("tracking.openWindows")}</span>
        <select
          className="select"
          value={selected}
          disabled={loading || linking || windows.length === 0}
          onChange={(event) => setSelected(event.target.value)}
        >
          <option value="">{t("tracking.selectWindow")}</option>
          {windows.map((window) => (
            <option key={window.key} value={window.key} disabled={!window.exePath}>
              {window.title}
              {window.exePath
                ? ` · ${window.exePath.split(/[\\/]/).at(-1)}`
                : ` · ${t("tracking.windowDenied")}`}
            </option>
          ))}
        </select>
      </label>
      {candidate?.exePath ? (
        <p className="map-exe__window-path">{candidate.exePath}</p>
      ) : null}
      {loading ? <p role="status">{t("tracking.loadingWindows")}</p> : null}
      {!loading && windows.length === 0 && !error ? (
        <p role="status">{t("tracking.noWindows")}</p>
      ) : null}
      {error ? <p role="alert">{error}</p> : null}
      {!gameId ? <p className="field__hint">{t("tracking.pickNeedsGame")}</p> : null}
      <div className="map-exe__choices">
        <Button
          variant="secondary"
          disabled={loading || linking}
          onClick={() => {
            setLoading(true);
            setError(null);
            setSelected("");
            setWindows([]);
            setRevision((value) => value + 1);
          }}
        >
          {t("tracking.refreshWindows")}
        </Button>
        <Button
          disabled={loading || linking || !gameId || !candidate?.exePath}
          onClick={() => void link()}
        >
          {linking ? t("tracking.linkingWindow") : t("tracking.linkWindow")}
        </Button>
      </div>
    </div>
  );
}
