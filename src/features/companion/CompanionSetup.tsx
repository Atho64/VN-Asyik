import { useEffect, useState } from "react";
import { Link } from "react-router";
import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { lunaApi } from "@/lib/ipc/luna";
import type { SessionOptions } from "@/lib/ipc/companion";
import { CompanionPersonality } from "./CompanionPersonality";

export function CompanionSetup({
  gameId,
  initial,
  replacing,
  pending,
  onStart,
  onCancel,
}: {
  gameId: string | null;
  initial?: SessionOptions | undefined;
  replacing: boolean;
  pending: boolean;
  onStart: (options: SessionOptions) => Promise<void>;
  onCancel?: (() => void) | undefined;
}) {
  const { t, locale } = useI18n();
  const cards = useAppStore((state) => state.cards);
  const [options, setOptions] = useState<SessionOptions>(() => ({
    gameId: gameId ?? initial?.gameId ?? "",
    playthroughLabel: t(
      replacing ? "companion.newContextLabel" : "companion.firstReading",
    ),
    knowledge: initial?.knowledge ?? "observed",
    boundary: initial?.boundary ?? "",
    source: initial?.source ?? "manual",
    language: locale,
    persona: initial?.persona ?? "companion",
    profile: initial?.profile,
  }));
  const [lunaConnected, setLunaConnected] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [editingProfile, setEditingProfile] = useState(false);

  useEffect(() => {
    let alive = true;
    const refresh = () => {
      void lunaApi
        .status()
        .then((status) => {
          if (alive) setLunaConnected(status.enabled && status.connected);
        })
        .catch(() => {
          if (alive) setLunaConnected(false);
        });
    };
    refresh();
    const poll = setInterval(refresh, 2_000);
    return () => {
      alive = false;
      clearInterval(poll);
    };
  }, []);

  const ready =
    !editingProfile &&
    Boolean(options.gameId && options.playthroughLabel.trim()) &&
    (options.source === "manual" || (lunaConnected && confirmed));

  return (
    <form
      className="companion__setup"
      onSubmit={(event) => {
        event.preventDefault();
        if (ready && !pending) void onStart({ ...options, language: locale });
      }}
    >
      {replacing && <p className="companion__muted">{t("companion.replaceHint")}</p>}
      <div className="companion__fields">
        <label className="companion__field">
          <span>{t("companion.game")}</span>
          <select
            required
            value={options.gameId}
            onChange={(event) => {
              setOptions({ ...options, gameId: event.target.value });
              setConfirmed(false);
            }}
          >
            <option value="">{t("companion.chooseGame")}</option>
            {cards.map(({ game }) => (
              <option key={game.meta.id} value={game.meta.id}>
                {game.title}
              </option>
            ))}
          </select>
        </label>
        <CompanionPersonality
          value={options.profile}
          persona={options.persona}
          disabled={pending}
          onChange={(profile) => setOptions({ ...options, profile })}
          onEditingChange={setEditingProfile}
        />
        <label className="companion__field">
          <span id="companion-knowledge-label">{t("companion.knowledge")}</span>
          <select
            value={options.knowledge}
            aria-labelledby="companion-knowledge-label"
            aria-describedby="companion-knowledge-hint"
            onChange={(event) =>
              setOptions({
                ...options,
                knowledge: event.target.value as SessionOptions["knowledge"],
              })
            }
          >
            {(["metadata", "observed", "all"] as const).map((knowledge) => (
              <option key={knowledge} value={knowledge}>
                {t(`companion.knowledge.${knowledge}`)}
              </option>
            ))}
          </select>
          <small id="companion-knowledge-hint">
            {t(`companion.knowledgeHint.${options.knowledge}`)}
          </small>
        </label>
        <label className="companion__field">
          <span>{t("companion.source")}</span>
          <select
            value={options.source}
            onChange={(event) => {
              setOptions({
                ...options,
                source: event.target.value as SessionOptions["source"],
              });
              setConfirmed(false);
            }}
          >
            <option value="manual">{t("companion.manual")}</option>
            <option value="luna">{t("companion.luna")}</option>
          </select>
        </label>
      </div>
      {options.source === "luna" && (
        <div className="companion__sources">
          {!lunaConnected && (
            <p className="companion__muted">
              {t("companion.lunaUnavailable")}{" "}
              <Link to="/settings">{t("companion.openSettings")}</Link>
            </p>
          )}
          <label className="companion__choice">
            <input
              type="checkbox"
              checked={confirmed}
              disabled={!lunaConnected}
              onChange={(event) => setConfirmed(event.target.checked)}
            />
            <span>{t("companion.confirmLuna")}</span>
          </label>
        </div>
      )}
      <details className="companion__details" open={replacing || undefined}>
        <summary>{t("companion.readingOptions")}</summary>
        <div className="companion__fields">
          <label className="companion__field">
            <span>{t("companion.readingLabel")}</span>
            <input
              required
              maxLength={80}
              value={options.playthroughLabel}
              onChange={(event) =>
                setOptions({ ...options, playthroughLabel: event.target.value })
              }
              onInvalid={(event) => {
                const details = event.currentTarget.closest("details");
                if (details) details.open = true;
              }}
            />
          </label>
          <label className="companion__field">
            <span>{t("companion.boundary")}</span>
            <input
              maxLength={500}
              value={options.boundary}
              placeholder={t("companion.boundaryPlaceholder")}
              onChange={(event) =>
                setOptions({ ...options, boundary: event.target.value })
              }
            />
          </label>
        </div>
      </details>
      <p className="companion__muted">{t("companion.privacy")}</p>
      <div className="companion__actions">
        <Button type="submit" variant="primary" disabled={!ready || pending}>
          {t(
            pending
              ? "companion.starting"
              : replacing
                ? "companion.replaceStart"
                : "companion.start",
          )}
        </Button>
        {onCancel && (
          <Button variant="ghost" disabled={pending} onClick={onCancel}>
            {t("common.cancel")}
          </Button>
        )}
        {cards.length === 0 && <Link to="/">{t("companion.addGame")}</Link>}
      </div>
    </form>
  );
}
