import { useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Chip } from "@/components/Chip";
import type { VndbCharacter } from "@/lib/ipc/types";
import {
  MAX_SAFE_SPOILER,
  formatBirthday,
  formatBloodType,
  groupTraits,
} from "@/features/game/traitFilters";

interface CharacterDetailsProps {
  character: VndbCharacter;
  /** Whether a reveal is currently allowed. Shared by the whole cast. */
  revealed: boolean;
  onToggleReveal: () => void;
}

/** Groups shown before the rest go behind a disclosure.
 *
 * Even at spoiler 0 a character carries around 40 traits across a dozen groups;
 * the first few are the ones a reader is looking at the sheet for. */
const VISIBLE_GROUPS = 6;

/**
 * Everything VNDB knows about one character, shown in a sheet.
 *
 * Traits are the bulk of it: a single CLANNAD character carries 94, of which 42
 * are at spoiler level 0. The rest name plot points, so they stay behind a
 * reveal that is off by default — the same rule the library applies to explicit
 * art, and for the same reason: showing them unasked is irreversible in the
 * reader's memory even if the UI can hide them again.
 */
export function CharacterDetails({
  character,
  revealed,
  onToggleReveal,
}: CharacterDetailsProps) {
  const { t, locale } = useI18n();
  const [showAllTraits, setShowAllTraits] = useState(false);

  const birthday = formatBirthday(character.birthday, locale);
  const blood = formatBloodType(character.blood_type);
  const ceiling = revealed ? 2 : MAX_SAFE_SPOILER;
  const groups = groupTraits(character.traits, ceiling);

  // How many traits the reveal would add, so the button says what it does rather
  // than "spoilers" with no sense of scale.
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
      {character.image?.url ? (
        <img
          className="cast__portrait"
          src={character.image.url}
          alt=""
        />
      ) : null}

      {character.original && character.original !== character.name ? (
        <p className="cast__sheet-original">{character.original}</p>
      ) : null}

      {facts.length > 0 ? (
        <dl className="cast__facts">
          {facts.map(([label, value]) => (
            <div key={label} className="cast__fact">
              <dt>{label}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
      ) : null}

      {groups.length > 0 ? (
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

      {hiddenCount > 0 ? (
        <div className="cast__spoiler">
          <Chip selected={revealed} onClick={onToggleReveal}>
            {revealed
              ? t("detail.castHideSpoilers")
              : t("detail.castShowSpoilers", { count: hiddenCount })}
          </Chip>
        </div>
      ) : null}
    </div>
  );
}
