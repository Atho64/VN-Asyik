import { listen } from "@tauri-apps/api/event";
import { z } from "zod";
import { call } from "./client";

const unit = z.null().transform(() => undefined);
export const companionWindowApi = {
  open: () => call("companion_open_overlay", unit),
  hide: () => call("companion_hide_overlay", unit),
  showMain: (path = "/companion") => call("companion_show_main", unit, { path }),
  listenMain: (navigate: (path: string) => void) =>
    listen<string>(
      "companion://open-main",
      ({ payload }) => {
        if (
          payload === "/companion" ||
          payload === "/settings" ||
          /^\/game\/[0-9a-f-]+$/i.test(payload)
        ) {
          navigate(payload);
        }
      },
      { target: "main" },
    ),
};
