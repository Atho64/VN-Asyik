import { useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { api, IpcError } from "@/lib/ipc/client";
import type { UiPrefsPatch, WindowLaunchMode, WindowLaunchView } from "@/lib/ipc/types";
import { toSavedLibraryBrowse, type SortKey } from "@/lib/library";

const SORT_OPTIONS: Array<{ value: SortKey; labelKey: string }> = [
  { value: "recent", labelKey: "library.sortRecent" },
  { value: "title", labelKey: "library.sortTitle" },
  { value: "rating", labelKey: "library.sortRating" },
  { value: "playtime", labelKey: "library.sortPlaytime" },
  { value: "started", labelKey: "library.sortStarted" },
];

/** Library startup filter/sort preferences inside Settings -> General. */
export function LibraryLaunchSection() {
  const { t } = useI18n();
  const uiPrefs = useAppStore((state) => state.uiPrefs);
  const setUiPrefs = useAppStore((state) => state.setUiPrefs);
  const libraryBrowse = useAppStore((state) => state.libraryBrowse);
  const updateLibraryBrowse = useAppStore((state) => state.updateLibraryBrowse);
  const pushToast = useAppStore((state) => state.pushToast);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (uiPrefs || typeof api.uiPrefsGet !== "function") return;
    let cancelled = false;
    void api
      .uiPrefsGet()
      .then((loaded) => {
        if (!cancelled) setUiPrefs(loaded);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [uiPrefs, setUiPrefs]);

  const remember = uiPrefs?.library_remember_browse ?? false;
  const defaultSort: SortKey = uiPrefs?.library_default_sort ?? "recent";

  const applyPatch = async (patch: UiPrefsPatch): Promise<void> => {
    if (typeof api.uiPrefsSet !== "function") return;
    setSaving(true);
    try {
      const next = await api.uiPrefsSet(patch);
      setUiPrefs(next);
      if (patch.library_default_sort && !next.library_remember_browse) {
        updateLibraryBrowse({ sort: patch.library_default_sort });
      }
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="settings__group">
      <h2 className="settings__group-title">{t("settings.libraryTitle")}</h2>

      <div className="settings__row">
        <span className="settings__label">
          {t("settings.libraryRememberLabel")}
          <span className="settings__hint">{t("settings.libraryRememberHint")}</span>
        </span>
        <div className="settings__control">
          <Chip
            selected={!remember}
            disabled={saving}
            onClick={() => void applyPatch({ library_remember_browse: false })}
          >
            {t("settings.libraryRememberOff")}
          </Chip>
          <Chip
            selected={remember}
            disabled={saving}
            onClick={() =>
              void applyPatch({
                library_remember_browse: true,
                library_saved_browse: toSavedLibraryBrowse(libraryBrowse),
              })
            }
          >
            {t("settings.libraryRememberOn")}
          </Chip>
        </div>
      </div>

      <div className="settings__row">
        <span className="settings__label">
          {t("settings.libraryDefaultSortLabel")}
          <span className="settings__hint">{t("settings.libraryDefaultSortHint")}</span>
        </span>
        <div className="settings__control">
          <select
            className="select"
            aria-label={t("settings.libraryDefaultSortLabel")}
            disabled={saving}
            value={defaultSort}
            onChange={(event) =>
              void applyPatch({
                library_default_sort: event.target.value as SortKey,
              })
            }
          >
            {SORT_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>
                {t(option.labelKey)}
              </option>
            ))}
          </select>
        </div>
      </div>
    </section>
  );
}

/** Desktop window launch positioning controls inside Settings -> General. */
export function WindowLaunchSection() {
  const { t } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const [view, setView] = useState<WindowLaunchView | null>(null);
  const [xInput, setXInput] = useState("");
  const [yInput, setYInput] = useState("");
  const [busy, setBusy] = useState(false);

  const syncInputs = (next: WindowLaunchView): void => {
    setView(next);
    const displayX = next.x ?? next.current_x;
    const displayY = next.y ?? next.current_y;
    setXInput(displayX !== null && displayX !== undefined ? String(displayX) : "");
    setYInput(displayY !== null && displayY !== undefined ? String(displayY) : "");
  };

  useEffect(() => {
    if (typeof api.windowLaunchGet !== "function") return;
    let cancelled = false;
    void api
      .windowLaunchGet()
      .then((loaded) => {
        if (!cancelled) syncInputs(loaded);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  if (view && !view.supported) {
    return null;
  }

  const mode: WindowLaunchMode = view?.mode ?? "centered";

  const parseIntInput = (raw: string): number | null => {
    const trimmed = raw.trim();
    if (!trimmed) return null;
    const num = Number(trimmed);
    return Number.isInteger(num) ? num : null;
  };

  const setMode = async (nextMode: WindowLaunchMode, applyNow = false): Promise<void> => {
    if (typeof api.windowLaunchSet !== "function") return;
    setBusy(true);
    try {
      const next = await api.windowLaunchSet(
        nextMode === "customized"
          ? {
              mode: nextMode,
              x: parseIntInput(xInput),
              y: parseIntInput(yInput),
              apply_now: applyNow,
            }
          : {
              mode: nextMode,
              apply_now: applyNow,
            },
      );
      syncInputs(next);
      if (applyNow) pushToast("success", t("settings.windowApplied"));
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  const captureCurrent = async (): Promise<void> => {
    if (typeof api.windowLaunchCapture !== "function") return;
    setBusy(true);
    try {
      const next = await api.windowLaunchCapture();
      syncInputs(next);
      pushToast("success", t("settings.windowApplied"));
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="settings__group">
      <h2 className="settings__group-title">{t("settings.windowTitle")}</h2>

      <div className="settings__row">
        <span className="settings__label">
          {t("settings.windowPositionLabel")}
          <span className="settings__hint">{t("settings.windowPositionHint")}</span>
        </span>
        <div className="settings__control">
          <Chip
            selected={mode === "centered"}
            disabled={busy}
            onClick={() => void setMode("centered", false)}
          >
            {t("settings.windowModeCentered")}
          </Chip>
          <Chip
            selected={mode === "customized"}
            disabled={busy}
            onClick={() => void setMode("customized", false)}
          >
            {t("settings.windowModeCustomized")}
          </Chip>
          {mode === "centered" ? (
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => void setMode("centered", true)}
            >
              {t("settings.windowCenterNow")}
            </Button>
          ) : null}
        </div>
      </div>

      {mode === "customized" ? (
        <div className="settings__row">
          <span className="settings__label">
            {t("settings.windowCoordsLabel")}
            <span className="settings__hint">{t("settings.windowCoordsHint")}</span>
          </span>
          <div className="settings__control">
            <input
              className="input"
              type="number"
              style={{ width: "var(--input-number-width)" }}
              value={xInput}
              onChange={(event) => setXInput(event.target.value)}
              aria-label={t("settings.windowPosX")}
              disabled={busy}
            />
            <input
              className="input"
              type="number"
              style={{ width: "var(--input-number-width)" }}
              value={yInput}
              onChange={(event) => setYInput(event.target.value)}
              aria-label={t("settings.windowPosY")}
              disabled={busy}
            />
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => void captureCurrent()}
            >
              {t("settings.windowUseCurrent")}
            </Button>
            <Button
              variant="secondary"
              disabled={busy || parseIntInput(xInput) === null || parseIntInput(yInput) === null}
              onClick={() => void setMode("customized", true)}
            >
              {t("settings.windowApply")}
            </Button>
          </div>
        </div>
      ) : null}
    </section>
  );
}
