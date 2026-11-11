import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { api, IpcError } from "@/lib/ipc/client";
import type { AppearanceView } from "@/lib/ipc/types";

/** Accents that read well on both themes, offered as one-click choices. */
const ACCENTS = [
  { hex: "#65d8c5", key: "appearance.accentTeal" },
  { hex: "#8fb4ff", key: "appearance.accentBlue" },
  { hex: "#7ad9e8", key: "appearance.accentCyan" },
  { hex: "#c79bff", key: "appearance.accentViolet" },
  { hex: "#ff8fa3", key: "appearance.accentRose" },
  { hex: "#ffb27a", key: "appearance.accentAmber" },
  { hex: "#ffd166", key: "appearance.accentGold" },
  { hex: "#9ee493", key: "appearance.accentGreen" },
] as const;

const DEFAULT_ACCENT = "#65d8c5";

/**
 * Appearance: the accent colour and the background image, as rows inside the
 * Settings appearance group (theme mode and effects sit above them).
 *
 * Both are stored on the Rust side and applied by `AppProvider`, so they survive a
 * reload and apply before the first screen paints. The chosen hex is kept exactly
 * as picked; the provider adapts it to the active theme so it stays readable.
 */
export function AppearancePanel() {
  const { t } = useI18n();
  const appearance = useAppStore((state) => state.appearance);
  const setAppearance = useAppStore((state) => state.setAppearance);
  const pushToast = useAppStore((state) => state.pushToast);
  const [busy, setBusy] = useState(false);

  const apply = async (
    change: {
      accent?: string | null;
      backgroundPath?: string | null;
      clearBackground?: boolean;
      backgroundDim?: number;
    },
  ): Promise<void> => {
    setBusy(true);
    try {
      setAppearance(
        await api.appearanceSet({
          accent: change.accent === undefined ? (appearance?.accent ?? null) : change.accent,
          backgroundPath: change.backgroundPath ?? null,
          clearBackground: change.clearBackground ?? false,
          backgroundDim: change.backgroundDim ?? appearance?.backgroundDim ?? 0.45,
        }),
      );
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? describe(caught.code, caught.message, t) : t("common.error"),
      );
    } finally {
      setBusy(false);
    }
  };

  const chooseBackground = async (): Promise<void> => {
    const picked = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Image", extensions: ["jpg", "jpeg", "png", "webp", "avif", "gif"] }],
    });
    if (typeof picked === "string") {
      await apply({ backgroundPath: picked });
    }
  };

  const currentAccent = appearance?.accent ?? DEFAULT_ACCENT;

  return (
    <>
      <p className="settings__hint">{t("appearance.intro")}</p>

      <div className="settings__row settings__row--stack">
        <div className="settings__label">
          <span>{t("appearance.accent")}</span>
          <span className="settings__hint">{t("appearance.accentHint")}</span>
        </div>
        <div className="settings__control appearance__accents">
          {ACCENTS.map((accent) => (
            <Chip
              key={accent.hex}
              selected={appearance?.accent === accent.hex}
              onClick={() => void apply({ accent: accent.hex })}
            >
              <span
                className="appearance__swatch"
                style={{ backgroundColor: accent.hex }}
                aria-hidden
              />
              {t(accent.key)}
            </Chip>
          ))}
          <label className="appearance__picker">
            <span className="settings__hint">{t("appearance.accentCustom")}</span>
            <input
              type="color"
              className="input appearance__color"
              value={currentAccent}
              onChange={(event) => void apply({ accent: event.target.value })}
              disabled={busy}
              aria-label={t("appearance.accentCustom")}
            />
            <code className="appearance__hex">{currentAccent}</code>
          </label>
          {appearance?.accent ? (
            <Button onClick={() => void apply({ accent: null })} disabled={busy}>
              {t("appearance.reset")}
            </Button>
          ) : null}
        </div>
      </div>

      <div className="settings__row settings__row--top">
        <div className="settings__label">
          <span>{t("appearance.background")}</span>
          <span className="settings__hint">
            {appearance?.backgroundName ?? t("appearance.backgroundNone")}
          </span>
        </div>
        <div className="settings__control">
          <Button onClick={() => void chooseBackground()} disabled={busy}>
            {t("appearance.chooseImage")}
          </Button>
          {appearance?.background ? (
            <Button
              onClick={() => void apply({ clearBackground: true })}
              disabled={busy}
            >
              {t("appearance.clearImage")}
            </Button>
          ) : null}
        </div>
      </div>

      {appearance?.background ? (
        <label className="field">
          <span className="field__label">{t("appearance.dim")}</span>
          <input
            type="range"
            min={0}
            max={0.9}
            step={0.05}
            value={appearance.backgroundDim}
            onChange={(event) =>
              // Optimistic: a slider that only moves after a round trip feels
              // broken. The stored value arrives with the answer.
              setAppearance({ ...appearance, backgroundDim: Number(event.target.value) })
            }
            onPointerUp={(event) =>
              void apply({ backgroundDim: Number(event.currentTarget.value) })
            }
            disabled={busy}
          />
          <span className="field__hint">{t("appearance.dimHint")}</span>
        </label>
      ) : null}
    </>
  );
}

function describe(code: string, message: string, t: (key: string) => string): string {
  const key = `appearance.error.${code}`;
  const translated = t(key);
  return translated === key ? message : translated;
}

export type { AppearanceView };
