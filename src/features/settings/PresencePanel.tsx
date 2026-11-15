import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { Icon } from "@/components/Icon";
import { Sheet } from "@/components/Sheet";
import { api, IpcError } from "@/lib/ipc/client";
import type {
  PresenceActivity,
  PresenceDetailsMode,
  PresenceSettings,
  PresenceStatus,
  TextTransform,
} from "@/lib/ipc/types";

/** Every token the renderer understands, in the order the hint lists them. */
const TEMPLATE_TOKENS = [
  "{vn_name}",
  "{title_original}",
  "{status}",
  "{playtime}",
  "{chapter}",
  "{route}",
  "{vndb_rating}",
  "{my_rating}",
  "{vndb_developer}",
  "{vndb_length}",
  "{vndb_status}",
  "{vndb_year}",
  "{vndb_released}",
  "{vndb_url}",
  "{window_title}",
  "{exe_name}",
  "{elapsed_time}",
  "{date}",
  "{time}",
  "{game_state}",
];

/** Mirrors `vn_core::presence::MIN_CYCLE_SECONDS`: Discord throttles presence to
 * roughly one update per 15 s, so a faster cycle would visibly skip entries. */
const MIN_CYCLE_SECONDS = 5;

/** Discord renders at most two buttons on an activity. */
const MAX_BUTTONS = 2;

type WhichField =
  | "details"
  | "state"
  | "title"
  | "vndb_details_items"
  | "vndb_state"
  | "custom_details_items"
  | "custom_state_items";

/** Which transform each template field is paired with. */
type TransformKey =
  | "custom_details_transform"
  | "custom_state_transform"
  | "vndb_details_transform"
  | "vndb_state_transform";

/**
 * Line-based template list editor backed by a plain text draft so partial entries
 * survive typing and deletion mid-keystroke.
 */
function RotationListEditor({
  id,
  label,
  hint,
  items,
  rotate,
  rotateLabel,
  seconds,
  secondsLabel,
  placeholder,
  onFocus,
  onItems,
  onRotate,
  onSeconds,
  disabled,
}: {
  id: string;
  label: string;
  hint: string;
  items: string[];
  rotate: boolean;
  rotateLabel: string;
  seconds: number;
  secondsLabel: string;
  placeholder?: string;
  onFocus?: () => void;
  onItems: (next: string[]) => void;
  onRotate: (next: boolean) => void;
  onSeconds: (next: number) => void;
  disabled: boolean;
}) {
  const { t } = useI18n();
  const [draft, setDraft] = useState(() => items.join("\n"));
  // Joining and splitting on every render would rewrite the textarea's value
  // under the cursor; only a change from outside (a reload) resets the draft.
  const lastExternal = useRef(draft);

  useEffect(() => {
    const external = items.join("\n");
    if (external !== lastExternal.current) {
      lastExternal.current = external;
      setDraft(external);
    }
  }, [items]);

  const commit = (text: string) => {
    setDraft(text);
    const next = text.split("\n");
    lastExternal.current = next.join("\n");
    onItems(next);
  };

  return (
    <div className="presence-rotator">
      <div className="presence-rotator__head">
        <label htmlFor={id} className="settings__label">
          {label}
        </label>
        <Chip selected={rotate} ariaLabel={rotateLabel} onClick={() => onRotate(!rotate)}>
          {rotate ? t("common.enabled") : t("common.disabled")}
        </Chip>
      </div>
      <textarea
        id={id}
        className="textarea presence-rotator__list"
        rows={Math.min(Math.max(draft.split("\n").length, 2), 6)}
        value={draft}
        placeholder={placeholder}
        disabled={disabled}
        onFocus={onFocus}
        onChange={(event) => commit(event.target.value)}
        spellCheck={false}
      />
      <span className="settings__hint">{hint}</span>
      {rotate ? (
        <div className="presence-rotator__seconds">
          <label htmlFor={`${id}-seconds`} className="settings__label">
            {secondsLabel}
          </label>
          <input
            id={`${id}-seconds`}
            type="number"
            className="input input--compact"
            min={MIN_CYCLE_SECONDS}
            max={120}
            value={seconds}
            disabled={disabled}
            onChange={(event) =>
              onSeconds(
                Math.max(
                  MIN_CYCLE_SECONDS,
                  Number(event.target.value) || MIN_CYCLE_SECONDS,
                ),
              )
            }
          />
        </div>
      ) : null}
    </div>
  );
}

/** Prefix / suffix / remove / replace / trim, collapsed by default. */
function TransformEditor({
  id,
  label,
  hint,
  transform,
  onChange,
}: {
  id: string;
  label: string;
  hint: string;
  transform: TextTransform;
  onChange: (next: TextTransform) => void;
}) {
  const { t } = useI18n();
  const [removeDraft, setRemoveDraft] = useState(() => transform.remove.join("\n"));
  const [replaceDraft, setReplaceDraft] = useState(() =>
    transform.replace.map((rule) => `${rule.old}=${rule.new}`).join("\n"),
  );

  const patch = (next: Partial<TextTransform>) => onChange({ ...transform, ...next });

  const commitRemove = (text: string) => {
    setRemoveDraft(text);
    patch({ remove: text.split("\n") });
  };

  const commitReplace = (text: string) => {
    setReplaceDraft(text);
    // A line without `=` is a rule still being typed, not a rule to discard: it
    // is kept with an empty value so the textarea does not delete it mid-keystroke.
    patch({
      replace: text.split("\n").map((line) => {
        const at = line.indexOf("=");
        return at === -1
          ? { old: line, new: "" }
          : { old: line.slice(0, at), new: line.slice(at + 1) };
      }),
    });
  };

  const removeCount = transform.remove.filter((entry) => entry !== "").length;
  const replaceCount = transform.replace.filter((rule) => rule.old !== "").length;
  const summary = [
    removeCount > 0
      ? t("settings.presenceTransformRemoving", { count: removeCount })
      : null,
    replaceCount > 0
      ? t("settings.presenceTransformReplacing", { count: replaceCount })
      : null,
  ].filter(Boolean);

  return (
    <details className="settings__details presence-transform">
      <summary>
        {label}
        {summary.length > 0 ? ` · ${summary.join(", ")}` : ""}
      </summary>
      <div className="presence-transform__body">
        <div className="presence-transform__pair">
          <div className="settings__field">
            <label htmlFor={`${id}-prefix`} className="settings__label">
              {t("settings.presenceTransformPrefix")}
            </label>
            <input
              id={`${id}-prefix`}
              className="input input--compact"
              value={transform.prefix}
              onChange={(event) => patch({ prefix: event.target.value })}
            />
          </div>
          <div className="settings__field">
            <label htmlFor={`${id}-suffix`} className="settings__label">
              {t("settings.presenceTransformSuffix")}
            </label>
            <input
              id={`${id}-suffix`}
              className="input input--compact"
              value={transform.suffix}
              onChange={(event) => patch({ suffix: event.target.value })}
            />
          </div>
        </div>

        <div className="settings__field">
          <label htmlFor={`${id}-remove`} className="settings__label">
            {t("settings.presenceTransformRemove")}
          </label>
          <textarea
            id={`${id}-remove`}
            className="textarea"
            rows={2}
            value={removeDraft}
            spellCheck={false}
            onChange={(event) => commitRemove(event.target.value)}
          />
        </div>

        <div className="settings__field">
          <label htmlFor={`${id}-replace`} className="settings__label">
            {t("settings.presenceTransformReplace")}
          </label>
          <textarea
            id={`${id}-replace`}
            className="textarea"
            rows={2}
            value={replaceDraft}
            spellCheck={false}
            onChange={(event) => commitReplace(event.target.value)}
          />
          <span className="settings__hint">
            {t("settings.presenceTransformReplaceHint")}
          </span>
        </div>

        <div className="settings__row">
          <span className="settings__label">{t("settings.presenceTransformTrim")}</span>
          <div className="settings__control">
            <Chip
              selected={transform.trim}
              onClick={() => patch({ trim: !transform.trim })}
            >
              {transform.trim ? t("common.enabled") : t("common.disabled")}
            </Chip>
          </div>
        </div>

        <span className="settings__hint">{hint}</span>
      </div>
    </details>
  );
}

