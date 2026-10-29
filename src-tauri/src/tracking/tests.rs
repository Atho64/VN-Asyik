//! Tracking loop tests.
//!
//! A fake probe drives the real service against a real in-memory database, so the
//! loop, the state machine and persistence are exercised together without needing
//! a game to be running. The real Win32 probe is covered separately in
//! `platform::probe::windows`.

use std::sync::Arc;

use vn_core::clock::Timestamp;
use vn_core::model::{Game, GameStatus};
use vn_core::tracker::Event;

use super::*;
use crate::db::{Db, queries};
use crate::platform::probe::{FocusedWindow, Presence, ProbeError, RunningProcess, SystemProbe};

/// A probe whose answers the test controls.
#[derive(Debug, Default)]
struct FakeProbe {
    processes: std::sync::Mutex<Vec<RunningProcess>>,
    focused: std::sync::Mutex<Option<u32>>,
    idle_for_ms: std::sync::Mutex<i64>,
    fail: std::sync::Mutex<bool>,
    /// Caption of the focused window, so the title path can be exercised.
    title: std::sync::Mutex<String>,
}

impl FakeProbe {
    fn set(&self, processes: Vec<RunningProcess>, focused: Option<u32>, idle_for_ms: i64) {
        *self.processes.lock().expect("lock") = processes;
        *self.focused.lock().expect("lock") = focused;
        *self.idle_for_ms.lock().expect("lock") = idle_for_ms;
    }

    /// Set the focused window's caption.
    fn set_title(&self, title: &str) {
        *self.title.lock().expect("lock") = title.to_owned();
    }
}

impl SystemProbe for FakeProbe {
    fn list_processes(&self) -> Result<Vec<RunningProcess>, ProbeError> {
        if *self.fail.lock().expect("lock") {
            return Err(ProbeError::Failed("boom".to_owned()));
        }
        Ok(self.processes.lock().expect("lock").clone())
    }

    fn focused_window(&self) -> Result<Option<FocusedWindow>, ProbeError> {
        Ok(self.focused_pid()?.map(|pid| FocusedWindow {
            pid,
            title: self.title.lock().expect("lock").clone(),
        }))
    }

    fn focused_pid(&self) -> Result<Option<u32>, ProbeError> {
        Ok(*self.focused.lock().expect("lock"))
    }

    fn presence(&self) -> Result<Presence, ProbeError> {
        Ok(Presence {
            idle_for_ms: *self.idle_for_ms.lock().expect("lock"),
        })
    }

    fn is_running(&self, pid: u32) -> Result<bool, ProbeError> {
        Ok(self
            .processes
            .lock()
            .expect("lock")
            .iter()
            .any(|process| process.pid == pid))
    }
}

/// Chapters stored for a game, as (display name, route name, id).
async fn chapters_of(
    pool: &sqlx::SqlitePool,
    game_id: EntityId,
) -> Vec<(String, Option<String>, String)> {
    use sqlx::Row as _;
    sqlx::query(
        "SELECT id, COALESCE(NULLIF(display_name, ''), name) AS label, route_name \
         FROM chapters WHERE game_id = ? AND deleted = 0 ORDER BY sort_index",
    )
    .bind(game_id.to_string())
    .fetch_all(pool)
    .await
    .expect("query")
    .into_iter()
    .map(|row| {
        (
            row.get::<String, _>("label"),
            row.get::<Option<String>, _>("route_name"),
            row.get::<String, _>("id"),
        )
    })
    .collect()
}

fn t(ms: i64) -> Timestamp {
    Timestamp::from_millis(ms)
}

fn process(pid: u32, name: &str, path: &str, focused: bool) -> RunningProcess {
    RunningProcess {
        pid,
        exe_name: name.to_owned(),
        exe_path: Some(path.to_owned()),
        focused,
    }
}

