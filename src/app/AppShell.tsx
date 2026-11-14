import { useEffect, useRef, useState } from "react";
import { NavLink, Outlet, useLocation, useNavigate } from "react-router";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Backdrop, focusArt } from "@/components/Backdrop";
import { updateAppearance } from "@/lib/theme";
import { listenScreenshotShortcut } from "@/lib/ipc/screenshots";
import { Icon, type IconName } from "@/components/Icon";
import { Toasts } from "@/components/Toasts";
import { ContextMenuProvider } from "@/app/ContextMenuProvider";
import { NowPlayingBar } from "@/features/tracking/NowPlayingBar";
import { CompanionBar } from "@/features/companion/CompanionBar";
import { CommandPalette } from "@/features/palette/CommandPalette";
import { AddGameSheet } from "@/features/library/AddGameSheet";
import { Button } from "@/components/Button";
import { Sheet } from "@/components/Sheet";
import { api } from "@/lib/ipc/client";
import { companionWindowApi } from "@/lib/ipc/companionWindow";
import appIcon from "@/assets/vn-asyik.svg";

interface NavEntry {
  to: string;
  labelKey:
    | "nav.library"
    | "nav.stats"
    | "nav.timeline"
    | "nav.recommend"
    | "vnchat.title"
    | "companion.title"
    | "nav.settings"
    | "nav.presence"
    | "nav.guide";
  icon: IconName;
  beta?: boolean;
}

/**
 * Top-level navigation: Library, Stats, Timeline, Recommend, Chat, Presence,
 * Guide, and Settings. Tracking, VNDB Sync and Device Data Transfer live inside
 * Settings to keep the main navigation uncluttered.
 */
const navEntries: NavEntry[] = [
  { to: "/", labelKey: "nav.library", icon: "library" },
  { to: "/stats", labelKey: "nav.stats", icon: "stats" },
  { to: "/timeline", labelKey: "nav.timeline", icon: "clock" },
  { to: "/recommend", beta: true, labelKey: "nav.recommend", icon: "spark" },
  { to: "/chat", beta: true, labelKey: "vnchat.title", icon: "chat" },
  { to: "/companion", beta: true, labelKey: "companion.title", icon: "shelf" },
  { to: "/presence", labelKey: "nav.presence", icon: "gamepad" },
  { to: "/guide", labelKey: "nav.guide", icon: "guide" },
  { to: "/settings", labelKey: "nav.settings", icon: "settings" },
];

const mobilePaths = new Set(["/", "/stats", "/settings"]);
const mobileEntries = navEntries.filter((entry) => mobilePaths.has(entry.to));

