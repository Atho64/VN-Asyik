import { useEffect, useRef, useState } from "react";
import { NavLink, Outlet, useLocation } from "react-router";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Backdrop, focusArt } from "@/components/Backdrop";
import { Icon, type IconName } from "@/components/Icon";
import { Toasts } from "@/components/Toasts";
import { ContextMenuProvider } from "@/app/ContextMenuProvider";
import { NowPlayingBar } from "@/features/tracking/NowPlayingBar";
import { CommandPalette } from "@/features/palette/CommandPalette";
import { Button } from "@/components/Button";
import { Sheet } from "@/components/Sheet";
import { api } from "@/lib/ipc/client";
import appIcon from "@/assets/vn-asyik.svg";

interface NavEntry {
  to: string;
  labelKey:
    | "nav.library"
    | "nav.tracking"
    | "nav.stats"
    | "nav.recommend"
    | "vnchat.title"
    | "nav.settings"
    | "nav.guide";
  icon: IconName;
}

/**
 * Top-level navigation: Library, Tracking, Stats, Discover, and Settings.
 * VNDB Sync and Device Data Transfer live inside Settings to keep the main
 * navigation uncluttered.
 */
const navEntries: NavEntry[] = [
  { to: "/", labelKey: "nav.library", icon: "library" },
  { to: "/tracking", labelKey: "nav.tracking", icon: "play" },
  { to: "/stats", labelKey: "nav.stats", icon: "stats" },
  { to: "/recommend", labelKey: "nav.recommend", icon: "spark" },
  { to: "/chat", labelKey: "vnchat.title", icon: "chat" },
  { to: "/guide", labelKey: "nav.guide", icon: "guide" },
  { to: "/settings", labelKey: "nav.settings", icon: "settings" },
];

const mobilePaths = new Set(["/", "/tracking", "/stats", "/settings"]);
const mobileEntries = navEntries.filter((entry) => mobilePaths.has(entry.to));
const moreEntries = navEntries.filter((entry) => !mobilePaths.has(entry.to));

export function AppShell() {
  const { t } = useI18n();
  const paletteOpen = useAppStore((state) => state.paletteOpen);
  const setPaletteOpen = useAppStore((state) => state.setPaletteOpen);
  const [moreOpen, setMoreOpen] = useState(false);
  const mainRef = useRef<HTMLElement>(null);
  const { pathname } = useLocation();
  const moreActive = moreEntries.some((entry) => entry.to === pathname);
  const settingsActive =
    pathname === "/settings" || pathname === "/sync" || pathname === "/devices";

  useEffect(() => {
    mainRef.current?.scrollTo?.({ top: 0 });
    if (pathname !== "/" && !pathname.startsWith("/game/")) {
      const root = document.documentElement;
      root.style.removeProperty("--accent");
      root.style.removeProperty("--accent-soft");
      root.style.removeProperty("--on-accent");
      focusArt(null);
    }
  }, [pathname]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && (e.key === "H" || e.key === "h")) {
        e.preventDefault();
        void api.toggleWindow();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  return (
    <div className="app">
      <Backdrop />

      <nav className="rail" aria-label={t("app.name")}>
        <div className="rail__brand">
          <img className="rail__mark" src={appIcon} alt="" />
          <span className="rail__name">{t("app.name")}</span>
        </div>
        <div className="rail__nav">
          {navEntries.map((entry) => (
            <NavLink
              key={entry.to}
              to={entry.to}
              end
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
              {t(entry.labelKey)}
            </NavLink>
          ))}
        </div>
        <div className="rail__footer">
          <Button
            variant="ghost"
            onClick={() => setPaletteOpen(true)}
            aria-keyshortcuts="Control+K Meta+K"
            style={{ justifyContent: "flex-start" }}
          >
            <Icon name="search" />
            {t("library.search")}
          </Button>
        </div>
      </nav>

      <div className="app__content">
        <main ref={mainRef} className="app__main" id="main-content">
          <Outlet />
        </main>
        <NowPlayingBar />
      </div>

      <nav className="bottomnav" aria-label={t("app.name")}>
        {mobileEntries.map((entry) => (
          <NavLink
            key={entry.to}
            to={entry.to}
            end
            className={({ isActive }) =>
              `bottomnav__item${
                isActive || (entry.to === "/settings" && settingsActive) ? " active" : ""
              }`
            }
          >
            <Icon name={entry.icon} size={18} />
            <span className="bottomnav__label">{t(entry.labelKey)}</span>
          </NavLink>
        ))}
        <button
          type="button"
          className="bottomnav__item"
          data-active={moreActive ? "true" : "false"}
          aria-expanded={moreOpen}
          aria-haspopup="dialog"
          onClick={() => setMoreOpen(true)}
        >
          <Icon name="grid" size={18} />
          <span className="bottomnav__label">{t("nav.more")}</span>
        </button>
      </nav>

      <Sheet open={moreOpen} title={t("nav.more")} onClose={() => setMoreOpen(false)}>
        <nav className="more-nav" aria-label={t("nav.more")}>
          {moreEntries.map((entry) => (
            <NavLink
              key={entry.to}
              to={entry.to}
              end
              className="more-nav__item"
              onClick={() => setMoreOpen(false)}
            >
              <Icon name={entry.icon} size={20} />
              {t(entry.labelKey)}
            </NavLink>
          ))}
          <Button
            variant="ghost"
            onClick={() => {
              setMoreOpen(false);
              setPaletteOpen(true);
            }}
          >
            <Icon name="search" />
            {t("library.search")}
          </Button>
        </nav>
      </Sheet>

      {paletteOpen ? <CommandPalette /> : null}
      <Toasts />
    </div>
  );
}

/** The shell with the context menu mounted once around it. */
export function AppShellWithMenu() {
  return (
    <ContextMenuProvider>
      <AppShell />
    </ContextMenuProvider>
  );
}
