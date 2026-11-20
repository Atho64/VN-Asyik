import { useI18n } from "@/app/useI18n";
import { ForYouPanel } from "@/features/recommend/ForYouPanel";

/**
 * Discover view: matches the VNDB catalogue against local library taste seeds
 * using deterministic tag, studio, staff, and voice-actor overlap.
 */
export function RecommendScreen() {
  const { t } = useI18n();

  return (
    <div className="discover discover--wide">
      <header className="discover__head">
        <h1 className="page-title">{t("recommend.title")}</h1>
        <p className="page-sub">{t("recommend.intro")}</p>
      </header>

      <ForYouPanel />
    </div>
  );
}
