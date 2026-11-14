import { useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Chip } from "@/components/Chip";
import type { VndbCharacter } from "@/lib/ipc/types";
import {
  MAX_SAFE_SPOILER,
  formatBirthday,
  formatBloodType,
  groupTraits,
  isSpoilerCharacter,
} from "@/features/game/traitFilters";

interface CharacterDetailsProps {
  character: VndbCharacter;
  /** Whether a reveal is currently allowed. Shared by the whole cast. */
  revealed: boolean;
  onToggleReveal: () => void;
}

/** Number of safe trait groups shown initially before folding behind a toggle. */
const VISIBLE_GROUPS = 6;

/**
 * Character detail sheet. Keeps spoiler-tagged traits (levels > 0) and spoiler
 * characters concealed behind an explicit opt-in reveal toggle.
 */
export function CharacterDetails({
  character,
  revealed,
  onToggleReveal,
}: CharacterDetailsProps) {
  const { t, locale } = useI18n();
  const [showAllTraits, setShowAllTraits] = useState(false);

  // Conceal portrait and name behind the same reveal toggle when the character itself is a spoiler.
  const spoilerCharacter = isSpoilerCharacter(character);
  const concealed = spoilerCharacter && !revealed;

  const birthday = formatBirthday(character.birthday, locale);
  const blood = formatBloodType(character.blood_type);
  const ceiling = revealed ? 2 : MAX_SAFE_SPOILER;
  const groups = concealed ? [] : groupTraits(character.traits, ceiling);

  // Count hidden spoiler traits so the reveal button shows how many remain concealed.
  const hiddenCount = character.traits.filter(
    (trait) => (trait.spoiler ?? 0) > MAX_SAFE_SPOILER,
  ).length;

  const facts: Array<[string, string]> = [];
  if (birthday) facts.push([t("detail.castBirthday"), birthday]);
  if (character.height) {
    facts.push([t("detail.castHeight"), `${character.height} cm`]);
  }
  if (character.weight) {
    facts.push([t("detail.castWeight"), `${character.weight} kg`]);
  }
  if (blood) facts.push([t("detail.castBlood"), blood]);
  if (character.voiced_by) {
    facts.push([t("detail.castVoicedBy"), character.voiced_by]);
  }

  const shownGroups = showAllTraits ? groups : groups.slice(0, VISIBLE_GROUPS);
  const remaining = groups.length - shownGroups.length;

  return (
    <div className="cast__details">
      {concealed || !character.image?.url ? (
        <span className="cast__portrait cast__portrait--empty" aria-hidden="true" />
      ) : (
        <img
          className="cast__portrait"
          src={character.image.url}
          alt=""
        />
      )}

      {!concealed && character.original && character.original !== character.name ? (
        <p className="cast__sheet-original">{character.original}</p>
      ) : null}

      {concealed ? (
        <p className="cast__sheet-original" role="status">
          {t("detail.castSpoilerBody")}
        </p>
      ) : facts.length > 0 ? (
        <dl className="cast__facts">
          {facts.map(([label, value]) => (
            <div key={label} className="cast__fact">
              <dt>{label}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
      ) : null}

      {!concealed && groups.length > 0 ? (
        <div className="cast__trait-block">
          <h3 className="caption">{t("detail.castTraits")}</h3>
          <dl className="cast__traits">
            {shownGroups.map(([group, names]) => (
              <div key={group} className="cast__trait">
                <dt>{group}</dt>
                <dd>{names.join(", ")}</dd>
              </div>
            ))}
          </dl>
          {remaining > 0 ? (
            <button
              type="button"
              className="linkbtn caption"
              onClick={() => setShowAllTraits(true)}
            >
              {t("detail.castTraitsMore", { count: remaining })}
            </button>
          ) : null}
        </div>
      ) : null}

      {/* A spoiler character offers the toggle even with no hidden traits:
          revealing the character itself is the whole point. */}
      {hiddenCount > 0 || spoilerCharacter ? (
        <div className="cast__spoiler">
          <Chip selected={revealed} onClick={onToggleReveal}>
            {revealed
              ? t("detail.castHideSpoilers")
              : spoilerCharacter && hiddenCount === 0
                ? t("detail.castRevealCharacter")
                : t("detail.castShowSpoilers", { count: hiddenCount })}
          </Chip>
        </div>
      ) : null}
    </div>
  );
}