function AssetEditor({
  size,
  image,
  text,
  onImage,
  onText,
  useExeIcon,
  onExeIcon,
}: {
  size: "large" | "small";
  image: string;
  text: string;
  onImage: (value: string) => void;
  onText: (value: string) => void;
  useExeIcon?: boolean;
  onExeIcon?: ((value: boolean) => void) | undefined;
}) {
  const { t } = useI18n();
  return (
    <details className="settings__details presence-assets">
      <summary>
        {t(
          size === "large"
            ? "settings.presenceLargeImageOptions"
            : "settings.presenceSmallImageOptions",
        )}
      </summary>
      <div className="presence-assets__body">
        {onExeIcon ? (
          <div className="settings__row">
            <span className="settings__label">{t("settings.presenceExeIcon")}</span>
            <div className="settings__control">
              <Chip
                selected={useExeIcon ?? true}
                ariaLabel={t("settings.presenceExeIcon")}
                onClick={() => onExeIcon(!(useExeIcon ?? true))}
              >
                {t((useExeIcon ?? true) ? "common.enabled" : "common.disabled")}
              </Chip>
            </div>
          </div>
        ) : null}
        <div className="settings__field">
          <label htmlFor={`presence-${size}-image`} className="settings__label">
            {t(
              size === "large"
                ? "settings.presenceLargeImage"
                : "settings.presenceSmallImage",
            )}
          </label>
          <input
            id={`presence-${size}-image`}
            className="input"
            value={image}
            maxLength={512}
            onChange={(event) => onImage(event.target.value)}
            placeholder={
              size === "large" ? t("settings.presenceLibraryCover") : undefined
            }
          />
          {size === "small" ? (
            <span className="settings__hint">
              {t(
                onExeIcon
                  ? "settings.presenceExeIconHint"
                  : "settings.presenceSmallImageHint",
              )}
            </span>
          ) : null}
          <span className="settings__hint">{t("settings.presenceImageAssetHint")}</span>
        </div>
        <div className="settings__field">
          <label htmlFor={`presence-${size}-text`} className="settings__label">
            {t(
              size === "large"
                ? "settings.presenceLargeText"
                : "settings.presenceSmallText",
            )}
          </label>
          <input
            id={`presence-${size}-text`}
            className="input"
            value={text}
            maxLength={2048}
            onChange={(event) => onText(event.target.value)}
            placeholder={size === "large" ? "{vn_name}" : undefined}
          />
          <span className="settings__hint">{t("settings.presenceImageTextHint")}</span>
        </div>
      </div>
    </details>
  );
}

function CustomButtonEditor({
  buttons,
  onChange,
}: {
  buttons: { label: string; url: string }[];
  onChange: (buttons: { label: string; url: string }[]) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="presence-buttons-editor">
      <span className="settings__hint">{t("settings.presenceCustomButtonsHint")}</span>
      {[0, 1].map((index) => {
        const button = buttons[index] ?? { label: "", url: "" };
        const patch = (next: Partial<typeof button>) => {
          const slots = [0, 1].map((slot) => buttons[slot] ?? { label: "", url: "" });
          slots[index] = { ...button, ...next };
          onChange(slots);
        };
        return (
          <fieldset key={index} className="presence-buttons-editor__slot">
            <legend>{t("settings.presenceButtonNumber", { number: index + 1 })}</legend>
            <div className="settings__field">
              <label
                htmlFor={`presence-button-${index}-label`}
                className="settings__label"
              >
                {t("settings.presenceButtonLabel")}
              </label>
              <input
                id={`presence-button-${index}-label`}
                className="input"
                value={button.label}
                maxLength={128}
                onChange={(event) => patch({ label: event.target.value })}
              />
            </div>
            <div className="settings__field">
              <label htmlFor={`presence-button-${index}-url`} className="settings__label">
                {t("settings.presenceButtonUrl")}
              </label>
              <input
                id={`presence-button-${index}-url`}
                className="input"
                value={button.url}
                maxLength={512}
                onChange={(event) => patch({ url: event.target.value })}
                placeholder="{vndb_url}"
              />
            </div>
          </fieldset>
        );
      })}
    </div>
  );
}

