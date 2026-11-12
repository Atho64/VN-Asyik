import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { resolveTheme, useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { api, IpcError } from "@/lib/ipc/client";
import type { AppearanceView, ThemeView } from "@/lib/ipc/types";
import {
  DEFAULT_THEME_TOKENS,
  THEME_PRESETS,
  readableTokens,
  themeContrast,
  type Theme,
  type ThemeTokens,
} from "@/lib/theme";

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

/** The six colours a theme owns, in the order the editor shows them. */
const COLOUR_TOKENS = [
  { key: "bg", label: "appearance.tokenBg" },
  { key: "bgElevated", label: "appearance.tokenBgElevated" },
  { key: "surfaceSolid", label: "appearance.tokenSurfaceSolid" },
  { key: "text", label: "appearance.tokenText" },
  { key: "textMuted", label: "appearance.tokenTextMuted" },
  { key: "textFaint", label: "appearance.tokenTextFaint" },
] as const satisfies ReadonlyArray<{ key: keyof ThemeTokens; label: string }>;

const RADIUS = [
  { value: "sharp", key: "appearance.radiusSharp" },
  { value: "soft", key: "appearance.radiusSoft" },
  { value: "round", key: "appearance.radiusRound" },
] as const;

const DENSITY = [
  { value: "compact", key: "appearance.densityCompact" },
  { value: "comfortable", key: "appearance.densityComfortable" },
] as const;

/** How long a colour picker may keep changing before the value is written. */
const COMMIT_DELAY_MS = 400;

/**
 * Identity for a theme the reader just created.
 *
 * Module scope, not the component body: a component that calls `Date.now()` while
 * rendering is impure, and the lint rule that says so cannot tell an event
 * handler from a render.
 */
function customThemeId(): string {
  return `custom:${Date.now().toString(36)}`;
}

interface Patch {
  accent?: string | null;
  backgroundPath?: string | null;
  clearBackground?: boolean;
  backgroundDim?: number;
  theme?: Theme | null;
  clearTheme?: boolean;
  gameTint?: boolean;
}

/**
 * Appearance: theme, accent, background image and the shape of the interface.
 *
 * Everything is stored on the Rust side and applied by `AppProvider` through
 * `updateAppearance`, so it survives a reload and applies before the first screen
 * paints. The chosen hex is kept exactly as picked; the resolver adapts it to the
 * active theme so it stays readable.
 *
 * A theme's colours are edited as a draft and written on a short delay: a colour
 * picker fires on every pointer move, and a round trip per frame would stutter.
 * The draft is published to the store immediately, so the preview is live even
 * though the save is not.
 */
export function AppearancePanel() {
  const { t } = useI18n();
  const appearance = useAppStore((state) => state.appearance);
  const setAppearance = useAppStore((state) => state.setAppearance);
  const pushToast = useAppStore((state) => state.pushToast);
  const prefs = useAppStore((state) => state.prefs);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState<Theme | null>(null);
  const commitTimer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearTimeout(commitTimer.current), []);

  const apply = useCallback(
    async (change: Patch): Promise<void> => {
      setBusy(true);
      try {
        setAppearance(
          await api.appearanceSet({
            accent: change.accent === undefined ? (appearance?.accent ?? null) : change.accent,
            backgroundPath: change.backgroundPath ?? null,
            clearBackground: change.clearBackground ?? false,
            backgroundDim: change.backgroundDim ?? appearance?.backgroundDim ?? 0.45,
            theme: change.theme === undefined ? (appearance?.theme ?? null) : change.theme,
            clearTheme: change.clearTheme ?? false,
            gameTint: change.gameTint ?? appearance?.gameTint ?? true,
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
    },
    [appearance, pushToast, setAppearance, t],
  );

  /** Write a draft theme, reading the rest of the record at call time. */
  const persistTheme = useCallback(
    async (next: Theme): Promise<void> => {
      const current = useAppStore.getState().appearance;
      setBusy(true);
      try {
        setAppearance(
          await api.appearanceSet({
            accent: current?.accent ?? null,
            backgroundPath: null,
            clearBackground: false,
            backgroundDim: current?.backgroundDim ?? 0.45,
            theme: next,
            clearTheme: false,
            gameTint: current?.gameTint ?? true,
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
    },
    [pushToast, setAppearance, t],
  );

  /** Publish a draft for the live preview and schedule the write. */
  const editDraft = useCallback(
    (next: Theme): void => {
      setDraft(next);
      const current = useAppStore.getState().appearance;
      if (current) setAppearance({ ...current, theme: next });
      window.clearTimeout(commitTimer.current);
      commitTimer.current = window.setTimeout(() => {
        void persistTheme(next);
      }, COMMIT_DELAY_MS);
    },
    [persistTheme, setAppearance],
  );

  const startCustom = useCallback((): void => {
    const mode = resolveTheme(prefs.theme);
    const seed: Theme = appearance?.theme ?? {
      id: customThemeId(),
      name: t("appearance.themeCustom"),
      base: mode,
      tokens: { ...DEFAULT_THEME_TOKENS[mode] },
    };
    setDraft(seed);
    editDraft(seed);
  }, [appearance?.theme, editDraft, prefs.theme, t]);

  /**
   * Change one token of the theme on screen.
   *
   * Editing a preset forks it into a custom theme rather than mutating the
   * preset: the preset list is shared, and a reader who tweaks Nord should get
   * "Nord, with my background", not a silently different Nord for everyone.
   */
  const editTokens = (patch: Partial<ThemeTokens>): void => {
    const base = draft ?? appearance?.theme;
    if (!base) return;
    const next: Theme = {
      ...base,
      id: base.id.startsWith("custom:") ? base.id : customThemeId(),
      tokens: { ...base.tokens, ...patch },
    };
    setDraft(next);
    editDraft(next);
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

  const exportTheme = async (): Promise<void> => {
    const theme = draft ?? appearance?.theme;
    if (!theme) return;
    setBusy(true);
    try {
      if (await api.themeExportFile(theme)) pushToast("success", t("appearance.themeExported"));
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? describe(caught.code, caught.message, t) : t("common.error"),
      );
    } finally {
      setBusy(false);
    }
  };

  const importTheme = async (): Promise<void> => {
    setBusy(true);
    try {
      const imported = await api.themeImportFile();
      if (!imported) return;
      setDraft(imported);
      await apply({ theme: imported, clearTheme: false });
      pushToast("success", t("appearance.themeImported"));
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? describe(caught.code, caught.message, t) : t("common.error"),
      );
    } finally {
      setBusy(false);
    }
  };

  const currentAccent = appearance?.accent ?? DEFAULT_ACCENT;
  const activeTheme = draft ?? appearance?.theme ?? null;
  const contrast = activeTheme ? themeContrast(readableTokens(activeTheme.tokens)) : 0;

  return (
    <>
      <p className="settings__hint">{t("appearance.intro")}</p>

      <div className="settings__row settings__row--stack">
        <div className="settings__label">
          <span>{t("appearance.theme")}</span>
          <span className="settings__hint">{t("appearance.themeHint")}</span>
        </div>
        <div className="settings__control appearance__accents">
          <Chip
            selected={!appearance?.theme}
            onClick={() => {
              setDraft(null);
              void apply({ theme: null, clearTheme: true });
            }}
          >
            {t("appearance.themeDefault")}
          </Chip>
          {THEME_PRESETS.map((preset) => (
            <Chip
              key={preset.id}
              selected={appearance?.theme?.id === preset.id}
              onClick={() => {
                setDraft(null);
                void apply({ theme: preset, clearTheme: false });
              }}
            >
              <span
                className="appearance__swatch"
                style={{ backgroundColor: preset.tokens.bg }}
                aria-hidden
              />
              {preset.name}
            </Chip>
          ))}
          <Button onClick={startCustom} disabled={busy}>
            {t("appearance.themeCustom")}
          </Button>
        </div>
      </div>

      {activeTheme ? (
        <>
          {/* Stacked, like the accent row: six pickers beside a label leave the
           * label a min-content column, which wraps its hint one word per line. */}
          <div className="settings__row settings__row--stack">
            <div className="settings__label">
              <span>{t("appearance.themeColours")}</span>
              <span className="settings__hint">
                {t("appearance.contrast", { ratio: contrast.toFixed(1) })}
              </span>
            </div>
            <div className="settings__control appearance__tokens">
              {COLOUR_TOKENS.map((token) => (
                <label key={token.key} className="appearance__picker">
                  <span className="settings__hint">{t(token.label)}</span>
                  <input
                    type="color"
                    className="input appearance__color"
                    value={activeTheme.tokens[token.key] as string}
                    onChange={(event) =>
                      editTokens({ [token.key]: event.target.value } as Partial<ThemeTokens>)
                    }
                    disabled={busy}
                    aria-label={t(token.label)}
                  />
                  <code className="appearance__hex">
                    {activeTheme.tokens[token.key] as string}
                  </code>
                </label>
              ))}
            </div>
          </div>

          <div className="settings__row settings__row--top">
            <div className="settings__label">
              <span>{t("appearance.radius")}</span>
              <span className="settings__hint">{t("appearance.radiusHint")}</span>
            </div>
            <div className="settings__control appearance__accents">
              {RADIUS.map((option) => (
                <Chip
                  key={option.value}
                  selected={activeTheme.tokens.radius === option.value}
                  onClick={() => editTokens({ radius: option.value })}
                >
                  {t(option.key)}
                </Chip>
              ))}
            </div>
          </div>

          <div className="settings__row settings__row--top">
            <div className="settings__label">
              <span>{t("appearance.density")}</span>
              <span className="settings__hint">{t("appearance.densityHint")}</span>
            </div>
            <div className="settings__control appearance__accents">
              {DENSITY.map((option) => (
                <Chip
                  key={option.value}
                  selected={activeTheme.tokens.density === option.value}
                  onClick={() => editTokens({ density: option.value })}
                >
                  {t(option.key)}
                </Chip>
              ))}
            </div>
          </div>

          <label className="field">
            <span className="field__label">{t("appearance.glass")}</span>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={activeTheme.tokens.glass}
              onChange={(event) => editTokens({ glass: Number(event.target.value) })}
              disabled={busy}
            />
            <span className="field__hint">{t("appearance.glassHint")}</span>
          </label>

        </>
      ) : null}

      {/* Outside the theme block on purpose: importing a theme is most useful
       * when you have none, and hiding the button until you already use a theme
       * would make it undiscoverable. */}
      <div className="settings__row settings__row--top">
        <div className="settings__label">
          <span>{t("appearance.themeShare")}</span>
          <span className="settings__hint">{t("appearance.themeShareHint")}</span>
        </div>
        <div className="settings__control">
          <Button onClick={() => void exportTheme()} disabled={busy || !activeTheme}>
            {t("appearance.themeExport")}
          </Button>
          <Button onClick={() => void importTheme()} disabled={busy}>
            {t("appearance.themeImport")}
          </Button>
        </div>
      </div>

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

      <label className="settings__row settings__row--top">
        <span className="settings__label">
          <span>{t("appearance.gameTint")}</span>
          <span className="settings__hint">{t("appearance.gameTintHint")}</span>
        </span>
        <span className="settings__control">
          <input
            type="checkbox"
            checked={appearance?.gameTint ?? true}
            onChange={(event) => void apply({ gameTint: event.target.checked })}
            disabled={busy}
          />
        </span>
      </label>

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
            <Button onClick={() => void apply({ clearBackground: true })} disabled={busy}>
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

export type { AppearanceView, ThemeView };
