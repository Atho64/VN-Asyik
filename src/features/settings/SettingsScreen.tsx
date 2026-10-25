import { useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { Icon } from "@/components/Icon";
import { ChatPanel } from "@/features/settings/ChatPanel";
import { LocalePanel } from "@/features/settings/LocalePanel";
import { LunaPanel } from "@/features/settings/LunaPanel";
import { PresencePanel } from "@/features/settings/PresencePanel";
import { TitleParsingPanel } from "@/features/settings/TitleParsingPanel";
import { SyncScreen } from "@/features/sync/SyncScreen";
import { DeviceSyncScreen } from "@/features/sync/DeviceSyncScreen";
import { api, IpcError } from "@/lib/ipc/client";
import { statusOrder } from "@/lib/library";
import type {
  AfkConfig,
  ConflictPolicy,
  PlatformInfo,
  RefreshAllOutcome,
  Support,
  SyncSettings,
} from "@/lib/ipc/types";

export type SettingsTab = "general" | "vndb" | "transfer";

/**
 * Settings screen consolidating General preferences, VNDB Sync, and Device
 * Data Transfer & Backup into a single place.
 */
export function SettingsScreen({ initialTab }: { initialTab?: SettingsTab } = {}) {
  const { t } = useI18n();
  const prefs = useAppStore((state) => state.prefs);
  const setPrefs = useAppStore((state) => state.setPrefs);
  const platform = useAppStore((state) => state.platform);
  const pushToast = useAppStore((state) => state.pushToast);
  const [tab, setTab] = useState<SettingsTab>(initialTab ?? "general");
  const [afk, setAfk] = useState<AfkConfig | null>(null);
  const [syncSettings, setSyncSettings] = useState<SyncSettings | null>(null);

  useEffect(() => {
    let cancelled = false;
    void api
      .getAfkConfig()
      .then((config) => {
        if (!cancelled) setAfk(config);
      })
      .catch(() => undefined);
    void api
      .vndbSyncSettings()
      .then((settings) => {
        if (!cancelled) setSyncSettings(settings);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  const apply = async (action: () => Promise<typeof prefs>): Promise<void> => {
    try {
      setPrefs(await action());
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    }
  };

  const isWindows = platform?.os === "windows";
  const canUsePresence = isWindows || platform?.os === "android" || platform?.discord_presence === "supported";

  return (
    <div className="settings">
      <header className="page-head">
        <h1 className="page-title">{t("nav.settings")}</h1>
        <p className="page-sub">{t("app.name")}</p>
      </header>

      <div
        role="tablist"
        aria-label={t("nav.settings")}
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--space-2)",
          marginBottom: "var(--space-2)",
        }}
      >
        <Chip selected={tab === "general"} onClick={() => setTab("general")}>
          <Icon name="settings" size={14} />
          {t("settings.tabGeneral")}
        </Chip>
        <Chip selected={tab === "vndb"} onClick={() => setTab("vndb")}>
          <Icon name="clock" size={14} />
          {t("settings.tabVndb")}
        </Chip>
        <Chip selected={tab === "transfer"} onClick={() => setTab("transfer")}>
          <Icon name="shelf" size={14} />
          {t("settings.tabTransfer")}
        </Chip>
      </div>

      {tab === "vndb" ? (
        <>
          <SyncScreen />
          <MetadataRefreshPanel />
          {syncSettings ? (
            <VndbSyncPreferences
              syncSettings={syncSettings}
              setSyncSettings={setSyncSettings}
            />
          ) : null}
        </>
      ) : null}

      {tab === "transfer" ? <DeviceSyncScreen /> : null}

      {tab === "general" ? (
        <>
          <section className="settings__group">
            <h2 className="settings__group-title">{t("settings.appearance")}</h2>

            <div className="settings__row">
              <span className="settings__label">{t("settings.theme")}</span>
              <div className="settings__control">
                {(
                  [
                    ["dark", "settings.themeDark"],
                    ["light", "settings.themeLight"],
                    ["system", "settings.themeSystem"],
                  ] as const
                ).map(([value, labelKey]) => (
                  <Chip
                    key={value}
                    selected={prefs.theme === value}
                    onClick={() => void apply(() => api.setTheme(value))}
                  >
                    {t(labelKey)}
                  </Chip>
                ))}
              </div>
            </div>

            <div className="settings__row">
              <span className="settings__label">
                {t("settings.effectsLabel")}
                <span className="settings__hint">{t("settings.reduceEffectsHint")}</span>
              </span>
              <div className="settings__control">
                <Chip
                  selected={prefs.reduce_effects}
                  onClick={() =>
                    void apply(() => api.setReduceEffects(!prefs.reduce_effects))
                  }
                >
                  {prefs.reduce_effects
                    ? t("settings.effectsReduced")
                    : t("settings.effectsOn")}
                </Chip>
              </div>
            </div>

            <div className="settings__row">
              <span className="settings__label">{t("settings.language")}</span>
              <div className="settings__control">
                <Chip
                  selected={prefs.language === "en"}
                  onClick={() => void apply(() => api.setLanguage("en"))}
                >
                  English
                </Chip>
                <Chip
                  selected={prefs.language === "id"}
                  onClick={() => void apply(() => api.setLanguage("id"))}
                >
                  Indonesia
                </Chip>
              </div>
            </div>
          </section>

          <section className="settings__group">
            <h2 className="settings__group-title">{t("settings.content")}</h2>
            <div className="settings__row">
              <span className="settings__label">{t("settings.nsfw")}</span>
              <div className="settings__control">
                {(
                  [
                    ["show", "settings.nsfwShow"],
                    ["blur", "settings.nsfwBlur"],
                    ["hide", "settings.nsfwHide"],
                  ] as const
                ).map(([value, labelKey]) => (
                  <Chip
                    key={value}
                    selected={prefs.nsfw === value}
                    onClick={() => void apply(() => api.setNsfwMode(value))}
                  >
                    {t(labelKey)}
                  </Chip>
                ))}
              </div>
            </div>
          </section>

          {isWindows ? (
            <section className="settings__group">
              <h2 className="settings__group-title">{t("settings.afk")}</h2>

              <div className="settings__row">
                <span className="settings__label">
                  {t("settings.afkThreshold")}
                  <span className="settings__hint">{t("settings.afkThresholdHint")}</span>
                </span>
                <div className="settings__control">
                  {[30_000, 60_000, 300_000].map((threshold) => (
                    <Chip
                      key={threshold}
                      selected={afk?.threshold_ms === threshold}
                      onClick={() => {
                        if (!afk) return;
                        void (async () => {
                          try {
                            setAfk(await api.setAfkConfig(threshold, afk.exclude_afk));
                          } catch (caught) {
                            pushToast(
                              "error",
                              caught instanceof IpcError
                                ? caught.message
                                : t("common.error"),
                            );
                          }
                        })();
                      }}
                    >
                      {t("settings.seconds", { count: threshold / 1000 })}
                    </Chip>
                  ))}
                </div>
              </div>

              <div className="settings__row">
                <span className="settings__label">
                  {t("settings.awayLabel")}
                  <span className="settings__hint">{t("settings.afkExcludeHint")}</span>
                </span>
                <div className="settings__control">
                  <Chip
                    selected={afk?.exclude_afk === true}
                    onClick={() => {
                      if (!afk) return;
                      void (async () => {
                        try {
                          setAfk(
                            await api.setAfkConfig(afk.threshold_ms, !afk.exclude_afk),
                          );
                        } catch (caught) {
                          pushToast(
                            "error",
                            caught instanceof IpcError
                              ? caught.message
                              : t("common.error"),
                          );
                        }
                      })();
                    }}
                  >
                    {afk?.exclude_afk
                      ? t("settings.afkExclude")
                      : t("settings.afkInclude")}
                  </Chip>
                </div>
              </div>
            </section>
          ) : null}

          <section className="settings__group">
            <h2 className="settings__group-title">{t("settings.languageTools")}</h2>
            <ChatPanel />
            <LunaPanel />
            {isWindows ? <LocalePanel /> : null}
            {isWindows ? <TitleParsingPanel /> : null}
            {canUsePresence ? <PresencePanel /> : null}
          </section>

          {syncSettings ? (
            <VndbSyncPreferences
              syncSettings={syncSettings}
              setSyncSettings={setSyncSettings}
            />
          ) : null}

          <section className="settings__group">
            <h2 className="settings__group-title">{t("settings.data")}</h2>
            <p className="settings__hint">{t("settings.backupHint")}</p>
            <div className="settings__backup-actions">
              <Button variant="ghost" onClick={() => setTab("transfer")}>
                {t("settings.tabTransfer")}
              </Button>
              <Button variant="ghost" onClick={() => setTab("vndb")}>
                {t("settings.tabVndb")}
              </Button>
            </div>
          </section>

          <section className="settings__group">
            <h2 className="settings__group-title">{t("settings.platform")}</h2>
            {platform ? <CapabilityMatrix platform={platform} /> : null}
          </section>
        </>
      ) : null}
    </div>
  );
}

/**
 * Refresh the VNDB metadata every game already has.
 *
 * Metadata used to be frozen at import time: `vndb_import` was the only writer and
 * it is reachable only when adding a game, so a cached row never learned about
 * fields added later. Refreshing one game at a time exists on its page; this is the
 * same operation across the library, because 41 individual clicks is not a feature.
 *
 * Slow by nature: one request per game, paced by the shared rate limiter. The run
 * reports how far it got rather than pretending it finished, and a stopped run
 * resumes from the stored index instead of starting over.
 */
function MetadataRefreshPanel() {
  const { t } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<RefreshAllOutcome | null>(null);

  const refreshAll = async (startAt?: number): Promise<void> => {
    setBusy(true);
    try {
      const outcome = await api.vndbRefreshAll(startAt);
      setResult(outcome);
      if (outcome.failed.length === 0) {
        pushToast(
          "success",
          t("settings.refreshDone", { updated: String(outcome.updated) }),
        );
      } else {
        // One dead VN id must not hide the games that did update.
        pushToast(
          "error",
          t("settings.refreshPartial", {
            updated: String(outcome.updated),
            failed: String(outcome.failed.length),
          }),
        );
      }
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("common.error"),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="settings__group">
      <h2 className="settings__group-title">{t("settings.metadataTitle")}</h2>
      <div className="settings__row">
        <span className="settings__label">
          {t("settings.refreshLabel")}
          <span className="settings__hint">{t("settings.refreshHint")}</span>
        </span>
        <div className="settings__control">
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => {
              void refreshAll();
            }}
          >
            {busy ? t("settings.refreshRunning") : t("settings.refreshAll")}
          </Button>
        </div>
      </div>

      {result ? (
        <p className="settings__hint" role="status">
          {t("settings.refreshResult", {
            updated: String(result.updated),
            attempted: String(result.attempted),
          })}
          {result.cancelled
            ? ` ${t("settings.refreshUnfinished", { at: String(result.lastIndex) })}`
            : ""}
        </p>
      ) : null}

      {result && result.failed.length > 0 ? (
        <details className="settings__details">
          <summary>{t("settings.refreshFailures")}</summary>
          <ul className="settings__hint">
            {result.failed.map((entry) => (
              <li key={entry}>{entry}</li>
            ))}
          </ul>
        </details>
      ) : null}

      {result?.cancelled ? (
        <Button
          variant="secondary"
          disabled={busy}
          onClick={() => {
            void refreshAll(result.lastIndex);
          }}
        >
          {t("settings.refreshResume")}
        </Button>
      ) : null}
    </section>
  );
}

function VndbSyncPreferences({
  syncSettings,
  setSyncSettings,
}: {
  syncSettings: SyncSettings;
  setSyncSettings: (next: SyncSettings) => void;
}) {
  const { t } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);

  return (
    <section className="settings__group">
      <h2 className="settings__group-title">{t("sync.policyTitle")}</h2>
      <div style={{ display: "grid", gap: "var(--space-2)" }}>
        {(
          [
            ["ask", "sync.policyAsk", "sync.policyAskHint"],
            ["prefer_local", "sync.policyLocal", "sync.policyLocalHint"],
            ["prefer_remote", "sync.policyRemote", "sync.policyRemoteHint"],
            ["newest", "sync.policyNewest", "sync.policyNewestHint"],
          ] as const
        ).map(([value, labelKey, hintKey]) => (
          <button
            type="button"
            className="settings__row settings__row--choice"
            key={value}
            data-active={syncSettings.policy === value ? "true" : "false"}
            aria-pressed={syncSettings.policy === value}
            onClick={() => {
              void (async () => {
                try {
                  const next = await api.vndbSaveSyncSettings(
                    syncSettings.mapping,
                    value as ConflictPolicy,
                  );
                  setSyncSettings(next);
                } catch (caught) {
                  pushToast(
                    "error",
                    caught instanceof IpcError ? caught.message : t("common.error"),
                  );
                }
              })();
            }}
          >
            <span className="settings__label">
              {t(labelKey)}
              <span className="settings__hint">{t(hintKey)}</span>
            </span>
            {syncSettings.policy === value ? (
              <span className="settings__inuse">
                <Icon name="check" />
                {t("sync.policyInUse")}
              </span>
            ) : null}
          </button>
        ))}
      </div>

      <h2 className="settings__group-title" style={{ marginTop: "var(--space-4)" }}>
        {t("sync.labelsTitle")}
      </h2>
      <p className="settings__hint">{t("sync.labelsHelp")}</p>
      {statusOrder.map((status) => (
        <div className="settings__row" key={status}>
          <span className="settings__label">{t(`status.${status}`)}</span>
          <div className="settings__control">
            <input
              className="input"
              type="number"
              min={0}
              max={999}
              style={{ width: "var(--input-number-width)" }}
              value={syncSettings.mapping[status]}
              aria-label={`${t("sync.labelsTitle")}: ${t(`status.${status}`)}`}
              onChange={(event) => {
                const value = Number(event.target.value);
                if (!Number.isFinite(value) || value < 0) return;
                setSyncSettings({
                  ...syncSettings,
                  mapping: { ...syncSettings.mapping, [status]: value },
                });
              }}
              onBlur={() => {
                void (async () => {
                  try {
                    const next = await api.vndbSaveSyncSettings(
                      syncSettings.mapping,
                      syncSettings.policy,
                    );
                    setSyncSettings(next);
                  } catch (caught) {
                    pushToast(
                      "error",
                      caught instanceof IpcError && caught.code === "collision"
                        ? t("sync.collision")
                        : caught instanceof IpcError
                          ? caught.message
                          : t("common.error"),
                    );
                  }
                })();
              }}
            />
          </div>
        </div>
      ))}
    </section>
  );
}

function CapabilityMatrix({ platform }: { platform: PlatformInfo }) {
  const { t } = useI18n();
  const rows: Array<[string, Support]> = [
    [t("capability.processTracking"), platform.process_tracking],
    [t("capability.chapterDetection"), platform.chapter_detection],
    [t("capability.discordPresence"), platform.discord_presence],
    [t("capability.cloudSync"), platform.cloud_sync],
  ];

  return (
    <div className="capabilities">
      <div className="capabilities__row">
        <span className="caption">{t("settings.platformOs")}</span>
        <span className="caption">{platform.os}</span>
      </div>
      {rows.map(([label, support]) => (
        <div className="capabilities__row" key={label}>
          <span>{label}</span>
          <span
            className="caption"
            style={{
              color:
                support === "supported"
                  ? "var(--success)"
                  : support === "experimental"
                    ? "var(--warning)"
                    : "var(--text-faint)",
            }}
          >
            {t(`support.${support}`)}
          </span>
        </div>
      ))}
    </div>
  );
}
