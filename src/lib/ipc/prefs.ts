import { listen } from "@tauri-apps/api/event";

export const listenBetaFeatures = (onChange: (enabled: boolean) => void) =>
  listen<boolean>("prefs://beta-features", ({ payload }) => {
    if (typeof payload === "boolean") onChange(payload);
  });
