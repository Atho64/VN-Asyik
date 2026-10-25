import { useEffect, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { api, IpcError } from "@/lib/ipc/client";
import type { LaunchCapability } from "@/lib/ipc/types";

/**
 * Where Locale Emulator lives, if the user has installed it.
 *
 * The app does not bundle it: its core binaries have no stated redistribution
 * terms, so this is a pointer to the user's own copy rather than a dependency.
 */
export function LocalePanel() {
  const { t } = useI18n();
  const [capability, setCapability] = useState<LaunchCapability | null>(null);
  const [folder, setFolder] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void api
      .launchCapability()
      .then((next) => {
        if (!active) return;
        setCapability(next);
        setFolder(next.configuredDir ?? "");
      })
      .catch(() => {
        // Optional feature: a failure here must not break Settings.
      });
    return () => {
      active = false;
    };
  }, []);

  async function save(next?: {
    launcherDir?: string;
    guid?: string;
    emulatorEnabled?: boolean;
  }) {
    setBusy(true);
    setError(null);
    try {
      setCapability(
        await api.launchSaveSettings({
          launcherDir: next?.launcherDir ?? folder,
          // Omitted rather than undefined: exactOptionalPropertyTypes rejects
          // an explicit undefined, and omitting keeps the stored profile.
          ...(next?.guid === undefined ? {} : { guid: next.guid }),
          ...(next?.emulatorEnabled === undefined
            ? {}
            : { emulatorEnabled: next.emulatorEnabled }),
        }),
      );
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  }

  if (!capability?.canLaunch) return null;

  return (
    <Card>
      <CardHeader title={t("play.section")} />
      <p className="settings__hint">{t("play.intro")}</p>

      {/* A real switch, separate from the profile dropdown. Before this, the only
          way to stop using Locale Emulator was to forget where it was installed. */}
      <div className="settings__row">
        <span className="settings__label">{t("play.enabledLabel")}</span>
        <div className="settings__control">
          <Chip
            selected={capability.emulator !== "disabled"}
            onClick={() => {
              void save({ emulatorEnabled: capability.emulator === "disabled" });
            }}
          >
            {capability.emulator === "disabled"
              ? t("play.enabledOff")
              : t("play.enabledOn")}
          </Chip>
        </div>
      </div>

      {/* Everything below is only meaningful while the emulator is on. Hiding it
          rather than greying it out removes five lines of text describing settings
          the user cannot currently use. */}
      {capability.emulator !== "disabled" ? (
        <>
          <form
            className="settings__row-form"
            onSubmit={(event) => {
              event.preventDefault();
              void save();
            }}
          >
            <input
              className="input"
              value={folder}
              onChange={(event) => setFolder(event.target.value)}
              placeholder={t("play.dirPlaceholder")}
              aria-label={t("play.dirLabel")}
            />
            <Button type="submit" variant="secondary" disabled={busy}>
              {t("play.save")}
            </Button>
          </form>

          {capability.emulatorPath ? (
            <p className="settings__hint">
              {t("play.detected", { path: capability.emulatorPath })}
            </p>
          ) : (
            <p className="settings__hint">{t("play.notFound")}</p>
          )}

          {capability.emulator !== "notInstalled" && capability.profiles.length > 0 ? (
            <label className="settings__field" htmlFor="le-profile">
              <span className="settings__label">{t("play.profileLabel")}</span>
              <select
                id="le-profile"
                className="input"
                value={capability.guid ?? ""}
                disabled={busy}
                onChange={(event) => {
                  void save({ guid: event.target.value });
                }}
              >
                <option value="">{t("play.profileNone")}</option>
                {capability.profiles.map((profile) => (
                  <option key={profile.guid} value={profile.guid}>
                    {profile.name}
                  </option>
                ))}
              </select>
              <span className="settings__hint">{t("play.profileWhyShort")}</span>
            </label>
          ) : null}

          {capability.emulator !== "notInstalled" && capability.profiles.length === 0 ? (
            <p className="notice notice--warn">{t("play.profileMissing")}</p>
          ) : null}
        </>
      ) : null}

      {/* Provenance is worth keeping but not worth six lines above the fold. */}
      <details className="settings__details">
        <summary>{t("play.whySummary")}</summary>
        <p className="settings__hint">{t("play.why")}</p>
      </details>

      {error && <p className="notice notice--danger">{error}</p>}
    </Card>
  );
}
