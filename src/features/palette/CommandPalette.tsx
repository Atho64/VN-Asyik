import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Icon } from "@/components/Icon";
import { rankGames } from "@/lib/library";
import { AddGameSheet } from "@/features/library/AddGameSheet";
import { api } from "@/lib/ipc/client";

interface PaletteAction {
  id: string;
  label: string;
  hint?: string;
  run: () => void;
}

/**
 * Command palette. Desktop shortcut Ctrl/Cmd+K.
 *
 * Two groups only, and both are real: screens that exist and games that exist.
 * Typing a game title navigates; there is no fuzzy "AI command" pretending to do
 * things that are not built.
 */
export function CommandPalette() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const setOpen = useAppStore((state) => state.setPaletteOpen);
  const cards = useAppStore((state) => state.cards);
  const isAndroid = useAppStore((state) => state.platform?.os === "android");
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const [addOpen, setAddOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const close = useCallback((): void => setOpen(false), [setOpen]);

  const actions = useMemo<PaletteAction[]>(() => {
    const entries: PaletteAction[] = [
      {
        id: "nav-chat",
        label: t("vnchat.title"),
        run: () => {
          void navigate("/chat");
          close();
        },
      },
      {
        id: "nav-library",
        label: t("palette.goLibrary"),
        run: () => {
          void navigate("/");
          close();
        },
      },
      {
        id: "nav-tracking",
        label: t("palette.goTracking"),
        run: () => {
          void navigate("/tracking");
          close();
        },
      },
      {
        id: "nav-stats",
        label: t("palette.goStats"),
        run: () => {
          void navigate("/stats");
          close();
        },
      },
      {
        id: "nav-settings",
        label: t("palette.goSettings"),
        run: () => {
          void navigate("/settings");
          close();
        },
      },
      {
        id: "nav-vndb",
        label: t("palette.goVndb"),
        run: () => {
          void navigate("/sync");
          close();
        },
      },
      {
        id: "nav-transfer",
        label: t("palette.goTransfer"),
        run: () => {
          void navigate("/devices");
          close();
        },
      },
      {
        id: "add-game",
        label: t("palette.addGame"),
        run: () => {
          setAddOpen(true);
          close();
        },
      },
      // Window visibility is a desktop idea: Android has no taskbar or tray to
      // come back from, and the commands behind these two answer "unsupported"
      // there. Offering them would be a button that cannot do what it says.
      ...(isAndroid
        ? []
        : [
            {
              id: "sys-hide",
              label: t("palette.hideWindow"),
              hint: t("palette.hideWindowHint"),
              run: () => {
                void api.hideWindow();
                close();
              },
            },
            {
              id: "sys-minimize",
              label: t("palette.minimizeWindow"),
              hint: t("palette.minimizeWindowHint"),
              run: () => {
                void api.minimizeWindow();
                close();
              },
            },
          ]),
      {
        id: "sys-quit",
        label: t("palette.quitApp"),
        hint: t("palette.quitAppHint"),
        run: () => {
          void api.quitApp();
          close();
        },
      },
    ];

    const needle = query.trim();
    if (needle.length === 0) return entries;

    const matchedActions = entries.filter((entry) =>
      entry.label.toLowerCase().includes(needle.toLowerCase()),
    );
    // Ranked, not filtered: typing "clannad" should offer CLANNAD before a
    // spin-off whose title merely contains the letters.
    const matchedGames: PaletteAction[] = rankGames(
      cards.map((card) => card.game),
      needle,
    )
      .slice(0, 6)
      .map((game) => ({
        id: `game-${game.meta.id}`,
        label: game.title,
        hint: t(`status.${game.status}`),
        run: () => {
          void navigate(`/game/${game.meta.id}`);
          close();
        },
      }));

    return [...matchedActions, ...matchedGames];
  }, [cards, close, isAndroid, navigate, query, t]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>): void => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActiveIndex((index) => Math.min(index + 1, actions.length - 1));
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setActiveIndex((index) => Math.max(index - 1, 0));
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      actions[activeIndex]?.run();
    }
  };

  return (
    <>
      <div
        className="scrim"
        role="presentation"
        onMouseDown={(event) => {
          if (event.target === event.currentTarget) close();
        }}
      >
        <div
          className="palette"
          role="dialog"
          aria-modal="true"
          aria-label={t("palette.title")}
          onKeyDown={onKeyDown}
        >
          <input
            ref={inputRef}
            className="palette__input"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              // The highlighted row is only meaningful for the current query.
              setActiveIndex(0);
            }}
            placeholder={t("palette.placeholder")}
            aria-label={t("palette.placeholder")}
          />
          <div className="palette__list" role="listbox" aria-label={t("palette.title")}>
            {actions.length === 0 ? (
              <p className="palette__empty">{t("palette.empty")}</p>
            ) : (
              actions.map((action, index) => (
                <button
                  key={action.id}
                  type="button"
                  role="option"
                  aria-selected={index === activeIndex}
                  className="palette__item"
                  data-active={index === activeIndex ? "true" : "false"}
                  onMouseEnter={() => setActiveIndex(index)}
                  onClick={action.run}
                >
                  <Icon
                    name={action.id.startsWith("game-") ? "library" : "search"}
                    size={14}
                  />
                  <span>{action.label}</span>
                  {action.hint ? (
                    <span className="palette__hint">{action.hint}</span>
                  ) : null}
                </button>
              ))
            )}
          </div>
        </div>
      </div>
      {addOpen ? <AddGameSheet open onClose={() => setAddOpen(false)} /> : null}
    </>
  );
}
