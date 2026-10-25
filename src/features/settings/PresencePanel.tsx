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
];

/** Mirrors `vn_core::presence::MIN_CYCLE_SECONDS`: Discord throttles presence to
 * roughly one update per 15 s, so a faster cycle would visibly skip entries. */
const MIN_CYCLE_SECONDS = 5;

/** Discord renders at most two buttons on an activity. */
const MAX_BUTTONS = 2;

type WhichField = "details" | "state" | "title";

/** Which transform each template field is paired with. */
type TransformKey =
  | "custom_details_transform"
  | "custom_state_transform"
  | "vndb_details_transform"
  | "vndb_state_transform";

/**
 * A rotation list: one template per line.
 *
 * Kept as text rather than a `string[]` while editing so a half-typed line is not
 * silently dropped on every keystroke, which is what made an empty entry
 * impossible to write and then delete.
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
        disabled={disabled}
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

export function PresencePanel() {
  const { t } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const platform = useAppStore((state) => state.platform);
  const isAndroid = platform?.os === "android";

  const [settings, setSettings] = useState<PresenceSettings | null>(null);
  const [status, setStatus] = useState<PresenceStatus | null>(null);
  const [preview, setPreview] = useState<PresenceActivity | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [activeField, setActiveField] = useState<WhichField>("details");
  // Which entry of each rotation list the preview is showing.
  const [previewCycle, setPreviewCycle] = useState(0);
  // Which game the preview renders. Until now the backend was always called
  // with no id, so the panel previewed the built-in sample no matter what the
  // user's library held -- {vn_name} could never show their own title.
  const [previewGameId, setPreviewGameId] = useState<string>("");
  const [games, setGames] = useState<{ id: string; title: string }[]>([]);
  const [previewStateCycle, setPreviewStateCycle] = useState(0);
  const [linkError, setLinkError] = useState<string | null>(null);

  // Discord Token state for Android Gateway
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
    void Promise.all([
      api.presenceGetSettings(),
      api.presenceStatus(),
      api.presenceHasToken(),
      api.presenceTokenPreview(),
      api.presenceImageKeyPreview(),
    ])
      .then(([initialSettings, initialStatus, tokenExists, previewStr, keyPreview]) => {
        if (!active) return;
        setSettings(initialSettings);
        setStatus(initialStatus);
        setHasToken(tokenExists);
        setTokenPreview(previewStr);
        setImageKeyPreview(keyPreview);
      })
      .catch(() => undefined);

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
  }, []);

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
        // Default to the first game so the preview is about a real title from
        // the moment the panel opens.
        setPreviewGameId((current) => current || (options[0]?.id ?? ""));
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, []);

  // Update live preview when settings or the previewed rotation entry change.
  useEffect(() => {
    if (!settings) return;
    let active = true;
    void api
      .presencePreview(
        settings,
        previewGameId || undefined,
        previewCycle,
        previewStateCycle,
      )
      .then((next) => {
        if (active) setPreview(next);
      })
      .catch(() => undefined);

    return () => {
      active = false;
    };
  }, [settings, previewCycle, previewStateCycle, previewGameId]);

  // A saved-to-play switch is not a field; the reset keeps the preview honest about
  // where a fresh rotation starts.
  const resetPreview = useCallback(() => {
    setPreviewCycle(0);
    setPreviewStateCycle(0);
  }, []);

  // How many entries each line can actually rotate through, or zero when the field
  // is not rotating. Zero hides the pager rather than offering to step through a
  // list the card is currently ignoring.
  const rotationLengths = useMemo(() => {
    if (!settings) return { details: 0, state: 0 };
    const usable = (list: string[]) => list.filter((entry) => entry.trim() !== "").length;
    const isVndb = settings.details_mode === "vndb";
    const detailsRotating = isVndb
      ? true
      : settings.custom_details_rotate && settings.custom_details_items.length > 0;
    return {
      details: detailsRotating
        ? usable(isVndb ? settings.vndb_details_items : settings.custom_details_items)
        : 0,
      state: settings.custom_state_rotate ? usable(settings.custom_state_items) : 0,
    };
  }, [settings]);

  if (!settings) return null;

  const updateSettings = (patch: Partial<PresenceSettings>) => {
    setSaved(false);
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

  const save = async () => {
    setBusy(true);
    setSaved(false);
    setLinkError(null);
    try {
      await api.presenceSaveSettings(settings);
      setSaved(true);
      setStatus(await api.presenceStatus());
      pushToast("info", t("settings.presenceSaved"));
    } catch (caught) {
      const message = caught instanceof IpcError ? caught.message : t("common.error");
      if (caught instanceof IpcError && caught.code === "invalid_profile_link") {
        setLinkError(message);
      }
      pushToast("error", message);
    } finally {
      setBusy(false);
    }
  };

  const setMode = (mode: PresenceDetailsMode) => {
    resetPreview();
    updateSettings({ details_mode: mode });
  };

  const insertToken = (token: string) => {
    if (activeField === "title") {
      updateSettings({ activity_name: (settings.activity_name ?? "") + token });
    } else if (activeField === "details") {
      const key =
        settings.details_mode === "vndb" ? "vndb_state_template" : "custom_details";
      updateSettings({ [key]: ((settings[key] as string) ?? "") + token });
    } else {
      updateSettings({ custom_state: settings.custom_state + token });
    }
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
              {/* Stacked rather than a label/control row: three mode chips are
                  wider than a 360 px phone's content box, so side by side they
                  overflowed the card. */}
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

              {/* The title line is what Discord bolds at the top of the card. */}
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
                  <div className="settings__row">
                    <span className="settings__label">
                      {t("settings.presenceCycleDetails")}
                    </span>
                    <div className="settings__control">
                      <Chip
                        selected={settings.cycle_vndb_details}
                        onClick={() =>
                          updateSettings({
                            cycle_vndb_details: !settings.cycle_vndb_details,
                          })
                        }
                      >
                        {settings.cycle_vndb_details
                          ? t("common.enabled")
                          : t("common.disabled")}
                      </Chip>
                    </div>
                  </div>
                  <span className="settings__hint">
                    {t("settings.presenceCycleDetailsHint")}
                  </span>

                  {/* The list owns the interval and the on/off switch for this
                      mode: a separate "Seconds per fact" field above it was the
                      same value rendered twice, and only one of the two was on
                      screen once rotation was switched off. */}
                  <RotationListEditor
                    id="vndb-details-items"
                    label={t("settings.presenceVndbDetailsItems")}
                    hint={t("settings.presenceVndbDetailsItemsHint")}
                    items={settings.vndb_details_items}
                    rotate={settings.cycle_vndb_details}
                    rotateLabel={t("settings.presenceCycleDetails")}
                    seconds={settings.cycle_seconds}
                    secondsLabel={t("settings.presenceCycleSeconds")}
                    onItems={(next) => {
                      resetPreview();
                      updateSettings({ vndb_details_items: next });
                    }}
                    onRotate={(next) => updateSettings({ cycle_vndb_details: next })}
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
                      onFocus={() => setActiveField("details")}
                      onChange={(event) =>
                        updateSettings({ vndb_state_template: event.target.value })
                      }
                      placeholder="★ {vndb_rating}/10 · {vndb_year} · {vndb_length}"
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
                <>
                  {/* Opt-in, EwE-style: a host lets the card show art Discord
                      cannot fetch itself (explicit covers as a local blur,
                      custom covers). "none" keeps every image local. */}
                  <div className="presence-mode">
                    <span className="settings__label">
                      {t("settings.presenceImageHost")}
                    </span>
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
                          <code className="presence-token-preview">
                            {imageKeyPreview}
                          </code>
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
                </>
              ) : null}

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

              <div className="settings__row">
                <span className="settings__label">
                  {t("settings.presenceShowButtons")}
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
                />
              </div>

              <div className="presence-actions">
                <Button onClick={() => void save()} disabled={busy}>
                  {t("settings.presenceSave")}
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

/**
 * The card as Discord draws it.
 *
 * Deliberately hard-coded to Discord's own palette: this is a picture of another
 * application, and rendering it in the app's theme tokens would make the preview
 * a lie about what the user will see on their profile.
 */
function DiscordCardPreview({
  activity,
  label,
  defaultTitle,
  elapsedLabel,
}: {
  activity: PresenceActivity | null;
  label: string;
  defaultTitle: string;
  elapsedLabel: string;
}) {
  const buttons = (activity?.buttons ?? []).slice(0, MAX_BUTTONS);

  return (
    <div className="dc-card">
      <div className="dc-card__label">{label}</div>
      <div className="dc-card__body">
        <div
          className="dc-card__art"
          style={
            activity?.large_image
              ? { backgroundImage: `url(${activity.large_image})` }
              : undefined
          }
        >
          {activity?.large_image ? null : <Icon name="image" size={28} />}
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
