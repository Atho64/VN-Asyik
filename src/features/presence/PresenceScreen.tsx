import { useI18n } from "@/app/useI18n";
import { PresencePanel } from "@/features/settings/PresencePanel";

/** Dedicated screen wrapper for PresencePanel. */
export function PresenceScreen() {
  const { t } = useI18n();

  return (
    <div className="settings">
      <header className="page-head">
        <h1 className="page-title">{t("presence.title")}</h1>
        <p className="page-sub">{t("presence.sub")}</p>
      </header>

      <section className="settings__group">
        <PresencePanel />
      </section>
    </div>
  );
}
