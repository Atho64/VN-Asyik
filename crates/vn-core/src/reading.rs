//! User-written reading checkpoints, separate from append-only playtime records.

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::ids::EntityId;
use crate::model::SyncMeta;

pub const MAX_NOTE_CHARS: usize = 4_000;
pub const MAX_SLOT_CHARS: usize = 80;
pub const MAX_POSITION_CHARS: usize = 200;

/// A journal entry keeps the names the reader saw when it was written. Renaming
/// or merging a route later must not rewrite their old reading checkpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingEntry {
    pub meta: SyncMeta,
    pub game_id: EntityId,
    pub session_id: Option<EntityId>,
    pub created_at: Timestamp,
    pub played_at: Option<Timestamp>,
    pub note: String,
    pub save_slot: String,
    pub route_name: String,
    pub chapter_name: String,
}

impl ReadingEntry {
    /// Validate both local input and incoming sync/backup rows.
    pub fn validate(&self) -> Result<(), String> {
        validate_text(
            &self.note,
            &self.save_slot,
            &self.route_name,
            &self.chapter_name,
        )
    }
}

pub fn validate_text(note: &str, slot: &str, route: &str, chapter: &str) -> Result<(), String> {
    if [note, slot, route, chapter]
        .iter()
        .all(|text| text.trim().is_empty())
    {
        return Err("empty_entry".to_owned());
    }
    for (name, text, max) in [
        ("note", note, MAX_NOTE_CHARS),
        ("save_slot", slot, MAX_SLOT_CHARS),
        ("route_name", route, MAX_POSITION_CHARS),
        ("chapter_name", chapter, MAX_POSITION_CHARS),
    ] {
        if text.chars().count() > max {
            return Err(format!("{name}_too_long"));
        }
    }
    Ok(())
}

impl crate::sync::MetaFields for ReadingEntry {
    fn meta(&self) -> &SyncMeta {
        &self.meta
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceId, model::DeviceTag, sync};

    #[test]
    fn accepts_a_manual_slot_without_requiring_a_note_or_detected_chapter() {
        assert!(validate_text("", "12", "", "").is_ok());
        assert!(validate_text(" \n", "", "　", "").is_err());
        assert!(validate_text(&"あ".repeat(MAX_NOTE_CHARS), "", "", "").is_ok());
        assert!(validate_text(&"あ".repeat(MAX_NOTE_CHARS + 1), "", "", "").is_err());
        assert!(validate_text("ok", &"x".repeat(MAX_SLOT_CHARS + 1), "", "").is_err());
    }

    #[test]
    fn journal_edits_and_deletes_converge_without_overwriting_other_entries() {
        let device = DeviceId::parse("a").unwrap();
        let entry = ReadingEntry {
            meta: SyncMeta::new(&device, Timestamp::from_millis(10)),
            game_id: EntityId::new(),
            session_id: None,
            created_at: Timestamp::from_millis(10),
            played_at: None,
            note: "At the station".to_owned(),
            save_slot: "12".to_owned(),
            route_name: String::new(),
            chapter_name: String::new(),
        };
        let mut peer = entry.clone();
        peer.meta.device_id_device = DeviceTag::new(&DeviceId::parse("b").unwrap());
        peer.note = "Before the choice".to_owned();
        let (forward, _) = sync::merge_rows(
            "reading_entries",
            std::slice::from_ref(&entry),
            std::slice::from_ref(&peer),
        );
        let (backward, _) = sync::merge_rows("reading_entries", &[peer.clone()], &[entry]);
        assert_eq!(forward, backward);
        assert_eq!(forward[0].note, "Before the choice");
        let mut deleted = peer.clone();
        deleted.meta.deleted = true;
        let (merged, _) = sync::merge_rows("reading_entries", &[deleted], &[peer]);
        assert!(merged[0].meta.deleted);
    }
}
