import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { companionWindowApi } from "@/lib/ipc/companionWindow";
import { failureCode, useCompanionStore } from "./store";

export async function windowAction(action: () => Promise<void>) {
  useCompanionStore.setState({ error: null });
  try {
    await action();
  } catch (error) {
    useCompanionStore.setState({ error: failureCode(error) });
  }
}

export function OpenOverlayButton({ compact = false }: { compact?: boolean }) {
  const { t } = useI18n();
  const available = useCompanionStore(
    (state) => state.snapshot?.capabilities.overlay === "available",
  );
  if (!available) return null;
  return (
    <Button
      variant="ghost"
      size={compact ? "icon" : "default"}
      aria-label={t("companion.openOverlay")}
      title={t("companion.openOverlay")}
      onClick={() => void windowAction(companionWindowApi.open)}
    >
      <Icon name="screen" />
      {!compact && t("companion.openOverlay")}
    </Button>
  );
}
