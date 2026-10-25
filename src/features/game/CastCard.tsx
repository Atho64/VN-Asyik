import { useCallback, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import { Sheet } from "@/components/Sheet";
import { CharacterDetails } from "@/features/game/CharacterDetails";
import type { VndbCharacter } from "@/lib/ipc/types";

interface CastCardProps {
  characters: VndbCharacter[];
}

/** How many characters the card lists before the rest go behind a disclosure.
 *
 * A long VN lists dozens; the first row is the cast a reader recognises. */
const VISIBLE_CHARACTERS = 12;

/**
 * The cast, as VNDB records it.
 *
 * Each entry is a button that opens a sheet, following the screenshot gallery
 * rather than expanding in place. A character carries up to ~90 traits, and
 * inline expansion turned the card into a wall of text that pushed the rest of
 * the page away — the sheet keeps the list scannable and gives the details room.
 *
 * Sorted main-first by VNDB's own `role` value, which is why the order is not
 * re-sorted here: their ordering already puts the protagonist at the front.
 *
 * Portraits come from VNDB's CDN rather than being cached like covers. A cast is
 * dozens of small images per game, and caching them all would trade hundreds of
 * megabytes for an offline view of a page that needs the network anyway.
 *
 * The card renders nothing at all when there is no cast: an empty "Characters"
 * heading reads as a loading failure.
 */
export function CastCard({ characters }: CastCardProps) {
  const { t } = useI18n();
  // Which character's sheet is open, by index into `characters`.
  const [openIndex, setOpenIndex] = useState<number | null>(null);
  // One reveal covers the whole sheet rather than one per character: a reader who
  // has decided spoilers are fine should not have to toggle again per person.
  const [revealed, setRevealed] = useState(false);
  const [showAll, setShowAll] = useState(false);

  const close = useCallback(() => setOpenIndex(null), []);

  if (characters.length === 0) return null;

  const shown = showAll ? characters : characters.slice(0, VISIBLE_CHARACTERS);
  const rest = characters.length - shown.length;
  const current = openIndex === null ? null : characters[openIndex];

  const item = (character: VndbCharacter, index: number) => {
    // A character with neither name is not worth a blank tile.
    const label = character.name ?? character.original;
    if (!label) return null;
    return (
      <li key={character.id ?? label} className="cast__item">
        <button
          type="button"
          className="cast__summary"
          onClick={() => setOpenIndex(index)}
        >
          <span className="cast__art">
            {character.image?.url ? (
              <img src={character.image.url} alt="" loading="lazy" />
            ) : (
              <span className="cast__art--empty" aria-hidden="true" />
            )}
          </span>
          <span className="cast__text">
            <span className="cast__name">{label}</span>
            {character.original && character.original !== label ? (
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
        title={current?.name ?? current?.original ?? t("detail.cast")}
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
