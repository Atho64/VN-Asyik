import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { api, IpcError } from "@/lib/ipc/client";
import type { AppearanceView } from "@/lib/ipc/types";

/** Accents that read well on both themes, offered as one-click choices. */
const ACCENTS = [
  { hex: "#65d8c5", key: "appearance.accentTeal" },
  { hex: "#8fb4ff", key: "appearance.accentBlue" },
  { hex: "#c79bff", key: "appearance.accentViolet" },
  { hex: "#ffb27a", key: "appearance.accentAmber" },
  { hex: "#ff8fa3", key: "appearance.accentRose" },
  { hex: "#9ee493", key: "appearance.accentGreen" },
] as const;

/**
 * Appearance: the accent colour and the background image.
 *
 * Both are stored on the Rust side and applied by `AppProvider`, so they survive a
 * reload and apply before the first screen paints. Nothing here is a theme mode:
 * dark/light/system stays where it was, in the Appearance section above.
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

  return (
    <Card>
      <CardHeader title={t("appearance.title")} />
      <p className="card__intro">{t("appearance.intro")}</p>

      <div className="settings__row settings__row--top">
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
              value={appearance?.accent ?? "#65d8c5"}
              onChange={(event) => void apply({ accent: event.target.value })}
              disabled={busy}
              aria-label={t("appearance.accentCustom")}
            />
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
    </Card>
  );
}

function describe(code: string, message: string, t: (key: string) => string): string {
  const key = `appearance.error.${code}`;
  const translated = t(key);
  return translated === key ? message : translated;
}

export type { AppearanceView };