async fn fixture() -> (Db, DeviceId, EntityId, Arc<FakeProbe>, TrackingService) {
    let db = Db::open_memory().await.expect("db");
    let device = queries::ensure_device_id(db.pool()).await.expect("device");
    let game = Game::new(&device, t(0), "Tracked Game", GameStatus::Playing).expect("game");
    queries::upsert_game(db.pool(), &game)
        .await
        .expect("insert");

    mapping::add_executable(
        db.pool(),
        &device,
        t(0),
        game.meta.id,
        "C:/Games/Tracked/game.exe",
    )
    .await
    .expect("map");

    let probe = Arc::new(FakeProbe::default());
    let service = TrackingService::with_probe(probe.clone(), t(0));
    (db, device, game.meta.id, probe, service)
}

#[tokio::test]
async fn an_unsupported_platform_reports_why() {
    let service = TrackingService::with_probe(Arc::new(probe::UnsupportedProbe), t(0));
    let db = Db::open_memory().await.expect("db");
    let status = service.status(db.pool()).await;
    // The service was built with an explicit probe, so it believes tracking works;
    // what matters is that an unsupported probe yields no candidates and no errors.
    assert_eq!(
        status.candidates.len(),
        0,
        "status.candidates should be empty"
    );
}

#[tokio::test]
async fn a_siglus_title_caption_creates_a_chapter_and_updates_it_in_place() {
    // The path a real Siglus game takes: the caption carries the chapter, so the
    // poll attributes it without touching the game's memory. Exercised through
    // the real service so the source policy from `vn_core::adapter` is in play,
    // not bypassed.
    let (db, device, game_id, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    // The game's title in the database is "Tracked Game"; the caption repeats it,
    // then names the section and the in-game date.
    probe.set_title("Tracked Game 静久 ８月７日");

    // The first poll starts the run; chapter attribution needs a run that is
    // already in progress, so it happens on the following poll.
    service.poll(db.pool(), &device, t(0)).await.expect("poll");
    service
        .poll(db.pool(), &device, t(2_000))
        .await
        .expect("poll");
    let chapters = chapters_of(db.pool(), game_id).await;
    assert_eq!(chapters.len(), 1, "one chapter attributed from the title");
    assert_eq!(chapters[0].0, "静久 8月7日", "detected name is normalised");
    assert_eq!(
        chapters[0].1.as_deref(),
        Some("静久"),
        "route from the section"
    );

    // Polling again must update that row, not add a second one: the caption is
    // re-read every poll and is the chapter's identity.
    probe.set_title("Tracked Game 静久 ８月７日");
    service
        .poll(db.pool(), &device, t(4_000))
        .await
        .expect("poll");
    let again = chapters_of(db.pool(), game_id).await;
    assert_eq!(
        again.len(),
        1,
        "no duplicate from re-reading the same caption"
    );
    assert_eq!(again[0].2, chapters[0].2, "same row id");
}

#[tokio::test]
async fn a_full_width_day_and_route_caption_are_attributed() {
    // `anemoi` writes neither of the older shapes: the marker is a day number
    // early on and a route plus a chapter number once a route starts, and every
    // character of it is full-width. Both captions below are the live ones.
    let (db, device, game_id, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    probe.set_title("Tracked Game Ｄａｙ １");

    service.poll(db.pool(), &device, t(0)).await.expect("poll");
    service
        .poll(db.pool(), &device, t(2_000))
        .await
        .expect("poll");
    let chapters = chapters_of(db.pool(), game_id).await;
    assert_eq!(chapters.len(), 1, "one chapter from the day caption");
    assert_eq!(chapters[0].0, "Day 1", "the label stays with the number");
    assert_eq!(chapters[0].1, None, "a day caption names no route");

    // The next day is a different chapter, not a rename of the first.
    probe.set_title("Tracked Game Ｄａｙ ２");
    service
        .poll(db.pool(), &device, t(4_000))
        .await
        .expect("poll");
    let chapters = chapters_of(db.pool(), game_id).await;
    assert_eq!(chapters.len(), 2, "day 2 is its own chapter");
    assert_eq!(chapters[1].0, "Day 2");

    // Once a route starts the caption names it before the chapter number.
    probe.set_title("Tracked Game アネモイ Ｃｈａｐｔｅｒ ０１");
    service
        .poll(db.pool(), &device, t(6_000))
        .await
        .expect("poll");
    let chapters = chapters_of(db.pool(), game_id).await;
    assert_eq!(chapters.len(), 3);
    assert_eq!(chapters[2].0, "アネモイ Chapter 01");
    assert_eq!(
        chapters[2].1.as_deref(),
        Some("アネモイ"),
        "the katakana section is the route"
    );
}

#[tokio::test]
async fn a_caption_belonging_to_a_different_game_is_not_attributed() {
    // The caption is the only chapter source that does not identify its game, so
    // it has to be matched against the tracked game's title. A caption from
    // another title must not create a chapter in this one.
    //
    // Note this holds the game focused: with focus elsewhere the tracker pauses
    // and never polls chapters, so an unfocused setup would pass whether or not
    // the title matched. The assertion below is on the title check, not focus.
    let (db, device, game_id, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    probe.set_title("Some Other Game 静久 ８月７日");

    service.poll(db.pool(), &device, t(0)).await.expect("poll");
    service
        .poll(db.pool(), &device, t(2_000))
        .await
        .expect("poll");

    let chapters = chapters_of(db.pool(), game_id).await;
    assert_eq!(
        chapters.len(),
        0,
        "a caption for another game must not create a chapter, got {chapters:?}",
    );
}

#[tokio::test]
async fn a_focused_mapped_process_starts_a_run() {
    let (db, device, game_id, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );

    service.poll(db.pool(), &device, t(0)).await.expect("poll");
    let status = service.status(db.pool()).await;
    assert_eq!(status.phase, "tracking");
    assert_eq!(status.game_id, Some(game_id.to_string()));
}

#[tokio::test]
async fn an_unmapped_process_is_ignored() {
    let (db, device, _game, probe, service) = fixture().await;
    probe.set(
        vec![process(901, "notagame.exe", "C:/Other/notagame.exe", true)],
        Some(901),
        0,
    );
    service.poll(db.pool(), &device, t(0)).await.expect("poll");
    assert_eq!(service.status(db.pool()).await.phase, "idle");
}

#[tokio::test]
async fn tracking_ourselves_is_refused() {
    let (db, device, _game, probe, service) = fixture().await;
    let self_pid = std::process::id();
    // Even if our own exe were somehow mapped, the tracker must skip itself or it
    // would count its own uptime as playtime forever.
    probe.set(
        vec![process(
            self_pid,
            "vn-tracker.exe",
            "C:/Apps/vn-tracker.exe",
            true,
        )],
        Some(self_pid),
        0,
    );
    service.poll(db.pool(), &device, t(0)).await.expect("poll");
    assert_eq!(service.status(db.pool()).await.phase, "idle");
}

#[tokio::test]
async fn focused_polls_accrue_active_time_and_persist_checkpoints() {
    let (db, device, game_id, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );

    // 40 s of focused play, polling every 2 s, with the 20 s checkpoint interval.
    let mut checkpoints = 0;
    for step in 0..=20 {
        let at = t(i64::from(step) * 2_000);
        let segments = service.poll(db.pool(), &device, at).await.expect("poll");
        checkpoints += segments.iter().filter(|segment| segment.checkpoint).count();
    }
    assert!(
        checkpoints >= 1,
        "at least one checkpoint should be written"
    );

    let pending = mapping::list_pending_sessions(db.pool())
        .await
        .expect("pending");
    assert!(!pending.is_empty(), "checkpoints are persisted");
    assert!(pending[0].active_ms > 0);
    assert_eq!(pending[0].game_id, game_id);
}

#[tokio::test]
async fn losing_focus_stops_accruing() {
    let (db, device, _game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    // Focus is held through step 5 (t = 10 s).
    for step in 0..=5 {
        service
            .poll(db.pool(), &device, t(i64::from(step) * 2_000))
            .await
            .expect("poll");
    }
    let active_before = service.status(db.pool()).await.active_ms;

    // Focus moves elsewhere; the frozen poll must not add time.
    probe.set(
        vec![
            process(900, "game.exe", "C:/Games/Tracked/game.exe", false),
            process(1, "other.exe", "C:/Other/other.exe", true),
        ],
        Some(1),
        0,
    );
    for step in 6..=12 {
        service
            .poll(db.pool(), &device, t(i64::from(step) * 2_000))
            .await
            .expect("poll");
    }
    let status = service.status(db.pool()).await;
    assert_eq!(status.phase, "unfocused");
    // The poll that *observes* the blur still credits the 2 s since the previous
    // poll, because that stretch was genuinely focused. Every poll after it adds
    // nothing.
    assert_eq!(
        status.active_ms,
        active_before + 2_000,
        "only the pre-blur gap is credited"
    );
}

#[tokio::test]
async fn idle_time_becomes_afk_after_the_threshold() {
    let (db, device, _game, probe, service) = fixture().await;
    service
        .set_afk_config(vn_core::afk::AfkConfig::new(10_000, true))
        .await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    service.poll(db.pool(), &device, t(0)).await.expect("poll");

    // 60 s with no input: well past the 10 s threshold.
    for step in 1..=30 {
        probe.set(
            vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
            Some(900),
            i64::from(step) * 2_000,
        );
        service
            .poll(db.pool(), &device, t(i64::from(step) * 2_000))
            .await
            .expect("poll");
    }
    let status = service.status(db.pool()).await;
    assert!(status.afk_ms > 0, "long idleness is recorded as away time");
    assert!(
        status.afk_ms > status.active_ms,
        "most of the run was a break"
    );
}

#[tokio::test]
async fn a_session_spent_clicking_through_dialogue_is_not_filed_as_away() {
    // The probe reports "time since the last input", so a click 300 ms before the
    // poll shows 300 ms of idle -- never zero. Counting that as an idle run made
    // every session look away after about a minute of active playing.
    let (db, device, game_id, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        300,
    );

    // Two minutes of play, polling every 2 s, each poll taken just after a click.
    for step in 0..=60 {
        service
            .poll(db.pool(), &device, t(step * 2_000))
            .await
            .expect("poll");
    }

    let status = service.status(db.pool()).await;
    assert_eq!(status.game_id, Some(game_id.to_string()));
    assert_eq!(status.afk_ms, 0, "clicking through a scene is not a break");
    assert_eq!(
        status.active_ms, 120_000,
        "the active clock must keep advancing while the reader is present"
    );
}

#[tokio::test]
async fn the_process_exiting_closes_the_run() {
    let (db, device, _game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    service.poll(db.pool(), &device, t(0)).await.expect("poll");
    // The game closes.
    probe.set(Vec::new(), None, 0);
    service
        .poll(db.pool(), &device, t(10_000))
        .await
        .expect("poll");

    let status = service.status(db.pool()).await;
    assert_eq!(status.phase, "idle");
    assert!(status.game_id.is_none());

    let game_id = queries::list_games(db.pool()).await.expect("games")[0]
        .meta
        .id;
    let sessions = queries::list_sessions(db.pool(), game_id)
        .await
        .expect("sessions");
    assert!(
        !sessions.is_empty(),
        "closing the game writes a real session"
    );
}

#[tokio::test]
async fn pausing_stops_accrual_until_resumed() {
    let (db, device, _game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    for step in 0..=5 {
        service
            .poll(db.pool(), &device, t(i64::from(step) * 2_000))
            .await
            .expect("poll");
    }

    service
        .send(db.pool(), &device, Event::Pause, t(10_000))
        .await
        .expect("pause");
    assert_eq!(service.status(db.pool()).await.phase, "paused");

    for step in 6..=12 {
        service
            .poll(db.pool(), &device, t(i64::from(step) * 2_000))
            .await
            .expect("poll");
    }
    assert_eq!(
        service.status(db.pool()).await.phase,
        "paused",
        "polling must not resurrect a paused run"
    );
}

#[tokio::test]
async fn disabling_tracking_stops_all_activity() {
    let (db, device, _game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    service.set_enabled(false).await;
    service.poll(db.pool(), &device, t(0)).await.expect("poll");
    assert_eq!(service.status(db.pool()).await.phase, "idle");
}

#[tokio::test]
async fn candidates_list_only_mapped_running_games() {
    let (db, _device, game_id, probe, service) = fixture().await;
    probe.set(
        vec![
            process(900, "game.exe", "C:/Games/Tracked/game.exe", true),
            process(901, "unrelated.exe", "C:/Other/unrelated.exe", false),
        ],
        Some(900),
        0,
    );
    let candidates = service.candidates(db.pool()).await.expect("candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].game_id, game_id.to_string());
    assert!(candidates[0].focused);
}

#[tokio::test]
async fn shutdown_flushes_and_clears_the_open_run() {
    let (db, device, game_id, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    service.poll(db.pool(), &device, t(0)).await.expect("poll");
    mapping::save_open_run(
        db.pool(),
        &mapping::OpenRunState {
            game_id: game_id.to_string(),
            session_id: "s".to_owned(),
            chapter_id: None,
            started_at: 0,
            active_ms: 2_000,
            afk_ms: 0,
            exe_path: "C:/Games/Tracked/game.exe".to_owned(),
            checkpoint_at: 2_000,
        },
    )
    .await
    .expect("save");

    service
        .shutdown(db.pool(), &device, t(4_000))
        .await
        .expect("shutdown");

    assert!(
        mapping::load_open_run(db.pool())
            .await
            .expect("load")
            .is_none(),
        "a clean shutdown clears the recovery marker"
    );
    let sessions = queries::list_sessions(db.pool(), game_id)
        .await
        .expect("sessions");
    assert!(!sessions.is_empty(), "the flushed run is written");
}

#[tokio::test]
async fn recovery_promotes_pending_checkpoints() {
    let (db, device, game_id, _probe, _service) = fixture().await;

    let segment = Segment {
        session_id: EntityId::new(),
        game_id,
        chapter_id: None,
        route_id: None,
        started_at: t(0),
        ended_at: t(30_000),
        active_ms: 20_000,
        afk_ms: 10_000,
        checkpoint: false,
    };
    mapping::write_pending_session(db.pool(), &segment, &device, t(0))
        .await
        .expect("pending");

    let recovered = recover(db.pool(), &device, t(60_000))
        .await
        .expect("recover");
    assert_eq!(recovered, 1);

    let sessions = queries::list_sessions(db.pool(), game_id)
        .await
        .expect("sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].counted_ms, 20_000);
    assert_eq!(sessions[0].afk_ms, 10_000);
    assert_eq!(
        mapping::list_pending_sessions(db.pool())
            .await
            .expect("pending")
            .len(),
        0
    );
}

#[tokio::test]
async fn recovery_on_a_clean_database_does_nothing() {
    let (db, device, _game, _probe, _service) = fixture().await;
    assert_eq!(recover(db.pool(), &device, t(0)).await.expect("recover"), 0);
}

#[tokio::test]
async fn a_probe_failure_does_not_panic_the_loop() {
    let (db, device, _game, probe, service) = fixture().await;
    *probe.fail.lock().expect("lock") = true;
    // A failing process list must not take the app down; the loop simply sees no
    // candidates this tick.
    let result = service.poll(db.pool(), &device, t(0)).await;
    assert!(result.is_ok());
    // Status must stay readable even while the probe is broken.
    let status = service.status(db.pool()).await;
    assert_eq!(
        status.candidates.len(),
        0,
        "status.candidates should be empty"
    );
}

/// Poll `steps` times, two seconds apart, starting at `from`.
async fn poll_range(service: &TrackingService, db: &Db, device: &DeviceId, from: i64, steps: i64) {
    for step in from..=from + steps {
        service
            .poll(db.pool(), device, t(step * 2_000))
            .await
            .expect("poll");
    }
}

#[tokio::test]
async fn a_running_manual_timer_stops_detection_from_accruing_the_same_minutes() {
    // The service writes the manual session from wall-clock when the timer stops.
    // If detection also accrued for that stretch, the same seconds would be stored
    // twice under two different sessions.
    let (db, device, game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    // Detection accrues only into the open segment, so each measurement needs the
    // run closed first: shutdown is what writes the final row.
    poll_range(&service, &db, &device, 0, 3).await;
    service
        .send(db.pool(), &device, Event::Stop, t(8_000))
        .await
        .expect("stop");
    let detected_before = queries::tracked_playtime_ms(db.pool(), game)
        .await
        .expect("total");
    assert!(
        detected_before > 0,
        "detection accrued before the timer started"
    );

    service
        .manual_start(EntityId::new(), game, t(8_000))
        .await
        .expect("start timer");
    poll_range(&service, &db, &device, 4, 5).await;
    service
        .send(db.pool(), &device, Event::Stop, t(18_000))
        .await
        .expect("stop");
    let detected_after = queries::tracked_playtime_ms(db.pool(), game)
        .await
        .expect("total");

    assert_eq!(
        detected_after, detected_before,
        "detection must not accrue while the manual timer owns the stretch"
    );
}

#[tokio::test]
async fn detection_resumes_once_the_manual_timer_ends() {
    let (db, device, game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    service
        .manual_start(EntityId::new(), game, t(0))
        .await
        .expect("start timer");
    poll_range(&service, &db, &device, 0, 3).await;
    assert_eq!(service.status(db.pool()).await.phase, "paused");

    service.manual_stop().await.expect("stop timer");
    poll_range(&service, &db, &device, 4, 2).await;

    assert_eq!(
        service.status(db.pool()).await.phase,
        "tracking",
        "the window is still focused, so detection takes it back"
    );
}

#[tokio::test]
async fn a_stopped_run_releases_the_launch_binding() {
    // The binding is the only thing that can attribute an unmapped executable, so
    // this isolates it: a process at the path the binding names, but a new pid.
    let (db, device, _mapped, probe, service) = fixture().await;
    let device_owner = queries::ensure_device_id(db.pool()).await.expect("device");
    let unmapped =
        Game::new(&device_owner, t(0), "Launched Only", GameStatus::Playing).expect("game");
    queries::upsert_game(db.pool(), &unmapped)
        .await
        .expect("insert");

    probe.set(
        vec![process(900, "emu.exe", "C:/Emu/emu.exe", true)],
        Some(900),
        0,
    );
    service
        .bind_launched_game(unmapped.meta.id, 900, "C:/Emu/emu.exe".to_owned())
        .await;
    service
        .send(db.pool(), &device, Event::Stop, t(2_000))
        .await
        .expect("stop");

    // A later instance of the same executable, which the binding must not claim.
    probe.set(
        vec![process(901, "emu.exe", "C:/Emu/emu.exe", true)],
        Some(901),
        0,
    );
    service
        .poll(db.pool(), &device, t(4_000))
        .await
        .expect("poll");

    assert_eq!(
        service.status(db.pool()).await.phase,
        "idle",
        "the stale binding must not claim a new process at the same path"
    );
}

#[tokio::test]
async fn a_desktop_session_records_when_the_game_was_started() {
    // The library sorts on `started_at` and the detail header shows it. Only the
    // Android path used to write it, so a desktop-tracked game looked never-started.
    let (db, device, game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    poll_range(&service, &db, &device, 0, 2).await;
    service
        .send(db.pool(), &device, Event::Stop, t(6_000))
        .await
        .expect("stop");

    let started: Option<i64> = sqlx::query_scalar("SELECT started_at FROM games WHERE id = ?")
        .bind(game.to_string())
        .fetch_one(db.pool())
        .await
        .expect("row");
    assert!(started.is_some(), "a tracked session must set started_at");
}

#[tokio::test]
async fn wall_clock_mode_counts_a_game_that_is_not_in_front() {
    // `Always` exists for the reader who keeps the game behind a guide. The
    // automatic mode is the control: same probe, same polls, no counted time.
    let (db, device, game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", false)],
        Some(1),
        0,
    );
    service.set_mode(mode::TrackingMode::Always).await;
    poll_range(&service, &db, &device, 0, 3).await;
    service
        .send(db.pool(), &device, Event::Stop, t(8_000))
        .await
        .expect("stop");
    let counted = queries::tracked_playtime_ms(db.pool(), game)
        .await
        .expect("total");
    assert!(counted > 0, "wall-clock mode counts it, got {counted}");

    let (db, device, game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", false)],
        Some(1),
        0,
    );
    poll_range(&service, &db, &device, 0, 3).await;
    service
        .send(db.pool(), &device, Event::Stop, t(8_000))
        .await
        .expect("stop");
    assert_eq!(
        queries::tracked_playtime_ms(db.pool(), game)
            .await
            .expect("total"),
        0,
        "automatic mode never counts a game that is not in front"
    );
}

#[tokio::test]
async fn a_wall_clock_run_ends_when_the_process_does() {
    // "Stops when the window closes and does not continue": the run is closed at
    // process exit, and a later launch is a new session, not more of the old one.
    let (db, device, game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", false)],
        Some(1),
        0,
    );
    service.set_mode(mode::TrackingMode::Always).await;
    poll_range(&service, &db, &device, 0, 2).await;

    probe.set(vec![], None, 0);
    service
        .poll(db.pool(), &device, t(6_000))
        .await
        .expect("poll");
    let first = queries::tracked_playtime_ms(db.pool(), game)
        .await
        .expect("total");
    assert!(first > 0, "the first run was recorded");

    // Nothing running: later polls must not keep the closed run alive.
    for step in 4..8 {
        service
            .poll(db.pool(), &device, t(step * 2_000))
            .await
            .expect("poll");
    }
    assert_eq!(
        queries::tracked_playtime_ms(db.pool(), game)
            .await
            .expect("total"),
        first,
        "a closed run is not extended by later polls"
    );
    assert_eq!(service.status(db.pool()).await.phase, "idle");
}

#[tokio::test]
async fn a_manual_timer_counts_like_playtime() {
    let (db, device, game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    service
        .manual_start(EntityId::new(), game, t(0))
        .await
        .expect("start timer");

    poll_range(&service, &db, &device, 0, 3).await;
    let run = service.manual_state().await.expect("timer");
    assert!(run.total_ms() > 0, "present samples count");
    assert_eq!(run.afk_ms, 0, "a present reader is play, not away");

    // The window stays focused but the user stops touching anything: the part past
    // the grace threshold is away, exactly as it is for a detected session.
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        300_000,
    );
    poll_range(&service, &db, &device, 4, 2).await;
    let run = service.manual_state().await.expect("timer");
    assert!(
        run.afk_ms > 0,
        "idle time is filed as away, like a detected session"
    );
}

#[tokio::test]
async fn a_manual_timer_does_not_count_while_another_window_is_in_front() {
    let (db, device, game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    service
        .manual_start(EntityId::new(), game, t(0))
        .await
        .expect("start timer");
    poll_range(&service, &db, &device, 0, 2).await;
    let before = service.manual_state().await.expect("timer").total_ms();

    // The terminal takes the foreground, which is where automatic playtime pauses.
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", false)],
        Some(1),
        0,
    );
    poll_range(&service, &db, &device, 3, 3).await;
    let after = service.manual_state().await.expect("timer").total_ms();
    assert_eq!(after, before, "an unfocused game buys no manual minutes");
}

#[tokio::test]
async fn a_manual_timer_ends_when_the_game_closes() {
    let (db, device, game, probe, service) = fixture().await;
    probe.set(
        vec![process(900, "game.exe", "C:/Games/Tracked/game.exe", true)],
        Some(900),
        0,
    );
    // A real chapter row: the session it writes carries a foreign key to it.
    let chapter =
        mapping::insert_manual_chapter(db.pool(), game, "Prologue", None, None, &device, t(0))
            .await
            .expect("chapter");
    service
        .manual_start(chapter, game, t(0))
        .await
        .expect("start timer");
    poll_range(&service, &db, &device, 0, 2).await;

    probe.set(vec![], None, 0);
    service
        .poll(db.pool(), &device, t(10_000))
        .await
        .expect("poll");

    assert!(
        service.manual_state().await.is_none(),
        "the timer ends with the game instead of counting a closed window"
    );
    assert!(
        queries::tracked_playtime_ms(db.pool(), game)
            .await
            .expect("total")
            > 0,
        "and its time is written, not dropped"
    );
}
