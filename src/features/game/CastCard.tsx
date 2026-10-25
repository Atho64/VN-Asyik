import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import type { VndbCharacter } from "@/lib/ipc/types";

interface CastCardProps {
  characters: VndbCharacter[];
}

/** How many characters to show before the list collapses.
 *
 * A long VN lists dozens; the first row is the cast a reader recognises, and the
 * rest is behind a disclosure rather than an endless scroll. */
const VISIBLE_CHARACTERS = 12;

/**
 * The cast, as VNDB records it.
 *
 * Sorted main-first by VNDB's own `role` value, which is why the order is not
 * re-sorted here: their ordering already puts the protagonist at the front, and a
 * local sort by name would bury them.
 *
 * Portraits are VNDB CDN URLs rendered directly, not cached the way covers are.
 * A cast list is dozens of small images rather than one per game, and caching all
 * of them would trade a few hundred megabytes of disk for an offline view of a
 * page that already needs the network for everything else.
 *
 * The card renders nothing at all when there is no cast: an empty "Characters"
 * heading reads as a loading failure, the same reason `VndbFacts` assembles only
 * the rows it has.
 */
export function CastCard({ characters }: CastCardProps) {
  const { t } = useI18n();

  if (characters.length === 0) return null;

  const visible = characters.slice(0, VISIBLE_CHARACTERS);
  const rest = characters.slice(VISIBLE_CHARACTERS);

  const item = (character: VndbCharacter) => {
    // A character with neither name is not worth a blank tile.
    const label = character.name ?? character.original;
    if (!label) return null;
    return (
      <li key={character.id ?? label} className="cast__item">
        <span className="cast__art">
          {character.image?.url ? (
            // Lazy: the collapsed tail is in the DOM from the first paint, and
            // a few dozen portraits should not all be fetched for a row the
            // reader has not opened.
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
      </li>
    );
  };

  return (
    <Card>
      <CardHeader title={t("detail.cast")} />
      <ul className="cast">{visible.map(item)}</ul>
      {rest.length > 0 ? (
        <details className="facts__more">
          <summary className="caption">
            {t("detail.castMore", { count: rest.length })}
          </summary>
          <ul className="cast">{rest.map(item)}</ul>
        </details>
      ) : null}
    </Card>
  );
}
