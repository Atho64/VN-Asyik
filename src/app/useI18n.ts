import { useContext } from "react";

import { AppContext } from "@/app/AppProvider";
import type { Translator } from "@/i18n";
import type { Locale } from "@/i18n";

interface UseI18n {
  t: Translator;
  locale: Locale;
}

/** Translation access for components. Throws outside the provider on purpose. */
export function useI18n(): UseI18n {
  const context = useContext(AppContext);
  if (!context) {
    throw new Error("useI18n must be used inside AppProvider");
  }
  return { t: context.t, locale: context.locale };
}

/** True once the library has loaded or failed. */
export function useAppReady(): boolean {
  const context = useContext(AppContext);
  return context?.ready ?? false;
}
