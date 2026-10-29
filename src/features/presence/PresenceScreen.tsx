import { useI18n } from "@/app/useI18n";
import { PresencePanel } from "@/features/settings/PresencePanel";

/**
 * Discord Rich Presence as its own screen.
 *
 * It grew from a settings card into the largest panel in the app: preview, image
 * host, templates and a rotation list. Kept in Settings it pushed the actual
 * settings below the fold, so the panel moved here and Settings kept a pointer.
 *
 * `PresencePanel` still owns the state and the layout; this screen only gives it a
 * page, the way the chat and stats screens have one.
 */
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
