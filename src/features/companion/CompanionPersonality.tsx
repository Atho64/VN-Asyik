import { useEffect, useState } from "react";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import type { Translator } from "@/i18n";
import {
  companionApi,
  type ProfileCatalogue,
  type ProfileSettings,
  type ProfileSnapshot,
  type SessionOptions,
} from "@/lib/ipc/companion";
import { companionError } from "./errors";
import { failureCode, performAction } from "./store";

export function personalityName(t: Translator, options: SessionOptions): string {
  const profile = options.profile;
  if (!profile) return t(`vnchat.persona.${options.persona}`);
  return profile.id.startsWith("20b00000-0000-4000-8000-")
    ? t(`companion.preset.${profile.settings.preset}`)
    : profile.settings.name;
}

const keyOf = (profile: ProfileSnapshot) => `${profile.id}:${profile.version}`;

/** A local editor, also safe to place inside the setup form or options sheet. */
export function CompanionPersonality({
  value,
  persona,
  disabled = false,
  onChange,
  onEditingChange,
}: {
  value: ProfileSnapshot | null | undefined;
  persona: SessionOptions["persona"];
  disabled?: boolean;
  onChange: (profile: ProfileSnapshot) => void;
  onEditingChange?: (editing: boolean) => void;
}) {
  const { t } = useI18n();
  const [catalogue, setCatalogue] = useState<ProfileCatalogue | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [busy, setBusy] = useState(false);
  const [editor, setEditor] = useState<{
    original: ProfileSnapshot;
    settings: ProfileSettings;
    custom: boolean;
  } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    let alive = true;
    const refresh = () => {
      void companionApi
        .profiles()
        .then((next) => {
          if (alive) setCatalogue(next);
        })
        .catch((reason) => {
          if (alive) setError(failureCode(reason));
        });
    };
    refresh();
    window.addEventListener("focus", refresh);
    return () => {
      alive = false;
      window.removeEventListener("focus", refresh);
    };
  }, [reload]);

  const selected = value ?? catalogue?.presets.find((p) => p.settings.preset === persona);
  const choices = catalogue ? [...catalogue.presets, ...catalogue.profiles] : [];
  const detached = selected && !choices.some((p) => keyOf(p) === keyOf(selected));
  const blocked = disabled || busy;

  function edit() {
    if (!selected) return;
    setError(null);
    setConfirmDelete(false);
    onEditingChange?.(true);
    setEditor({
      original: selected,
      settings: { ...selected.settings },
      custom: Boolean(catalogue?.profiles.some((p) => p.id === selected.id)),
    });
  }

  function closeEditor() {
    setEditor(null);
    onEditingChange?.(false);
  }

  async function save() {
    if (!editor || blocked) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await companionApi.saveProfile(
        editor.custom ? editor.original.id : null,
        editor.custom ? editor.original.version : null,
        { ...editor.settings, name: editor.settings.name.trim() },
      );
      setCatalogue(
        (previous) =>
          previous && {
            ...previous,
            profiles: [saved, ...previous.profiles.filter((p) => p.id !== saved.id)],
          },
      );
      onChange(saved);
      closeEditor();
    } catch (reason) {
      setError(failureCode(reason));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!editor || blocked) return;
    setBusy(true);
    setError(null);
    try {
      await companionApi.deleteProfile(editor.original.id, editor.original.version);
      setCatalogue(
        (previous) =>
          previous && {
            ...previous,
            profiles: previous.profiles.filter((p) => p.id !== editor.original.id),
          },
      );
      // Keep the selected snapshot. Deleting a saved identity does not change a session.
      closeEditor();
      setConfirmDelete(false);
    } catch (reason) {
      setError(failureCode(reason));
    } finally {
      setBusy(false);
    }
  }

  function update<K extends keyof ProfileSettings>(key: K, next: ProfileSettings[K]) {
    setEditor(
      (current) =>
        current && { ...current, settings: { ...current.settings, [key]: next } },
    );
  }

  return (
    <div className="companion__personality">
      <label className="companion__field">
        <span>{t("companion.personality")}</span>
        <select
          disabled={blocked || Boolean(editor) || !catalogue}
          value={selected ? keyOf(selected) : ""}
          onChange={(event) => {
            const next = choices.find((p) => keyOf(p) === event.target.value);
            if (next) onChange(next);
          }}
        >
          {!selected && <option value="">{t(`vnchat.persona.${persona}`)}</option>}
          {detached && (
            <option value={keyOf(selected)}>
              {selected.settings.name} ·{" "}
              {t("companion.profile.savedVersion", { version: selected.version })}
            </option>
          )}
          {catalogue && (
            <optgroup label={t("companion.profile.presets")}>
              {catalogue.presets.map((p) => (
                <option key={p.id} value={keyOf(p)}>
                  {t(`companion.preset.${p.settings.preset}`)}
                </option>
              ))}
            </optgroup>
          )}
          {Boolean(catalogue?.profiles.length) && (
            <optgroup label={t("companion.profile.custom")}>
              {catalogue?.profiles.map((p) => (
                <option key={p.id} value={keyOf(p)}>
                  {p.settings.name}
                </option>
              ))}
            </optgroup>
          )}
        </select>
      </label>
      {!editor && (
        <Button
          variant="ghost"
          disabled={blocked || !catalogue || !selected}
          onClick={edit}
        >
          {t("companion.profile.customize")}
        </Button>
      )}
      {error && (
        <div role="alert" className="companion__error">
          <p>{companionError(t, error)}</p>
          <Button
            disabled={busy}
            onClick={() => {
              setError(null);
              setReload((n) => n + 1);
            }}
          >
            {t("companion.profile.reload")}
          </Button>
        </div>
      )}
      {editor && (
        <div
          className="companion__profile-editor"
          onKeyDown={(event) => {
            // This editor also lives inside setup's form. Enter in a name field
            // must not start a session with unsaved preferences.
            if (event.key === "Enter" && event.target instanceof HTMLInputElement)
              event.preventDefault();
          }}
        >
          <div className="companion__fields">
            <label className="companion__field">
              <span>{t("companion.profile.name")}</span>
              <input
                maxLength={60}
                value={editor.settings.name}
                disabled={blocked}
                onChange={(e) => update("name", e.target.value)}
              />
            </label>
            <label className="companion__field">
              <span>{t("companion.profile.base")}</span>
              <select
                value={editor.settings.preset}
                disabled={blocked}
                onChange={(e) =>
                  update("preset", e.target.value as ProfileSettings["preset"])
                }
              >
                {catalogue?.presets.map((p) => (
                  <option key={p.id} value={p.settings.preset}>
                    {t(`companion.preset.${p.settings.preset}`)}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <label className="companion__field">
            <span>{t("companion.profile.style")}</span>
            <textarea
              rows={3}
              maxLength={1000}
              value={editor.settings.style}
              disabled={blocked}
              onChange={(e) => update("style", e.target.value)}
            />
          </label>
          <details className="companion__details">
            <summary>{t("companion.profile.tuning")}</summary>
            <div className="companion__fields">
              <label className="companion__field">
                <span>{t("companion.profile.length")}</span>
                <select
                  value={editor.settings.length}
                  disabled={blocked}
                  onChange={(e) =>
                    update("length", e.target.value as ProfileSettings["length"])
                  }
                >
                  {(["short", "balanced", "detailed"] as const).map((v) => (
                    <option key={v} value={v}>
                      {t(`companion.profile.length.${v}`)}
                    </option>
                  ))}
                </select>
              </label>
              <label className="companion__field">
                <span>{t("companion.profile.language")}</span>
                <select
                  value={editor.settings.language}
                  disabled={blocked}
                  onChange={(e) =>
                    update("language", e.target.value as ProfileSettings["language"])
                  }
                >
                  {(["app", "en", "id", "ja"] as const).map((v) => (
                    <option key={v} value={v}>
                      {t(`companion.profile.language.${v}`)}
                    </option>
                  ))}
                </select>
              </label>
              {(["humor", "energy", "warmth", "questions"] as const).map((field) => (
                <label key={field} className="companion__field">
                  <span>{t(`companion.profile.${field}`)}</span>
                  <select
                    value={editor.settings[field]}
                    disabled={blocked}
                    onChange={(e) => update(field, Number(e.target.value))}
                  >
                    {[0, 1, 2].map((v) => (
                      <option key={v} value={v}>
                        {t(`companion.profile.level.${v}`)}
                      </option>
                    ))}
                  </select>
                </label>
              ))}
            </div>
            <label className="companion__field">
              <span>{t("companion.profile.interests")}</span>
              <textarea
                rows={2}
                maxLength={500}
                value={editor.settings.interests}
                disabled={blocked}
                onChange={(e) => update("interests", e.target.value)}
              />
            </label>
          </details>
          <details className="companion__details">
            <summary>{t("companion.profile.example")}</summary>
            <p className="companion__muted">{t("companion.profile.exampleHint")}</p>
            <blockquote className="companion__dialogue">
              {t("companion.profile.passage")}
            </blockquote>
            <p>{t(`companion.example.${editor.settings.preset}`)}</p>
          </details>
          <div className="companion__actions">
            <Button
              variant="primary"
              disabled={blocked || !editor.settings.name.trim()}
              onClick={() => void save()}
            >
              {t("companion.profile.save")}
            </Button>
            <Button disabled={blocked} onClick={closeEditor}>
              {t("common.cancel")}
            </Button>
            {editor.custom && (
              <Button
                variant="ghost"
                disabled={blocked}
                onClick={() => setConfirmDelete(true)}
              >
                {t("companion.profile.delete")}
              </Button>
            )}
          </div>
          {confirmDelete && (
            <div className="companion__notice">
              <p>{t("companion.profile.deleteHint")}</p>
              <div className="companion__actions">
                <Button variant="danger" disabled={blocked} onClick={() => void remove()}>
                  {t("companion.profile.confirmDelete")}
                </Button>
                <Button disabled={blocked} onClick={() => setConfirmDelete(false)}>
                  {t("common.cancel")}
                </Button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function SessionPersonality({
  id,
  options,
  disabled,
}: {
  id: string;
  options: SessionOptions;
  disabled: boolean;
}) {
  const { t } = useI18n();
  const [selected, setSelected] = useState(options.profile);
  const [editing, setEditing] = useState(false);
  const changed =
    selected && JSON.stringify(selected) !== JSON.stringify(options.profile);
  return (
    <div className="companion__options-section">
      <CompanionPersonality
        value={selected}
        persona={options.persona}
        disabled={disabled}
        onChange={setSelected}
        onEditingChange={setEditing}
      />
      {changed && !editing && (
        <>
          <p className="companion__muted">{t("companion.profile.applyHint")}</p>
          <Button
            disabled={disabled}
            onClick={() =>
              void performAction(() => companionApi.applyProfile(id, selected))
            }
          >
            {t("companion.profile.apply")}
          </Button>
        </>
      )}
    </div>
  );
}
