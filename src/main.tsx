import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router";

import { BetaRoute } from "@/app/BetaRoute";
import { AppProvider } from "@/app/AppProvider";
import { AppShellWithMenu } from "@/app/AppShell";
import { LibraryScreen } from "@/features/library/LibraryScreen";
import { RouteError } from "@/app/RouteError";
import "@/styles/index.css";

/** Split non-home routes lazily while keeping `LibraryScreen` eager so first paint has no blank frame. */
const router = createBrowserRouter([
  {
    path: "/companion-overlay",
    errorElement: <RouteError />,
    lazy: async () => {
      const { CompanionOverlay } = await import("@/features/companion/CompanionOverlay");
      return {
        Component: () => (
          <BetaRoute overlay>
            <CompanionOverlay />
          </BetaRoute>
        ),
      };
    },
  },
  {
    path: "/",
    element: <AppShellWithMenu />,
    errorElement: <RouteError />,
    children: [
      { index: true, element: <LibraryScreen /> },
      {
        path: "game/:id",
        lazy: async () => {
          const { GameDetailScreen } = await import("@/features/game/GameDetailScreen");
          return { Component: GameDetailScreen };
        },
      },
      {
        path: "stats",
        lazy: async () => {
          const { StatsScreen } = await import("@/features/stats/StatsScreen");
          return { Component: StatsScreen };
        },
      },
      {
        path: "timeline",
        lazy: async () => {
          const { TimelineScreen } = await import("@/features/timeline/TimelineScreen");
          return { Component: TimelineScreen };
        },
      },
      {
        path: "guide",
        lazy: async () => {
          const { GuideScreen } = await import("@/features/guide/GuideScreen");
          return { Component: GuideScreen };
        },
      },
      {
        path: "settings",
        lazy: async () => {
          const { default: SettingsScreen } =
            await import("@/features/settings/SettingsScreen");
          return { Component: () => <SettingsScreen key="general" /> };
        },
      },
      {
        path: "sync",
        lazy: async () => {
          const { default: SettingsScreen } =
            await import("@/features/settings/SettingsScreen");
          return { Component: () => <SettingsScreen key="vndb" initialTab="vndb" /> };
        },
      },
      {
        path: "devices",
        lazy: async () => {
          const { default: SettingsScreen } =
            await import("@/features/settings/SettingsScreen");
          return {
            Component: () => <SettingsScreen key="transfer" initialTab="transfer" />,
          };
        },
      },
      {
        path: "tracking",
        lazy: async () => {
          const { default: SettingsScreen } =
            await import("@/features/settings/SettingsScreen");
          return { Component: () => <SettingsScreen key="tracking" initialTab="tracking" /> };
        },
      },
      {
        path: "recommend",
        lazy: async () => {
          const { RecommendScreen } =
            await import("@/features/recommend/RecommendScreen");
          return {
            Component: () => (
              <BetaRoute>
                <RecommendScreen />
              </BetaRoute>
            ),
          };
        },
      },
      {
        path: "big-picture",
        lazy: async () => {
          const { BigPictureScreen } =
            await import("@/features/bigpicture/BigPictureScreen");
          return { Component: BigPictureScreen };
        },
      },
      {
        path: "presence",
        lazy: async () => {
          const { PresenceScreen } = await import("@/features/presence/PresenceScreen");
          return { Component: PresenceScreen };
        },
      },
      {
        path: "chat",
        lazy: async () => {
          const { ChatScreen } = await import("@/features/chat/ChatScreen");
          return {
            Component: () => (
              <BetaRoute>
                <ChatScreen />
              </BetaRoute>
            ),
          };
        },
      },
      {
        path: "companion",
        lazy: async () => {
          const { CompanionScreen } =
            await import("@/features/companion/CompanionScreen");
          return {
            Component: () => (
              <BetaRoute>
                <CompanionScreen />
              </BetaRoute>
            ),
          };
        },
      },
    ],
  },
]);

const container = document.getElementById("root");
if (!container) {
  throw new Error("Root element missing from index.html");
}

createRoot(container).render(
  <StrictMode>
    <AppProvider companionWindow={window.location.pathname === "/companion-overlay"}>
      <RouterProvider router={router} />
    </AppProvider>
  </StrictMode>,
);
