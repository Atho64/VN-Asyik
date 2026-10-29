import { useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { api, IpcError } from "@/lib/ipc/client";
import type { Translator } from "@/i18n";
import type { LlmProfile, LlmProfiles, LlmProvider } from "@/lib/ipc/types";

/**
 * The optional chat provider, as a list of saved endpoints.
 *
 * Optional is the operative word: the taste profile works without this. The panel
 * says so, because a settings screen that implies the feature needs a key would
 * push users toward a paid API for something that runs locally for free.
 *
 * A profile is a name, an endpoint and a model; its key is stored separately, in
 * the credential store, so switching endpoints does not mean retyping keys and no
 * key is ever written to the database.
 */

/**
 * Translate a rejected call.
 *
 * The backend answers a broken field with a stable code, which is what the panel
 * translates; the message it also carries is the English fallback for a code this
 * build does not know yet.
 */
function describeError(caught: unknown, t: Translator): string {
  if (!(caught instanceof IpcError)) return t("common.error");
  const key = `chat.error.${caught.code}`;
  const translated = t(key);
  return translated === key ? caught.message : translated;
}

/** The fields the form edits. A missing id means "add a new profile". */
interface Draft {
  id?: string;
  provider: LlmProvider;
  name: string;
  baseUrl: string;
  model: string;
  key: string;
}

/**
 * Starting points for the form. Not a whitelist: every field stays editable, and
 * the saved profile is whatever the user leaves in the boxes. The two providers
 * differ in wire format, which is why the preset also sets `provider`; Gemini
 * speaks the OpenAI shape through its compatibility endpoint.
 */
const PRESETS: {
  id: string;
  provider: LlmProvider;
  name: string;
  baseUrl: string;
  model: string;
}[] = [
  {
    id: "openai",
    provider: "openai",
    name: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
  },
  {
    id: "gemini",
    provider: "openai",
    name: "Gemini",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    model: "gemini-2.5-flash",
  },
  {
    id: "anthropic",
    provider: "anthropic",
    name: "Anthropic",
    baseUrl: "https://api.anthropic.com/v1",
    model: "claude-sonnet-4-5",
  },
  {
    id: "local",
    provider: "openai",
    name: "Local",
    baseUrl: "http://127.0.0.1:11434/v1",
    model: "qwen3",
  },
];

const emptyDraft = (): Draft => ({
  provider: "openai",
  name: "",
  baseUrl: "https://api.openai.com/v1",
  model: "",
  key: "",
});

export function ChatPanel() {
  const { t } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);

  const [view, setView] = useState<LlmProfiles | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const next = await api.aiLlmProfiles();
        if (!cancelled) setView(next);
      } catch (caught) {
        if (!cancelled) setError(describeError(caught, t));
      }
    })();
    return () => {
      cancelled = true;
    };
    // `t` is read inside the error mapping, so a locale change must re-run it.
  }, [t]);

  async function run(action: () => Promise<LlmProfiles>): Promise<boolean> {
    setBusy(true);
    setError(null);
    try {
      setView(await action());
      return true;
    } catch (caught) {
      setError(describeError(caught, t));
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function save(): Promise<void> {
    if (!draft) return;
    const trimmedKey = draft.key.trim();
    const input = {
      ...(draft.id ? { id: draft.id } : {}),
      name: draft.name,
      provider: draft.provider,
      baseUrl: draft.baseUrl,
      model: draft.model,
      // An empty box keeps the stored key, so editing a model never means
      // re-entering a secret the user cannot read back.
      ...(trimmedKey.length > 0 ? { key: trimmedKey } : {}),
    };
    if (await run(() => api.aiLlmSaveProfile(input))) {
      setDraft(null);
      pushToast("success", t("chat.profileSaved"));
    }
  }

  async function activate(id: string): Promise<void> {
    if (await run(() => api.aiLlmActivateProfile(id))) {
      pushToast("success", t("chat.profileActivated"));
    }
  }

  async function remove(id: string): Promise<void> {
    setConfirming(null);
    if (await run(() => api.aiLlmDeleteProfile(id))) {
      if (draft?.id === id) setDraft(null);
      pushToast("success", t("chat.profileDeleted"));
    }
  }

  async function clearKey(id: string): Promise<void> {
    if (await run(() => api.aiLlmClearKey(id))) {
      pushToast("success", t("chat.keyRemoved"));
    }
  }

  const editing = draft?.id
    ? (view?.profiles.find((profile) => profile.id === draft.id) ?? null)
    : null;

  return (
    <Card>
      <CardHeader title={t("chat.title")} />
      <p className="card__intro">{t("chat.intro")}</p>

      {!view ? null : !view.supported ? (
        <p className="notice" role="status">
          {t("chat.unsupported")}
        </p>
      ) : (
        <>
          {view.profiles.length === 0 ? (
            <p className="settings__hint">{t("chat.profilesEmpty")}</p>
          ) : (
            <ul className="llm-profiles" aria-label={t("chat.profiles")}>
              {view.profiles.map((profile) => (
                <ProfileRow
                  key={profile.id}
                  profile={profile}
                  active={profile.id === view.active}
                  busy={busy}
                  confirming={confirming === profile.id}
                  onActivate={() => void activate(profile.id)}
                  onEdit={() =>
                    setDraft({
                      id: profile.id,
                      provider: profile.provider,
                      name: profile.name,
                      baseUrl: profile.baseUrl,
                      model: profile.model,
                      key: "",
                    })
                  }
                  onClearKey={() => void clearKey(profile.id)}
                  onAskDelete={() => setConfirming(profile.id)}
                  onCancelDelete={() => setConfirming(null)}
                  onDelete={() => void remove(profile.id)}
                />
              ))}
            </ul>
          )}

          {draft ? (
            <div className="llm-form">
              <p className="settings__hint">
                {editing ? t("chat.editProfile") : t("chat.addProfile")}
              </p>
              <div className="llm-form__presets">
                <span className="settings__hint">{t("chat.preset")}</span>
                <div className="llm-form__actions">
                  {PRESETS.map((preset) => (
                    <Chip
                      key={preset.id}
                      selected={
                        draft.provider === preset.provider &&
                        draft.baseUrl === preset.baseUrl
                      }
                      onClick={() =>
                        setDraft({
                          ...draft,
                          provider: preset.provider,
                          name: draft.name.trim().length > 0 ? draft.name : preset.name,
                          baseUrl: preset.baseUrl,
                          model: preset.model,
                        })
                      }
                    >
                      {preset.name}
                    </Chip>
                  ))}
                </div>
              </div>
              <label className="field">
                <span className="field__label">{t("chat.profileName")}</span>
                <input
                  className="input"
                  value={draft.name}
                  onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                  placeholder={t("chat.profileNamePlaceholder")}
                  spellCheck={false}
                />
              </label>
              <label className="field">
                <span className="field__label">{t("chat.baseUrl")}</span>
                <input
                  className="input"
                  value={draft.baseUrl}
                  onChange={(event) =>
                    setDraft({ ...draft, baseUrl: event.target.value })
                  }
                  spellCheck={false}
                  placeholder="https://api.openai.com/v1"
                />
                <span className="field__hint">{t("chat.baseUrlHint")}</span>
              </label>
              <label className="field">
                <span className="field__label">{t("chat.model")}</span>
                <input
                  className="input"
                  value={draft.model}
                  onChange={(event) => setDraft({ ...draft, model: event.target.value })}
                  spellCheck={false}
                  placeholder={t("chat.modelPlaceholder")}
                />
              </label>
              <label className="field">
                <span className="field__label">{t("chat.key")}</span>
                <input
                  className="input"
                  type="password"
                  value={draft.key}
                  onChange={(event) => setDraft({ ...draft, key: event.target.value })}
                  spellCheck={false}
                  autoComplete="off"
                  placeholder={
                    editing?.hasKey
                      ? (editing.keyHint ?? t("chat.keyStored"))
                      : t("chat.keyPlaceholder")
                  }
                />
                <span className="field__hint">{t("chat.keyHint")}</span>
              </label>
              <div className="llm-form__actions">
                <Button
                  variant="primary"
                  onClick={() => void save()}
                  disabled={busy}
                >
                  {busy ? t("chat.saving") : t("chat.save")}
                </Button>
                <Button onClick={() => setDraft(null)} disabled={busy}>
                  {t("common.cancel")}
                </Button>
              </div>
            </div>
          ) : (
            <Button
              onClick={() => setDraft(emptyDraft())}
              disabled={busy || view.profiles.length >= 10}
            >
              {t("chat.addProfile")}
            </Button>
          )}
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

interface RowProps {
  profile: LlmProfile;
  active: boolean;
  busy: boolean;
  confirming: boolean;
  onActivate: () => void;
  onEdit: () => void;
  onClearKey: () => void;
  onAskDelete: () => void;
  onCancelDelete: () => void;
  onDelete: () => void;
}

/** One saved endpoint. */
function ProfileRow({
  profile,
  active,
  busy,
  confirming,
  onActivate,
  onEdit,
  onClearKey,
  onAskDelete,
  onCancelDelete,
  onDelete,
}: RowProps) {
  const { t } = useI18n();
  return (
    <li className={active ? "llm-profile llm-profile--active" : "llm-profile"}>
      <div className="llm-profile__head">
        <span className="llm-profile__name">{profile.name}</span>
        {active ? <Chip selected>{t("chat.active")}</Chip> : null}
        <Chip selected={profile.hasKey}>
          {profile.hasKey ? t("chat.keySet") : t("chat.noKey")}
        </Chip>
      </div>
      {/* Model and endpoint on their own lines: a bullet separator that wraps to
          the end of a line reads as part of the endpoint. */}
      <p className="llm-profile__meta">
        {profile.provider === "anthropic"
          ? t("chat.providerAnthropic")
          : t("chat.providerOpenai")}{" "}
        · {profile.model}
      </p>
      <p className="llm-profile__meta">{profile.baseUrl}</p>
      <div className="llm-profile__actions">
        {active ? null : (
          <Button onClick={onActivate} disabled={busy}>
            {t("chat.useProfile")}
          </Button>
        )}
        <Button onClick={onEdit} disabled={busy}>
          {t("chat.editProfile")}
        </Button>
        {profile.hasKey ? (
          <Button onClick={onClearKey} disabled={busy}>
            {t("chat.removeKey")}
          </Button>
        ) : null}
        {confirming ? (
          <>
            <Button variant="danger" onClick={onDelete} disabled={busy}>
              {t("chat.deleteConfirm")}
            </Button>
            <Button onClick={onCancelDelete} disabled={busy}>
              {t("common.cancel")}
            </Button>
          </>
        ) : (
          <Button onClick={onAskDelete} disabled={busy}>
            {t("chat.deleteProfile")}
          </Button>
        )}
      </div>
    </li>
  );
}
