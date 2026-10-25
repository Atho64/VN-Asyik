import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { ContextMenu, type MenuItem } from "@/components/ContextMenu";

interface MenuState {
  x: number;
  y: number;
  items: MenuItem[];
}

interface MenuApi {
  /** Open the app's context menu at a pointer position. */
  openMenu: (event: { clientX: number; clientY: number }, items: MenuItem[]) => void;
}

const MenuContext = createContext<MenuApi | null>(null);

/**
 * One context menu for the whole app.
 *
 * Mounted once at the shell rather than per screen: two menus on screen at the
 * same time would both be listening for the same dismiss events.
 */
export function ContextMenuProvider({ children }: { children: ReactNode }) {
  const [menu, setMenu] = useState<MenuState | null>(null);

  const openMenu = useCallback(
    (event: { clientX: number; clientY: number }, items: MenuItem[]) => {
      if (items.length === 0) return;
      setMenu({ x: event.clientX, y: event.clientY, items });
    },
    [],
  );

  const close = useCallback(() => {
    setMenu(null);
  }, []);

  const api = useMemo<MenuApi>(() => ({ openMenu }), [openMenu]);

  return (
    <MenuContext.Provider value={api}>
      {children}
      {menu ? (
        <ContextMenu position={{ x: menu.x, y: menu.y }} items={menu.items} onClose={close} />
      ) : null}
    </MenuContext.Provider>
  );
}

/** Open the app's context menu. */
export function useContextMenu(): MenuApi {
  const context = useContext(MenuContext);
  if (!context) {
    throw new Error("useContextMenu must be used inside ContextMenuProvider");
  }
  return context;
}