export function PresencePanel() {
  const { t } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const platform = useAppStore((state) => state.platform);
  const isAndroid = platform?.os === "android";

  const [settings, setSettings] = useState<PresenceSettings | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [status, setStatus] = useState<PresenceStatus | null>(null);
  const [preview, setPreview] = useState<PresenceActivity | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [activeField, setActiveField] = useState<WhichField>("details");
  const [previewCycle, setPreviewCycle] = useState(0);
  // Active library game id for live token preview so `{vn_name}` resolves a real title.
  const [previewGameId, setPreviewGameId] = useState<string>("");
  // `""` edits global defaults; a game id edits per-game overrides (untouched fields inherit global).
  const [scopeGameId, setScopeGameId] = useState<string>("");
  const scopeGeneration = useRef(0);
  const scopeGameRef = useRef("");
  const scopeReady = useRef(true);
  const [scopeLoading, setScopeLoading] = useState(false);
  const [scopeError, setScopeError] = useState<string | null>(null);
  const [games, setGames] = useState<{ id: string; title: string }[]>([]);
  const [previewStateCycle, setPreviewStateCycle] = useState(0);
  const [linkError, setLinkError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  const [hasToken, setHasToken] = useState(false);
  const [tokenPreview, setTokenPreview] = useState<string | null>(null);
  const [showTokenModal, setShowTokenModal] = useState(false);
  const [tosAccepted, setTosAccepted] = useState(false);
  const [tokenInput, setTokenInput] = useState("");
  const [tokenError, setTokenError] = useState<string | null>(null);
  const [tokenBusy, setTokenBusy] = useState(false);

  // imgbb API key for the optional image-host upload.
  const [imageKeyPreview, setImageKeyPreview] = useState<string | null>(null);
  const [imageKeyInput, setImageKeyInput] = useState("");
  const [imageKeyError, setImageKeyError] = useState<string | null>(null);
  const [imageKeyBusy, setImageKeyBusy] = useState(false);

  useEffect(() => {
    let active = true;
    const generation = scopeGeneration.current;
    const gameId = scopeGameRef.current;
    void Promise.all([
      gameId ? api.presenceEffectiveSettings(gameId) : api.presenceGetSettings(),
      api.presenceStatus(),
      api.presenceHasToken(),
      api.presenceTokenPreview(),
      api.presenceImageKeyPreview(),
    ])
      .then(([initialSettings, initialStatus, tokenExists, previewStr, keyPreview]) => {
        if (!active) return;
        setLoadError(null);
        if (generation === scopeGeneration.current) {
          scopeReady.current = true;
          setSettings(initialSettings);
        }
        setStatus(initialStatus);
        setHasToken(tokenExists);
        setTokenPreview(previewStr);
        setImageKeyPreview(keyPreview);
      })
      .catch((caught: unknown) => {
        if (active)
          setLoadError(caught instanceof IpcError ? caught.message : t("common.error"));
      });

    const interval = window.setInterval(() => {
      void api
        .presenceStatus()
        .then((nextStatus) => {
          if (active) setStatus(nextStatus);
        })
        .catch(() => undefined);
    }, 5000);

    return () => {
      active = false;
      window.clearInterval(interval);
    };
  }, [loadAttempt, t]);

  useEffect(
    () => () => {
      scopeGeneration.current += 1;
      scopeReady.current = false;
    },
    [],
  );

  useEffect(() => {
    let active = true;
    void api
      .listGames()
      .then((cards) => {
        if (!active) return;
        const options = cards.map((card) => ({
          id: card.game.meta.id,
          title: card.game.title,
        }));
        setGames(options);
        // Seed preview with the first real title so template tokens expand on open.
        setPreviewGameId((current) => current || (options[0]?.id ?? ""));
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!settings || scopeLoading || scopeError) return;
    let active = true;
    const generation = scopeGeneration.current;
    void api
      .presencePreview(
        settings,
        previewGameId || undefined,
        previewCycle,
        previewStateCycle,
      )
      .then((next) => {
        if (active && generation === scopeGeneration.current) setPreview(next);
      })
      .catch(() => undefined);

    return () => {
      active = false;
    };
  }, [
    settings,
    previewCycle,
    previewStateCycle,
    previewGameId,
    imageKeyPreview,
    scopeLoading,
    scopeError,
  ]);

  const resetPreview = useCallback(() => {
    setPreviewCycle(0);
    setPreviewStateCycle(0);
  }, []);

  // Rotation entry counts; 0 hides the preview pager when rotation is disabled.
  const rotationLengths = useMemo(() => {
    if (!settings) return { details: 0, state: 0 };
    const usable = (list: string[]) => list.filter((entry) => entry.trim() !== "").length;
    const isVndb = settings.details_mode === "vndb";
    const detailsRotating = isVndb
      ? settings.cycle_vndb_details
      : settings.custom_details_rotate && settings.custom_details_items.length > 0;
    const detailsCount = !detailsRotating
      ? 0
      : isVndb
        ? settings.vndb_details_items.length > 0
          ? usable(settings.vndb_details_items)
          : 4
        : usable(settings.custom_details_items);
    return {
      details: detailsCount,
      state: settings.custom_state_rotate ? usable(settings.custom_state_items) : 0,
    };
  }, [settings]);

  // Load effective baseline values for the selected scope so inherited fields remain visible.
  const switchScope = useCallback(
    (gameId: string) => {
      const generation = ++scopeGeneration.current;
      scopeGameRef.current = gameId;
      scopeReady.current = false;
      setScopeLoading(true);
      setScopeError(null);
      setBusy(false);
      setSaved(false);
      setSaveError(null);
      setLinkError(null);
      setPreview(null);
      setScopeGameId(gameId);
      if (gameId) {
        setPreviewGameId(gameId);
      }
      resetPreview();
      void api
        .presenceEffectiveSettings(gameId || undefined)
        .then((next) => {
          if (generation !== scopeGeneration.current) return;
          scopeReady.current = true;
          setScopeLoading(false);
          setSettings(next);
        })
        .catch((caught: unknown) => {
          if (generation !== scopeGeneration.current) return;
          setScopeLoading(false);
          setScopeError(caught instanceof IpcError ? caught.message : t("common.error"));
        });
    },
    [resetPreview, t],
  );

  if (!settings)
    return (
      <Card className="settings__card">
        <CardHeader title={t("settings.presence")} />
        <div className="presence-panel">
          <p role={loadError ? "alert" : "status"}>{loadError ?? t("common.loading")}</p>
          {loadError ? (
            <Button
              onClick={() => {
                setLoadError(null);
                setLoadAttempt((attempt) => attempt + 1);
              }}
            >
              {t("common.retry")}
            </Button>
          ) : null}
        </div>
      </Card>
    );

  const updateSettings = (patch: Partial<PresenceSettings>) => {
    if (!scopeReady.current) return;
    setSaved(false);
    setSaveError(null);
    setSettings((prev) => (prev ? { ...prev, ...patch } : prev));
  };

  const toggleEnabled = async () => {
    try {
      const next = await api.presenceSetEnabled(!settings.enabled);
      setSettings((prev) => (prev ? { ...prev, enabled: next.enabled } : next));
      setStatus(await api.presenceStatus());
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    }
  };

  const resetToGlobal = async (): Promise<void> => {
    if (!scopeGameId || !scopeReady.current || busy) return;
    const generation = scopeGeneration.current;
    setBusy(true);
    try {
      const next = await api.presenceResetGameSettings(scopeGameId);
      if (generation !== scopeGeneration.current) return;
      setSettings(next);
      setSaved(false);
      pushToast("info", t("settings.presenceScopeReset"));
    } catch (caught) {
      if (generation !== scopeGeneration.current) return;
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      if (generation === scopeGeneration.current) setBusy(false);
    }
  };

  const save = async () => {
    if (!scopeReady.current || busy) return;
    const generation = scopeGeneration.current;
    setBusy(true);
    setSaved(false);
    setLinkError(null);
    setSaveError(null);
    try {
      await api.presenceSaveSettings(settings, scopeGameId || undefined);
      if (generation !== scopeGeneration.current) return;
      const nextStatus = await api.presenceStatus();
      if (generation !== scopeGeneration.current) return;
      setSaved(true);
      setStatus(nextStatus);
      pushToast("info", t("settings.presenceSaved"));
    } catch (caught) {
      if (generation !== scopeGeneration.current) return;
      const message = caught instanceof IpcError ? caught.message : t("common.error");
      setSaveError(message);
      if (caught instanceof IpcError && caught.code === "invalid_profile_link") {
        setLinkError(message);
      }
      pushToast("error", message);
    } finally {
      if (generation === scopeGeneration.current) setBusy(false);
    }
  };

  const setMode = (mode: PresenceDetailsMode) => {
    resetPreview();
    updateSettings({ details_mode: mode });
  };

  const appendTokenToList = (list: string[], token: string): string[] => {
    if (list.length === 0) return [token];
    const next = [...list];
    next[next.length - 1] = `${next[next.length - 1]}${token}`;
    return next;
  };

  const insertToken = (token: string) => {
    if (activeField === "title") {
      updateSettings({ activity_name: (settings.activity_name ?? "") + token });
    } else if (activeField === "vndb_state") {
      updateSettings({ vndb_state_template: settings.vndb_state_template + token });
    } else if (activeField === "vndb_details_items") {
      resetPreview();
      updateSettings({
        vndb_details_items: appendTokenToList(settings.vndb_details_items, token),
      });
    } else if (activeField === "custom_details_items") {
      resetPreview();
      updateSettings({
        custom_details_items: appendTokenToList(settings.custom_details_items, token),
      });
    } else if (activeField === "custom_state_items") {
      resetPreview();
      updateSettings({
        custom_state_items: appendTokenToList(settings.custom_state_items, token),
      });
    } else if (activeField === "details") {
      if (settings.details_mode === "vndb") {
        resetPreview();
        updateSettings({
          vndb_details_items: appendTokenToList(settings.vndb_details_items, token),
        });
      } else {
        updateSettings({ custom_details: settings.custom_details + token });
      }
    } else {
      updateSettings({ custom_state: settings.custom_state + token });
    }
  };

  const resetFormatting = () => {
    if (!scopeReady.current || busy) return;
    const emptyTransform: TextTransform = {
      prefix: "",
      suffix: "",
      remove: [],
      replace: [],
      trim: false,
    };
    resetPreview();
    updateSettings({
      details_mode: "vndb",
      activity_name: null,
      cycle_vndb_details: false,
      cycle_seconds: 5,
      vndb_details_items: [],
      vndb_state_template: "",
      vndb_swap_details_state: false,
      vndb_loading_details: "Reading",
      vndb_loading_state: "Loading VNDB data...",
      vndb_details_transform: emptyTransform,
      vndb_state_transform: emptyTransform,
      custom_details: "{vn_name}",
      custom_state: "{chapter}",
      custom_details_items: [],
      custom_state_items: [],
      custom_details_rotate: false,
      custom_state_rotate: false,
      custom_details_cycle_seconds: 5,
      custom_state_cycle_seconds: 5,
      custom_details_transform: emptyTransform,
      custom_state_transform: emptyTransform,
      manual_details: "",
      manual_state: "",
      large_image: "",
      large_text: "",
      small_image: "",
      use_exe_icon: true,
      small_text: "",
      use_custom_buttons: false,
      custom_buttons: [],
    });
    pushToast("info", t("settings.presenceDefaultsRestored"));
  };

  const setTransform = (key: TransformKey, next: TextTransform) => {
    updateSettings({ [key]: next });
  };

  const openTokenDialog = () => {
    setTokenInput("");
    setTokenError(null);
    setTosAccepted(false);
    setShowTokenModal(true);
  };

  const handleSaveToken = async () => {
    if (!tokenInput.trim()) {
      setTokenError(t("settings.presenceTokenInputPlaceholder"));
      return;
    }
    setTokenBusy(true);
    setTokenError(null);
    try {
      await api.presenceSaveToken(tokenInput.trim());
      setHasToken(true);
      setTokenPreview(await api.presenceTokenPreview());
      setShowTokenModal(false);
      setTokenInput("");
      setStatus(await api.presenceStatus());
      pushToast("info", t("settings.presenceTokenSaved"));
    } catch (caught) {
      setTokenError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setTokenBusy(false);
    }
  };

  const handleRemoveToken = async () => {
    try {
      await api.presenceClearToken();
      setHasToken(false);
      setTokenPreview(null);
      setStatus(await api.presenceStatus());
      pushToast("info", t("settings.presenceTokenRemoved"));
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    }
  };

  const handleSaveImageKey = async () => {
    setImageKeyBusy(true);
    setImageKeyError(null);
    try {
      await api.presenceImageKeySave(imageKeyInput.trim());
      setImageKeyPreview(await api.presenceImageKeyPreview());
      setImageKeyInput("");
      pushToast("info", t("settings.presenceImageKeySaved"));
    } catch (caught) {
      setImageKeyError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setImageKeyBusy(false);
    }
  };

  const handleRemoveImageKey = async () => {
    try {
      await api.presenceImageKeyClear();
      setImageKeyPreview(null);
      pushToast("info", t("settings.presenceImageKeyRemoved"));
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    }
  };

  const renderStatusBadge = () => {
    if (!status) return null;
    let label: string;
    let color: string;

    switch (status.status) {
      case "connected":
        label = t("settings.presenceStatusConnected");
        color = "var(--success)";
        break;
      case "connecting":
        label = t("settings.presenceStatusConnecting");
        color = "var(--warning)";
        break;
      case "discord_not_running":
        label = t("settings.presenceStatusDiscordNotRunning");
        color = "var(--text-muted)";
        break;
      case "disconnected":
        label = t("settings.presenceStatusDisconnected");
        color = "var(--text-muted)";
        break;
      case "unsupported_platform":
        label = t("settings.presenceStatusUnsupported");
        color = "var(--text-muted)";
        break;
      case "error":
        label = status.message || "Error";
        color = "var(--danger)";
        break;
      case "disabled":
      default:
        label = t("settings.presenceStatusDisabled");
        color = "var(--text-muted)";
        break;
    }

    return (
      <span className="presence-status" style={{ borderColor: color, color }}>
        <span className="presence-status__dot" style={{ backgroundColor: color }} />
        {label}
      </span>
    );
  };

  /** Steps the preview through a rotation list without saving. */
  const renderCycleControls = (
    length: number,
    value: number,
    onChange: (next: number) => void,
    label: string,
  ) => {
    if (length < 2) return null;
    return (
      <div className="presence-cycle">
        <span className="settings__hint">{label}</span>
        <div className="settings__control">
          <Button
            variant="ghost"
            aria-label={t("settings.presencePreviewPrev")}
            onClick={() => onChange((value - 1 + length) % length)}
          >
            ‹
          </Button>
          <span className="presence-cycle__count">
            {(value % length) + 1} / {length}
          </span>
          <Button
            variant="ghost"
            aria-label={t("settings.presencePreviewNext")}
            onClick={() => onChange((value + 1) % length)}
          >
            ›
          </Button>
        </div>
      </div>
    );
  };

  return (
    <>
      <Card className="settings__card">
        <CardHeader title={t("settings.presence")} aside={renderStatusBadge()} />

        <div className="presence-panel">
          <div className="settings__row">
            <span className="settings__label">{t("settings.presenceEnable")}</span>
            <div className="settings__control">
              <Chip selected={settings.enabled} onClick={() => void toggleEnabled()}>
                {settings.enabled ? t("common.enabled") : t("common.disabled")}
              </Chip>
            </div>
          </div>

          {isAndroid ? (
            <div className="settings__row settings__row--top">
              <div className="settings__label">
                <span>{t("settings.presenceDiscordToken")}</span>
                <span className="settings__hint">
                  {t("settings.presenceDiscordTokenHint")}
                </span>
              </div>
              <div className="settings__control">
                {hasToken && tokenPreview ? (
                  <>
                    <code className="presence-token-preview">{tokenPreview}</code>
                    <Button variant="secondary" onClick={openTokenDialog}>
                      {t("settings.presenceChangeToken")}
                    </Button>
                    <Button variant="ghost" onClick={() => void handleRemoveToken()}>
                      {t("settings.presenceRemoveToken")}
                    </Button>
                  </>
                ) : (
                  <>
                    <span className="settings__hint">
                      {t("settings.presenceTokenNotConfigured")}
                    </span>
                    <Button variant="primary" onClick={openTokenDialog}>
                      {t("settings.presenceSetToken")}
                    </Button>
                  </>
                )}
              </div>
            </div>
          ) : null}

          {settings.enabled ? (
            <>
              {/* Stack label above mode chips so three chips never overflow narrow mobile cards. */}
              <div className="presence-mode">
                <span className="settings__label">{t("settings.presenceMode")}</span>
                <div className="settings__control">
                  {(
                    [
                      ["vndb", "settings.presenceModeVndb"],
                      ["custom", "settings.presenceModeCustom"],
                      ["manual", "settings.presenceModeManual"],
                    ] as const
                  ).map(([mode, labelKey]) => (
                    <Chip
                      key={mode}
                      selected={settings.details_mode === mode}
                      onClick={() => setMode(mode)}
                    >
                      {t(labelKey)}
                    </Chip>
                  ))}
                </div>
              </div>

              <div className="settings__field">
                <label htmlFor="activity-name" className="settings__label">
                  {t("settings.presenceCardTitle")}
                </label>
                <input
                  id="activity-name"
                  type="text"
                  className="input"
                  value={settings.activity_name ?? ""}
                  onFocus={() => setActiveField("title")}
                  onChange={(event) =>
                    updateSettings({
                      activity_name:
                        event.target.value === "" ? null : event.target.value,
                    })
                  }
                  placeholder="{vn_name}"
                />
                <span className="settings__hint">
                  {t("settings.presenceCardTitleHint")}
                </span>
              </div>

              {settings.details_mode === "manual" ? (
                <>
                  <div className="settings__field">
                    <label htmlFor="manual-details" className="settings__label">
                      {t("settings.presenceDetails")}
                    </label>
                    <input
                      id="manual-details"
                      type="text"
                      className="input"
                      value={settings.manual_details}
                      onChange={(event) =>
                        updateSettings({ manual_details: event.target.value })
                      }
                    />
                  </div>
                  <div className="settings__field">
                    <label htmlFor="manual-state" className="settings__label">
                      {t("settings.presenceState")}
                    </label>
                    <input
                      id="manual-state"
                      type="text"
                      className="input"
                      value={settings.manual_state}
                      onChange={(event) =>
                        updateSettings({ manual_state: event.target.value })
                      }
                    />
                  </div>
                </>
              ) : null}

              {settings.details_mode === "custom" ? (
                <>
                  <div className="settings__field">
                    <label htmlFor="custom-details" className="settings__label">
                      {t("settings.presenceDetails")}
                    </label>
                    <input
                      id="custom-details"
                      type="text"
                      className="input"
                      value={settings.custom_details}
                      onFocus={() => setActiveField("details")}
                      onChange={(event) =>
                        updateSettings({ custom_details: event.target.value })
                      }
                      placeholder="{vn_name}"
                    />
                    <span className="settings__hint">
                      {t("settings.presenceSingleTemplateHint")}
                    </span>
                  </div>

                  <RotationListEditor
                    id="custom-details-items"
                    label={t("settings.presenceRotationList")}
                    hint={t("settings.presenceRotationListHint")}
                    items={settings.custom_details_items}
                    rotate={settings.custom_details_rotate}
                    rotateLabel={t("settings.presenceRotate")}
                    seconds={settings.custom_details_cycle_seconds}
                    secondsLabel={t("settings.presenceCycleSeconds")}
                    onFocus={() => setActiveField("custom_details_items")}
                    onItems={(next) => {
                      resetPreview();
                      updateSettings({ custom_details_items: next });
                    }}
                    onRotate={(next) => updateSettings({ custom_details_rotate: next })}
                    onSeconds={(next) =>
                      updateSettings({ custom_details_cycle_seconds: next })
                    }
                    disabled={false}
                  />

                  <div className="settings__field">
                    <label htmlFor="custom-state" className="settings__label">
                      {t("settings.presenceState")}
                    </label>
                    <input
                      id="custom-state"
                      type="text"
                      className="input"
                      value={settings.custom_state}
                      onFocus={() => setActiveField("state")}
                      onChange={(event) =>
                        updateSettings({ custom_state: event.target.value })
                      }
                      placeholder="{chapter}"
                    />
                    <span className="settings__hint">
                      {t("settings.presenceSingleTemplateHint")}
                    </span>
                  </div>

                  <RotationListEditor
                    id="custom-state-items"
                    label={t("settings.presenceRotationList")}
                    hint={t("settings.presenceRotationListHint")}
                    items={settings.custom_state_items}
                    rotate={settings.custom_state_rotate}
                    rotateLabel={t("settings.presenceRotate")}
                    seconds={settings.custom_state_cycle_seconds}
                    secondsLabel={t("settings.presenceCycleSeconds")}
                    onFocus={() => setActiveField("custom_state_items")}
                    onItems={(next) => {
                      resetPreview();
                      updateSettings({ custom_state_items: next });
                    }}
                    onRotate={(next) => updateSettings({ custom_state_rotate: next })}
                    onSeconds={(next) =>
                      updateSettings({ custom_state_cycle_seconds: next })
                    }
                    disabled={false}
                  />

                  <TransformEditor
                    id="custom-details-transform"
                    label={t("settings.presenceTransform")}
                    hint={t("settings.presenceTransformHint")}
                    transform={settings.custom_details_transform}
                    onChange={(next) => setTransform("custom_details_transform", next)}
                  />
                  <TransformEditor
                    id="custom-state-transform"
                    label={t("settings.presenceTransform")}
                    hint={t("settings.presenceTransformHint")}
                    transform={settings.custom_state_transform}
                    onChange={(next) => setTransform("custom_state_transform", next)}
                  />
                </>
              ) : null}

              {settings.details_mode === "vndb" ? (
                <>
                  {/* RotationListEditor owns the per-mode toggle and interval so controls aren't duplicated. */}
                  <RotationListEditor
                    id="vndb-details-items"
                    label={t("settings.presenceVndbDetailsItems")}
                    hint={t("settings.presenceVndbDetailsItemsHint")}
                    items={settings.vndb_details_items}
                    rotate={settings.cycle_vndb_details}
                    rotateLabel={t("settings.presenceCycleDetails")}
                    seconds={settings.cycle_seconds}
                    secondsLabel={t("settings.presenceCycleSeconds")}
                    placeholder="★ {vndb_rating}/10 · {vndb_year} · {vndb_length}"
                    onFocus={() => setActiveField("vndb_details_items")}
                    onItems={(next) => {
                      resetPreview();
                      updateSettings({ vndb_details_items: next });
                    }}
                    onRotate={(next) => {
                      resetPreview();
                      updateSettings({ cycle_vndb_details: next });
                    }}
                    onSeconds={(next) => updateSettings({ cycle_seconds: next })}
                    disabled={false}
                  />

                  <div className="settings__field">
                    <label htmlFor="vndb-state-template" className="settings__label">
                      {t("settings.presenceVndbStateTemplate")}
                    </label>
                    <input
                      id="vndb-state-template"
                      type="text"
                      className="input"
                      value={settings.vndb_state_template}
                      onFocus={() => setActiveField("vndb_state")}
                      onChange={(event) =>
                        updateSettings({ vndb_state_template: event.target.value })
                      }
                      placeholder="{route} - {chapter}"
                    />
                    <span className="settings__hint">
                      {t("settings.presenceVndbStateTemplateHint")}
                    </span>
                  </div>

                  <div className="settings__row">
                    <span className="settings__label">
                      {t("settings.presenceSwapLines")}
                    </span>
                    <div className="settings__control">
                      <Chip
                        selected={settings.vndb_swap_details_state}
                        onClick={() =>
                          updateSettings({
                            vndb_swap_details_state: !settings.vndb_swap_details_state,
                          })
                        }
                      >
                        {settings.vndb_swap_details_state
                          ? t("common.enabled")
                          : t("common.disabled")}
                      </Chip>
                    </div>
                  </div>
                  <span className="settings__hint">
                    {t("settings.presenceSwapLinesHint")}
                  </span>

                  <TransformEditor
                    id="vndb-details-transform"
                    label={t("settings.presenceTransform")}
                    hint={t("settings.presenceTransformHint")}
                    transform={settings.vndb_details_transform}
                    onChange={(next) => setTransform("vndb_details_transform", next)}
                  />
                  <TransformEditor
                    id="vndb-state-transform"
                    label={t("settings.presenceTransform")}
                    hint={t("settings.presenceTransformHint")}
                    transform={settings.vndb_state_transform}
                    onChange={(next) => setTransform("vndb_state_transform", next)}
                  />
                </>
              ) : null}

              <div className="settings__field">
                <span className="settings__label">
                  {t("settings.presenceTokensTitle")}
                </span>
                <div className="presence-tokens">
                  {TEMPLATE_TOKENS.map((token) => (
                    <Chip key={token} onClick={() => insertToken(token)}>
                      {token}
                    </Chip>
                  ))}
                </div>
                <span className="settings__hint">{t("settings.presenceTokensHint")}</span>
              </div>

              <div className="settings__row">
                <span className="settings__label">{t("settings.presenceShowCover")}</span>
                <div className="settings__control">
                  <Chip
                    selected={settings.show_cover}
                    onClick={() => updateSettings({ show_cover: !settings.show_cover })}
                  >
                    {settings.show_cover ? t("common.enabled") : t("common.disabled")}
                  </Chip>
                </div>
              </div>
              <span className="settings__hint">
                {t("settings.presenceShowCoverHint")}
              </span>

              {settings.show_cover ? (
                <AssetEditor
                  size="large"
                  image={settings.large_image ?? ""}
                  text={settings.large_text ?? ""}
                  onImage={(large_image) => updateSettings({ large_image })}
                  onText={(large_text) => updateSettings({ large_text })}
                />
              ) : null}
              {/* The host also publishes the default small icon when the large cover is off. */}
              <div className="presence-mode">
                <span className="settings__label">{t("settings.presenceImageHost")}</span>
                <div className="settings__control">
                  {(
                    [
                      ["none", "settings.presenceImageHostNone"],
                      ["catbox", "settings.presenceImageHostCatbox"],
                      ["imgbb", "settings.presenceImageHostImgbb"],
                    ] as const
                  ).map(([host, labelKey]) => (
                    <Chip
                      key={host}
                      selected={settings.rpc_image_host === host}
                      onClick={() => updateSettings({ rpc_image_host: host })}
                    >
                      {t(labelKey)}
                    </Chip>
                  ))}
                </div>
                <span className="settings__hint">
                  {t("settings.presenceImageHostHint")}
                </span>
              </div>

              {settings.rpc_image_host === "imgbb" ? (
                <div className="settings__field">
                  <label htmlFor="imgbb-key" className="settings__label">
                    {t("settings.presenceImageKey")}
                  </label>
                  {imageKeyPreview ? (
                    <div className="presence-tokens presence-tokens--key">
                      <code className="presence-token-preview">{imageKeyPreview}</code>
                      <Button variant="ghost" onClick={() => void handleRemoveImageKey()}>
                        {t("settings.presenceImageKeyRemove")}
                      </Button>
                    </div>
                  ) : (
                    <>
                      <input
                        id="imgbb-key"
                        type="password"
                        className="input"
                        value={imageKeyInput}
                        onChange={(event) => {
                          setImageKeyInput(event.target.value);
                          setImageKeyError(null);
                        }}
                        placeholder={t("settings.presenceImageKeyPlaceholder")}
                      />
                      <div className="presence-actions">
                        <Button
                          disabled={imageKeyBusy || !imageKeyInput.trim()}
                          onClick={() => void handleSaveImageKey()}
                        >
                          {t("settings.presenceImageKeySave")}
                        </Button>
                      </div>
                    </>
                  )}
                  <span className="settings__hint">
                    {t("settings.presenceImageKeyHint")}
                  </span>
                  {imageKeyError ? (
                    <span className="presence-error">{imageKeyError}</span>
                  ) : null}
                </div>
              ) : null}

              <AssetEditor
                size="small"
                image={settings.small_image ?? ""}
                text={settings.small_text ?? ""}
                onImage={(small_image) => updateSettings({ small_image })}
                onText={(small_text) => updateSettings({ small_text })}
                useExeIcon={settings.use_exe_icon ?? true}
                onExeIcon={
                  platform?.os === "windows"
                    ? (use_exe_icon) => updateSettings({ use_exe_icon })
                    : undefined
                }
              />
              <div className="settings__row">
                <span className="settings__label">
                  {t("settings.presenceShowTimestamps")}
                </span>
                <div className="settings__control">
                  <Chip
                    selected={settings.show_timestamps}
                    onClick={() =>
                      updateSettings({ show_timestamps: !settings.show_timestamps })
                    }
                  >
                    {settings.show_timestamps
                      ? t("common.enabled")
                      : t("common.disabled")}
                  </Chip>
                </div>
              </div>

              <div className="settings__row">
                <span className="settings__label">
                  {t("settings.presenceShowWhenUnfocused")}
                </span>
                <div className="settings__control">
                  <Chip
                    selected={settings.show_when_unfocused}
                    onClick={() =>
                      updateSettings({
                        show_when_unfocused: !settings.show_when_unfocused,
                      })
                    }
                  >
                    {settings.show_when_unfocused
                      ? t("common.enabled")
                      : t("common.disabled")}
                  </Chip>
                </div>
              </div>
              <span className="settings__hint">
                {t("settings.presenceShowWhenUnfocusedHint")}
              </span>

              <div className="presence-mode">
                <span className="settings__label">
                  {t("settings.presenceButtonMode")}
                </span>
                <div className="settings__control">
                  <Chip
                    selected={!settings.use_custom_buttons}
                    onClick={() => updateSettings({ use_custom_buttons: false })}
                  >
                    {t("settings.presenceButtonModeDefault")}
                  </Chip>
                  <Chip
                    selected={settings.use_custom_buttons ?? false}
                    onClick={() => updateSettings({ use_custom_buttons: true })}
                  >
                    {t("settings.presenceButtonModeCustom")}
                  </Chip>
                </div>
              </div>
              <div className="settings__row">
                <span className="settings__label">
                  {t(
                    settings.use_custom_buttons
                      ? "settings.presenceShowCustomButtons"
                      : "settings.presenceShowButtons",
                  )}
                </span>
                <div className="settings__control">
                  <Chip
                    selected={settings.show_buttons}
                    onClick={() =>
                      updateSettings({ show_buttons: !settings.show_buttons })
                    }
                  >
                    {settings.show_buttons ? t("common.enabled") : t("common.disabled")}
                  </Chip>
                </div>
              </div>

              {settings.use_custom_buttons ? (
                <CustomButtonEditor
                  buttons={settings.custom_buttons ?? []}
                  onChange={(custom_buttons) => updateSettings({ custom_buttons })}
                />
              ) : (
                <>
                  <div className="settings__row">
                    <span className="settings__label">
                      {t("settings.presenceShowListButton")}
                    </span>
                    <div className="settings__control">
                      <Chip
                        selected={settings.show_list_button}
                        onClick={() =>
                          updateSettings({ show_list_button: !settings.show_list_button })
                        }
                      >
                        {settings.show_list_button
                          ? t("common.enabled")
                          : t("common.disabled")}
                      </Chip>
                    </div>
                  </div>

                  {settings.show_list_button ? (
                    <div className="settings__field">
                      <label htmlFor="my-vndb-link" className="settings__label">
                        {t("settings.presenceMyVndbLink")}
                      </label>
                      <input
                        id="my-vndb-link"
                        type="url"
                        className="input"
                        aria-invalid={linkError !== null}
                        value={settings.my_vndb_link}
                        onChange={(event) => {
                          setLinkError(null);
                          updateSettings({ my_vndb_link: event.target.value });
                        }}
                        placeholder="https://vndb.org/u123456"
                      />
                      {linkError ? (
                        <span className="presence-error">{linkError}</span>
                      ) : (
                        <span className="settings__hint">
                          {t("settings.presenceMyVndbLinkHint")}
                        </span>
                      )}
                    </div>
                  ) : null}
                </>
              )}

              <div className="settings__field">
                <label htmlFor="client-id" className="settings__label">
                  {t("settings.presenceClientId")}
                </label>
                <input
                  id="client-id"
                  type="text"
                  className="input"
                  value={settings.client_id}
                  onChange={(event) => updateSettings({ client_id: event.target.value })}
                  placeholder="1563096795532230756"
                />
                <span className="settings__hint">
                  {t("settings.presenceClientIdHint")}
                </span>
              </div>

              <div className="settings__field">
                <span className="settings__label">{t("settings.presenceScope")}</span>
                <span className="settings__hint">{t("settings.presenceScopeHint")}</span>
                <div className="settings__scope">
                  <select
                    className="input"
                    aria-label={t("settings.presenceScope")}
                    value={scopeGameId}
                    onChange={(event) => switchScope(event.target.value)}
                  >
                    <option value="">{t("settings.presenceScopeGlobal")}</option>
                    {games.map((option) => (
                      <option key={option.id} value={option.id}>
                        {option.title}
                      </option>
                    ))}
                  </select>
                  {scopeGameId ? (
                    <Button
                      onClick={() => void resetToGlobal()}
                      disabled={busy || scopeLoading || scopeError !== null}
                    >
                      {t("settings.presenceScopeResetButton")}
                    </Button>
                  ) : null}
                </div>
                {scopeLoading ? (
                  <span role="status" className="settings__hint">
                    {t("common.loading")}
                  </span>
                ) : null}
                {scopeError ? (
                  <div className="presence-error" role="alert">
                    {scopeError}
                    <Button onClick={() => switchScope(scopeGameId)}>
                      {t("common.retry")}
                    </Button>
                  </div>
                ) : null}
              </div>

              <div className="settings__field">
                <span className="settings__label">{t("settings.presencePreview")}</span>
                {games.length > 0 ? (
                  <label className="settings__preview-game">
                    <span className="settings__hint">
                      {t("settings.presencePreviewGame")}
                    </span>
                    <select
                      className="input"
                      value={previewGameId}
                      onChange={(event) => {
                        setPreviewGameId(event.target.value);
                        resetPreview();
                      }}
                    >
                      {games.map((option) => (
                        <option key={option.id} value={option.id}>
                          {option.title}
                        </option>
                      ))}
                    </select>
                  </label>
                ) : null}
                {renderCycleControls(
                  rotationLengths.details,
                  previewCycle,
                  setPreviewCycle,
                  t("settings.presencePreviewDetailsEntry"),
                )}
                {settings.details_mode === "custom"
                  ? renderCycleControls(
                      rotationLengths.state,
                      previewStateCycle,
                      setPreviewStateCycle,
                      t("settings.presencePreviewStateEntry"),
                    )
                  : null}
                <DiscordCardPreview
                  activity={preview}
                  label={t("settings.presencePlaying")}
                  defaultTitle={t("settings.presenceDefaultTitle")}
                  elapsedLabel={t("settings.presencePreviewElapsed")}
                  imagePlaceholder={t("settings.presenceRemoteImagePreview")}
                />
              </div>

              {saveError && !linkError ? (
                <span className="presence-error" role="alert">
                  {saveError}
                </span>
              ) : null}
              <div className="presence-actions">
                <Button
                  onClick={() => void save()}
                  disabled={busy || scopeLoading || scopeError !== null}
                >
                  {t("settings.presenceSave")}
                </Button>
                <Button
                  variant="secondary"
                  onClick={resetFormatting}
                  disabled={busy || scopeLoading || scopeError !== null}
                >
                  {t("settings.presenceResetDefaults")}
                </Button>
                {saved ? (
                  <span className="presence-saved">{t("settings.presenceSaved")}</span>
                ) : null}
              </div>
            </>
          ) : null}
        </div>
      </Card>

      <Sheet
        open={showTokenModal}
        title={t("settings.presenceTosTitle")}
        onClose={() => setShowTokenModal(false)}
        footer={
          <div className="presence-sheet-actions">
            <Button variant="ghost" onClick={() => setShowTokenModal(false)}>
              {t("common.cancel")}
            </Button>
            {tosAccepted ? (
              <Button
                variant="primary"
                disabled={tokenBusy || !tokenInput.trim()}
                onClick={() => void handleSaveToken()}
              >
                {t("settings.presenceSaveTokenBtn")}
              </Button>
            ) : (
              <Button variant="primary" onClick={() => setTosAccepted(true)}>
                {t("settings.presenceTosAccept")}
              </Button>
            )}
          </div>
        }
      >
        <div className="presence-sheet">
          <div className="presence-warning">
            <div className="presence-warning__head">
              <Icon name="alert" size={18} />
              <span>{t("settings.presenceTosWarningHead")}</span>
            </div>
            <p>{t("settings.presenceTosBody1")}</p>
            <p>{t("settings.presenceTosBody2")}</p>
            <p className="settings__hint">{t("settings.presenceTosBody3")}</p>
          </div>

          {tosAccepted ? (
            <div className="settings__field">
              <label htmlFor="discord-token-input" className="settings__label">
                {t("settings.presenceDiscordToken")}
              </label>
              <input
                id="discord-token-input"
                type="password"
                className="input"
                value={tokenInput}
                onChange={(event) => {
                  setTokenInput(event.target.value);
                  setTokenError(null);
                }}
                placeholder={t("settings.presenceTokenInputPlaceholder")}
                autoFocus
              />
              <span className="settings__hint">
                {t("settings.presenceTokenInputHelp")}
              </span>
              {tokenError ? <span className="presence-error">{tokenError}</span> : null}
            </div>
          ) : null}
        </div>
      </Sheet>
    </>
  );
}

/** Fixed Discord-palette preview card representing how the activity appears on Discord. */
function DiscordCardPreview({
  activity,
  label,
  defaultTitle,
  elapsedLabel,
  imagePlaceholder,
}: {
  activity: PresenceActivity | null;
  label: string;
  defaultTitle: string;
  elapsedLabel: string;
  imagePlaceholder: string;
}) {
  const buttons = (activity?.buttons ?? []).slice(0, MAX_BUTTONS);
  const localImage = (source?: string | null) =>
    source?.startsWith("data:image/") ? source : undefined;
  const largeImage = localImage(activity?.large_image);
  const smallImage = localImage(activity?.small_image);
  const hasRemoteImage =
    (activity?.large_image && !largeImage) || (activity?.small_image && !smallImage);

  return (
    <div className="dc-card">
      <div className="dc-card__label">{label}</div>
      <div className="dc-card__body">
        <div className="dc-card__art-wrap">
          <div
            className="dc-card__art"
            title={activity?.large_text ?? undefined}
            style={largeImage ? { backgroundImage: `url(${largeImage})` } : undefined}
          >
            {largeImage ? null : <Icon name="image" size={28} />}
          </div>
          {activity?.small_image ? (
            <div
              className="dc-card__small-art"
              role="img"
              aria-label={activity.small_text || imagePlaceholder}
              title={activity.small_text || imagePlaceholder}
              style={smallImage ? { backgroundImage: `url(${smallImage})` } : undefined}
            >
              {smallImage ? null : <Icon name="image" size={14} />}
            </div>
          ) : null}
        </div>

        <div className="dc-card__lines">
          <div className="dc-card__title" title={activity?.name ?? defaultTitle}>
            {activity?.name ?? defaultTitle}
          </div>
          {activity?.details ? (
            <div className="dc-card__details" title={activity.details}>
              {activity.details}
            </div>
          ) : null}
          {activity?.state ? (
            <div className="dc-card__state" title={activity.state}>
              {activity.state}
            </div>
          ) : null}
          {activity?.start_timestamp ? (
            <div className="dc-card__state">{elapsedLabel}</div>
          ) : null}
        </div>
      </div>

      {hasRemoteImage ? <p className="dc-card__image-hint">{imagePlaceholder}</p> : null}
      {buttons.length > 0 ? (
        <div className="dc-card__buttons">
          {buttons.map((button) => (
            <div key={`${button.label}-${button.url}`} className="dc-card__button">
              {button.label}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
