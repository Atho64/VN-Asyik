use super::*;

#[test]
fn legacy_personas_decode_without_a_profile_and_keep_their_prompt() {
    for persona in crate::chat::Persona::ALL {
        let options = SessionOptions {
            game_id: uuid::Uuid::new_v4().to_string(),
            playthrough_label: "Old reading".into(),
            knowledge: Knowledge::Observed,
            boundary: String::new(),
            source: DialogueSource::Manual,
            language: "en".into(),
            persona,
            profile: None,
        };
        let json = serde_json::to_string(&options).unwrap();
        assert!(!json.contains("profile"));
        let decoded: SessionOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, options);
        assert!(
            decoded
                .prompt(profile::PromptMode::Reaction)
                .contains("Mode: a brief reaction")
        );
        assert!(
            decoded
                .prompt(profile::PromptMode::Recap)
                .contains("Mode: recap only")
        );
        assert_eq!(
            decoded.prompt(profile::PromptMode::Answer),
            options.chat_options().prompt()
        );
    }
}

fn options() -> SessionOptions {
    SessionOptions {
        game_id: uuid::Uuid::new_v4().to_string(),
        playthrough_label: "First reading".into(),
        knowledge: Knowledge::Observed,
        boundary: String::new(),
        source: DialogueSource::Luna,
        language: "en".into(),
        persona: Persona::Companion,
        profile: None,
    }
}

#[test]
fn observed_policy_does_not_reinterpret_legacy_no_spoilers() {
    let mut options = options();
    options.validate().unwrap();
    let chat = options.chat_options();
    assert_eq!(chat.spoilers, Spoilers::Boundary);
    assert!(chat.boundary.contains("this reading context"));
    assert!(!chat.allow_online);
    options.knowledge = Knowledge::Metadata;
    assert_eq!(options.chat_options().spoilers, Spoilers::None);
    options.knowledge = Knowledge::All;
    assert_eq!(options.chat_options().spoilers, Spoilers::All);
}

#[test]
fn session_requires_a_valid_game_and_bounded_reading_label() {
    let mut options = options();
    options.playthrough_label = " ".into();
    assert_eq!(options.validate().unwrap_err(), "invalid_playthrough");
    options.playthrough_label = "x".repeat(81);
    assert!(options.validate().is_err());
    options.playthrough_label = "Replay".into();
    options.game_id = "not-a-game".into();
    assert_eq!(options.validate().unwrap_err(), "invalid_game");
}

fn cursor(generation: u64, sequence: u64) -> SourceCursor {
    SourceCursor {
        generation,
        sequence,
    }
}

fn line(generation: u64, sequence: u64, at: i64, text: &str) -> Observation {
    Observation {
        cursor: cursor(generation, sequence),
        at,
        text: text.into(),
    }
}

#[test]
fn dialogue_excludes_old_game_future_stale_and_post_turn_lines() {
    let lines = vec![
        line(1, 4, 150_000, "different connection"),
        line(2, 3, 150_000, "before the session"),
        line(2, 4, 1, "stale"),
        line(2, 5, 150_001, "future timestamp"),
        line(2, 6, 150_000, "current scene"),
        line(2, 7, 150_000, "arrived after turn started"),
    ];
    let selected = eligible_lines(&lines, cursor(2, 3), cursor(2, 6), 150_000);
    assert_eq!(selected, vec![lines[4].clone()]);
    assert!(eligible_lines(&lines, cursor(1, 3), cursor(2, 6), 150_000).is_empty());
}

#[test]
fn dialogue_window_is_bounded_by_bytes_and_lines_in_reading_order() {
    let lines: Vec<_> = (1..=50)
        .map(|i| line(1, i, 10, &format!("line {i}")))
        .collect();
    let selected = eligible_lines(&lines, cursor(1, 0), cursor(1, 50), 10);
    assert_eq!(selected.len(), MAX_SOURCE_LINES);
    assert_eq!(selected[0].text, "line 21");
    let lines = vec![
        line(1, 1, 10, &"あ".repeat(3000)),
        line(1, 2, 10, &"い".repeat(3000)),
    ];
    assert_eq!(
        eligible_lines(&lines, cursor(1, 0), cursor(1, 2), 10).len(),
        1
    );
}

#[test]
fn navigation_does_not_own_the_turn_but_pause_end_and_rebind_revoke_it() {
    let mut machine = SessionMachine::default();
    assert!(machine.begin_turn().is_err());
    machine.starting().unwrap();
    machine.activate(Some(cursor(1, 5))).unwrap();
    let first = machine.begin_turn().unwrap();
    // A snapshot/read/subscriber disappearing performs no transition.
    assert!(machine.accepts(first));
    assert!(machine.begin_turn().is_err());
    machine.pause().unwrap();
    assert!(!machine.accepts(first));
    machine.activate(Some(cursor(2, 99))).unwrap();
    let second = machine.begin_turn().unwrap();
    machine.finish_turn(first);
    assert!(
        machine.accepts(second),
        "late completion cannot finish a newer turn"
    );
    assert_eq!(machine.source_cursor, Some(cursor(2, 99)));
    machine.ending();
    assert!(!machine.accepts(second));
    machine.off();
    machine.starting().unwrap();
    machine.activate(None).unwrap();
    let third = machine.begin_turn().unwrap();
    assert!(!machine.accepts(first));
    assert!(!machine.accepts(second));
    machine.cancel_turn();
    assert!(!machine.accepts(third));
    assert_eq!(machine.lifecycle, Lifecycle::Active);
}
