import { useI18n } from "@/app/useI18n";

import chapterShot from "@/assets/guide/chapters.jpg";
import chatShot from "@/assets/guide/chat.jpg";
import discoverShot from "@/assets/guide/discover.jpg";
import gameShot from "@/assets/guide/game.jpg";
import libraryShot from "@/assets/guide/library.jpg";
import settingsShot from "@/assets/guide/settings.jpg";
import statsShot from "@/assets/guide/stats.jpg";
import trackingShot from "@/assets/guide/tracking.jpg";

/**
 * Every feature the app actually has, with a screenshot of this build.
 *
 * The pictures are real captures of the running app, not mockups: a guide that
 * shows a screen the user cannot find is worse than no guide. Each section states
 * what the feature does and the three things about it that are easy to get wrong,
 * because those are the questions the app itself cannot answer.
 */
const sections = [
  { key: "library", shot: libraryShot },
  { key: "tracking", shot: trackingShot },
  { key: "chapters", shot: chapterShot },
  { key: "game", shot: gameShot },
  { key: "stats", shot: statsShot },
  { key: "discover", shot: discoverShot },
  { key: "chat", shot: chatShot },
  { key: "settings", shot: settingsShot },
] as const;

export function GuideScreen() {
  const { t } = useI18n();

  return (
    <div className="guide">
      <header className="guide__head">
        <h1 className="page-title">{t("guide.title")}</h1>
        <p className="page-sub">{t("guide.intro")}</p>
      </header>

      {sections.map(({ key, shot }) => (
        <article key={key} className="guide__section">
          <img
            className="guide__shot"
            src={shot}
            alt=""
            loading="lazy"
            decoding="async"
          />
          <div className="guide__body">
            <h2 className="guide__title">{t(`guide.${key}.title`)}</h2>
            <p className="guide__text">{t(`guide.${key}.body`)}</p>
            <ul className="guide__points">
              <li>{t(`guide.${key}.p1`)}</li>
              <li>{t(`guide.${key}.p2`)}</li>
              <li>{t(`guide.${key}.p3`)}</li>
            </ul>
          </div>
        </article>
      ))}

      <p className="guide__footer">{t("guide.footer")}</p>
    </div>
  );
}
