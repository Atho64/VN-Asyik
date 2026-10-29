import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router";

import { AppProvider } from "@/app/AppProvider";
import { AppShellWithMenu } from "@/app/AppShell";
import { LibraryScreen } from "@/features/library/LibraryScreen";
import { RouteError } from "@/app/RouteError";
import "@/styles/index.css";

/**
 * Screens are split by route rather than shipped as one bundle.
 *
 * The library is the only screen that runs at launch; chat, settings, stats,
 * discover, tracking and the game page were all parsed on the first frame and
 * kept resident for a visit that may never happen. Each lazy screen is fetched
 * once, then cached by the router. `LibraryScreen` stays eager so the first paint
 * is not a blank frame behind a network-shaped import.
 */
const router = createBrowserRouter([
  {
    path: "/",
    element: <AppShellWithMenu />,
    errorElement: <RouteError />,
    children: [
      { index: true, element: <LibraryScreen /> },
      {
        path: "game/:id",
        lazy: async () => {
          const { GameDetailScreen } = await import(
            "@/features/game/GameDetailScreen"
          );
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
        path: "guide",
        lazy: async () => {
          const { GuideScreen } = await import("@/features/guide/GuideScreen");
          return { Component: GuideScreen };
        },
      },
      {
        path: "settings",
        lazy: async () => {
          const { default: SettingsScreen } = await import(
            "@/features/settings/SettingsScreen"
          );
          return { Component: () => <SettingsScreen key="general" /> };
        },
      },
      {
        path: "sync",
        lazy: async () => {
          const { default: SettingsScreen } = await import(
            "@/features/settings/SettingsScreen"
          );
          return { Component: () => <SettingsScreen key="vndb" initialTab="vndb" /> };
        },
      },
      {
        path: "devices",
        lazy: async () => {
          const { default: SettingsScreen } = await import(
            "@/features/settings/SettingsScreen"
          );
          return {
            Component: () => <SettingsScreen key="transfer" initialTab="transfer" />,
          };
        },
      },
      {
        path: "tracking",
        lazy: async () => {
          const { TrackingScreen } = await import(
            "@/features/tracking/TrackingScreen"
          );
          return { Component: TrackingScreen };
        },
      },
      {
        path: "recommend",
        lazy: async () => {
          const { RecommendScreen } = await import(
            "@/features/recommend/RecommendScreen"
          );
          return { Component: RecommendScreen };
        },
      },
      {
        path: "chat",
        lazy: async () => {
          const { ChatScreen } = await import("@/features/chat/ChatScreen");
          return { Component: ChatScreen };
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
    <AppProvider>
      <RouterProvider router={router} />
    </AppProvider>
  </StrictMode>,
);
