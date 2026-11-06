import type { ReactNode } from "react";
import { Navigate } from "react-router";
import { useAppStore } from "./store";

/** Do not mount a Beta screen (or its effects) until the saved opt-in is known. */
export function BetaRoute({
  children,
  overlay = false,
}: {
  children: ReactNode;
  overlay?: boolean;
}) {
  const enabled = useAppStore((state) => state.prefs.beta_features);
  const loaded = useAppStore((state) => state.prefsLoaded);
  if (!loaded) return null;
  if (!enabled) return overlay ? null : <Navigate to="/settings" replace />;
  return children;
}