export function AppShell() {
  const { t } = useI18n();
  const beta = useAppStore((state) => state.prefs.beta_features);
  const visibleEntries = navEntries.filter((entry) => !entry.beta || beta);
  const moreEntries = visibleEntries.filter((entry) => !mobilePaths.has(entry.to));
  const paletteOpen = useAppStore((state) => state.paletteOpen);
  const appearance = useAppStore((state) => state.appearance);
  const setPaletteOpen = useAppStore((state) => state.setPaletteOpen);
  const sidebarCompact = useAppStore((state) => state.sidebarCompact);
  const setSidebarCompact = useAppStore((state) => state.setSidebarCompact);
  const setUiPrefs = useAppStore((state) => state.setUiPrefs);
  const toggleSidebar = (): void => {
    const next = !sidebarCompact;
    setSidebarCompact(next);
    if (typeof api.uiPrefsSet === "function") {
      void api
        .uiPrefsSet({ sidebar_compact: next })
        .then((prefs) => setUiPrefs(prefs))
        .catch(() => undefined);
    }
  };
  const [moreOpen, setMoreOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const mainRef = useRef<HTMLElement>(null);
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const moreActive = moreEntries.some((entry) => entry.to === pathname);
  const settingsActive =
    pathname === "/settings" ||
    pathname === "/sync" ||
    pathname === "/devices" ||
    pathname === "/tracking";

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void companionWindowApi
      .listenMain((path) => {
        void navigate(path);
      })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [navigate]);

  // The global shortcut captures from Rust; this only reports what happened.
  useEffect(() => {
    let stop: (() => void) | undefined;
    let disposed = false;
    void listenScreenshotShortcut((result) => {
      const { pushToast } = useAppStore.getState();
      if (result.ok) {
        pushToast("success", t("screenshot.captured"));
        return;
      }
      const key = `screenshot.error.${result.code ?? "screenshot_failed"}`;
      const translated = t(key);
      pushToast("error", translated === key ? t("common.error") : translated);
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stop = unlisten;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stop?.();
    };
  }, [t]);

  useEffect(() => {
    mainRef.current?.scrollTo?.({ top: 0 });
    if (pathname !== "/" && !pathname.startsWith("/game/")) {
      // Clearing the tint, not the accent: `removeProperty` also deleted the
      // reader's own accent, which then stayed gone until the next restart.
      updateAppearance({ tint: null });
      focusArt(null);
    }
  }, [pathname]);

  // The launch mode chosen in Settings applies on every launch: Big Picture
  // navigates once after boot, after the prefs have hydrated. Manual and tray
  // need nothing here (tray stays hidden until called up).
  const startupMode = useAppStore((state) => state.startupPrefs?.prefs.startup_mode);
  const navigatedForBoot = useRef(false);
  useEffect(() => {
    if (navigatedForBoot.current || startupMode === undefined) return;
    navigatedForBoot.current = true;
    if (startupMode === "big_picture") {
      navigate("/big-picture");
    }
  }, [startupMode, navigate]);

  // Leaving Big Picture drops the fullscreen the launch mode set; entering it
  // by hand never forces fullscreen, matching the old behaviour.
  const wasBigPicture = useRef(false);
  useEffect(() => {
    const bigPicture = pathname === "/big-picture";
    if (wasBigPicture.current && !bigPicture) {
      void api.windowSetFullscreen(false).catch(() => {});
    }
    wasBigPicture.current = bigPicture;
  }, [pathname]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing || e.repeat || e.altKey) return;
      if (
        (e.ctrlKey || e.metaKey) &&
        !e.shiftKey &&
        (e.code === "KeyK" || e.key.toLowerCase() === "k")
      ) {
        e.preventDefault();
        setPaletteOpen(!useAppStore.getState().paletteOpen);
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && (e.key === "H" || e.key === "h")) {
        e.preventDefault();
        void api.toggleWindow();
      }
      // Big Picture is the one mode worth a global shortcut: it is opened from a
      // sofa, where reaching the toolbar is the awkward part.
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && (e.key === "B" || e.key === "b")) {
        e.preventDefault();
        void navigate("/big-picture");
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [navigate, setPaletteOpen]);

  return (
    <div className={pathname === "/big-picture" ? "app app--big-picture" : "app"}>
      {/* A chosen wallpaper replaces the living artwork wash: a wallpaper is a
       * deliberate choice, and two full-screen images fighting over the same
       * pixels is a blur, not a backdrop. */}
      {appearance?.background ? (
        <div
          className="app-backdrop"
          aria-hidden
          style={{ backgroundImage: `url(${appearance.background})` }}
        >
          <div
            className="backdrop__scrim"
            style={{ opacity: appearance.backgroundDim }}
          />
        </div>
      ) : (
        <Backdrop />
      )}

      <nav
        className={sidebarCompact ? "rail rail--compact" : "rail"}
        data-compact={sidebarCompact ? "true" : "false"}
        aria-label={t("app.name")}
      >
        <div className="rail__brand">
          <div className="rail__brand-id">
            <img className="rail__mark" src={appIcon} alt="" />
            <span
              className={sidebarCompact ? "rail__name visually-hidden" : "rail__name"}
            >
              {t("app.name")}
            </span>
          </div>
          <button
            type="button"
            className="rail__toggle"
            onClick={toggleSidebar}
            aria-expanded={!sidebarCompact}
            aria-label={
              sidebarCompact ? t("nav.expandSidebar") : t("nav.collapseSidebar")
            }
            title={sidebarCompact ? t("nav.expandSidebar") : t("nav.collapseSidebar")}
          >
            <Icon name="sidebar" size={16} />
          </button>
        </div>
        <div className="rail__nav">
          {visibleEntries.map((entry) => {
            const label = t(entry.labelKey);
            return (
              <NavLink
                key={entry.to}
                to={entry.to}
                end
                title={sidebarCompact ? label : undefined}
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
                <span
                  className={
                    sidebarCompact ? "rail__label visually-hidden" : "rail__label"
                  }
                >
                  {label}
                </span>
              </NavLink>
            );
          })}
        </div>
        <div className="rail__footer">
          <Button
            variant="ghost"
            className={
              sidebarCompact ? "rail__search rail__search--compact" : "rail__search"
            }
            onClick={() => setPaletteOpen(true)}
            aria-keyshortcuts="Control+K Meta+K"
            title={sidebarCompact ? t("library.search") : undefined}
          >
            <Icon name="search" />
            <span
              className={sidebarCompact ? "rail__label visually-hidden" : "rail__label"}
            >
              {t("library.search")}
            </span>
          </Button>
        </div>
      </nav>

      <div className="app__content">
        <main ref={mainRef} className="app__main" id="main-content">
          <Outlet />
        </main>
        <NowPlayingBar />
        {beta ? <CompanionBar /> : null}
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

      {paletteOpen ? <CommandPalette onAddGame={() => setAddOpen(true)} /> : null}
      {addOpen ? <AddGameSheet open onClose={() => setAddOpen(false)} /> : null}
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
