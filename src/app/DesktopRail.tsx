import { useRef } from "react";
import { NavLink } from "react-router";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Icon, type IconName } from "@/components/Icon";
import { api } from "@/lib/ipc/client";
import appIcon from "@/assets/vn-asyik.svg";

export interface NavEntry {
  to: string;
  labelKey:
    | "nav.library"
    | "nav.stats"
    | "nav.timeline"
    | "nav.recommend"
    | "nav.achievements"
    | "nav.settings"
    | "nav.presence"
    | "nav.guide";
  icon: IconName;
}

export function DesktopRail({
  entries,
  settingsActive,
}: {
  entries: readonly NavEntry[];
  settingsActive: boolean;
}) {
  const { t } = useI18n();
  const compact = useAppStore((state) => state.sidebarCompact);
  const setPaletteOpen = useAppStore((state) => state.setPaletteOpen);
  const pendingSave = useRef<boolean | null>(null);
  const saving = useRef(false);

  const persist = async (): Promise<void> => {
    if (saving.current || typeof api.uiPrefsSet !== "function") return;
    saving.current = true;
    try {
      while (pendingSave.current !== null) {
        const requested = pendingSave.current;
        pendingSave.current = null;
        try {
          const prefs = await api.uiPrefsSet({ sidebar_compact: requested });
          // A completed write must not undo a newer click while its save is queued.
          if (
            pendingSave.current === null &&
            useAppStore.getState().sidebarCompact === requested
          ) {
            useAppStore.getState().setUiPrefs(prefs);
          }
        } catch {
          // The immediate local preference remains usable when native storage fails.
        }
      }
    } finally {
      saving.current = false;
    }
  };

  const toggle = (): void => {
    const state = useAppStore.getState();
    const next = !state.sidebarCompact;
    state.setSidebarCompact(next);
    pendingSave.current = next;
    void persist();
  };

  return (
    <nav
      className={compact ? "rail rail--compact" : "rail"}
      data-compact={compact ? "true" : "false"}
      aria-label={t("app.name")}
    >
      <div className="rail__brand">
        <div className="rail__brand-id">
          <img className="rail__mark" src={appIcon} alt="" />
          <span className={compact ? "rail__name visually-hidden" : "rail__name"}>
            {t("app.name")}
          </span>
        </div>
        <button
          type="button"
          className="rail__toggle"
          onClick={toggle}
          aria-expanded={!compact}
          aria-label={compact ? t("nav.expandSidebar") : t("nav.collapseSidebar")}
          title={compact ? t("nav.expandSidebar") : t("nav.collapseSidebar")}
        >
          <Icon name="sidebar" size={16} />
        </button>
      </div>
      <div className="rail__nav">
        {entries.map((entry) => {
          const label = t(entry.labelKey);
          return (
            <NavLink
              key={entry.to}
              to={entry.to}
              end
              title={compact ? label : undefined}
              className={({ isActive }) =>
                `rail__item${
                  isActive || (entry.to === "/settings" && settingsActive)
                    ? " active"
                    : ""
                }`
              }
            >
              <span className="rail__icon">
                <Icon name={entry.icon} />
              </span>
              <span className={compact ? "rail__label visually-hidden" : "rail__label"}>
                {label}
              </span>
            </NavLink>
          );
        })}
      </div>
      <div className="rail__footer">
        <Button
          variant="ghost"
          className={compact ? "rail__search rail__search--compact" : "rail__search"}
          onClick={() => setPaletteOpen(true)}
          aria-keyshortcuts="Control+K Meta+K"
          title={compact ? t("library.search") : undefined}
        >
          <Icon name="search" />
          <span className={compact ? "rail__label visually-hidden" : "rail__label"}>
            {t("library.search")}
          </span>
        </Button>
      </div>
    </nav>
  );
}
