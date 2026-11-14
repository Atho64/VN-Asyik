import { useCallback, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import { Sheet } from "@/components/Sheet";
import { CharacterDetails } from "@/features/game/CharacterDetails";
import { isSpoilerCharacter, sortSpoilerLast } from "@/features/game/traitFilters";
import type { VndbCharacter } from "@/lib/ipc/types";

interface CastCardProps {
  characters: VndbCharacter[];
}

/** Initial characters shown before the "show more" disclosure. */
const VISIBLE_CHARACTERS = 12;

/**
 * Character cast list that opens trait details in a sheet, moves spoiler-tagged
 * entries to the end while keeping them selectable, and hides when empty.
 */
export function CastCard({ characters }: CastCardProps) {
  const { t } = useI18n();
  // Keep spoiler characters last so the initial slice remains safe without shifting indices.
  const ordered = sortSpoilerLast(characters);
  const [openIndex, setOpenIndex] = useState<number | null>(null);
  // Single spoiler reveal toggle covers the whole sheet session.
  const [revealed, setRevealed] = useState(false);
  const [showAll, setShowAll] = useState(false);

  const close = useCallback(() => setOpenIndex(null), []);

  if (characters.length === 0) return null;

  const shown = showAll ? ordered : ordered.slice(0, VISIBLE_CHARACTERS);
  const rest = ordered.length - shown.length;
  const current = openIndex === null ? null : ordered[openIndex];

  const item = (character: VndbCharacter, index: number) => {
    // A character with neither name is not worth a blank tile.
    const label = character.name ?? character.original;
    if (!label) return null;
    const concealed = isSpoilerCharacter(character) && !revealed;
    return (
      <li key={character.id ?? label} className="cast__item">
        <button
          type="button"
          className="cast__summary"
          data-spoiler={concealed ? "true" : "false"}
          aria-label={concealed ? t("detail.castSpoilerTile") : undefined}
          onClick={() => setOpenIndex(index)}
        >
          <span className="cast__art">
            {concealed || !character.image?.url ? (
              <span className="cast__art--empty" aria-hidden="true" />
            ) : (
              <img src={character.image.url} alt="" loading="lazy" />
            )}
          </span>
          <span className="cast__text">
            {concealed ? (
              <span className="cast__spoiler-name">{t("detail.castSpoiler")}</span>
            ) : (
              <span className="cast__name">{label}</span>
            )}
            {!concealed && character.original && character.original !== label ? (
              <span className="cast__original">{character.original}</span>
            ) : null}
            {character.role ? (
              <span className="cast__role">
                {t(`detail.castRole.${character.role}`)}
              </span>
            ) : null}
          </span>
        </button>
      </li>
    );
  };

  return (
    <Card>
      <CardHeader title={t("detail.cast")} />
      <ul className="cast">
        {shown.map((character, index) => item(character, index))}
      </ul>
      {rest > 0 ? (
        <button
          type="button"
          className="linkbtn caption"
          onClick={() => setShowAll(true)}
        >
          {t("detail.castMore", { count: rest })}
        </button>
      ) : null}

      <Sheet
        open={current !== null}
        title={
          current && isSpoilerCharacter(current) && !revealed
            ? t("detail.castSpoiler")
            : (current?.name ?? current?.original ?? t("detail.cast"))
        }
        onClose={close}
      >
        {current ? (
          <CharacterDetails
            character={current}
            revealed={revealed}
            onToggleReveal={() => setRevealed((value) => !value)}
          />
        ) : null}
      </Sheet>
    </Card>
  );
}
