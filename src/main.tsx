import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router";

import { AppProvider } from "@/app/AppProvider";
import { AppShellWithMenu } from "@/app/AppShell";
import { GameDetailScreen } from "@/features/game/GameDetailScreen";
import { LibraryScreen } from "@/features/library/LibraryScreen";
import { SettingsScreen } from "@/features/settings/SettingsScreen";
import { StatsScreen } from "@/features/stats/StatsScreen";
import { RecommendScreen } from "@/features/recommend/RecommendScreen";
import { ChatScreen } from "@/features/chat/ChatScreen";
import { TrackingScreen } from "@/features/tracking/TrackingScreen";
import { RouteError } from "@/app/RouteError";
import "@/styles/index.css";

const router = createBrowserRouter([
  {
    path: "/",
    element: <AppShellWithMenu />,
    errorElement: <RouteError />,
    children: [
      { index: true, element: <LibraryScreen /> },
      { path: "game/:id", element: <GameDetailScreen /> },
      { path: "stats", element: <StatsScreen /> },
      { path: "settings", element: <SettingsScreen key="general" /> },
      { path: "sync", element: <SettingsScreen key="vndb" initialTab="vndb" /> },
      {
        path: "devices",
        element: <SettingsScreen key="transfer" initialTab="transfer" />,
      },
      { path: "tracking", element: <TrackingScreen /> },
      { path: "recommend", element: <RecommendScreen /> },
      { path: "chat", element: <ChatScreen /> },
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
