//! Desktop playtime tracking: the loop that joins the OS probe to the state machine.
//!
//! Responsibilities are split so each part can be tested:
//!
//! * [`vn_core::tracker`] decides *what* time counts. Pure, heavily tested.
//! * [`crate::platform::probe`] answers *whether* the game is focused and whether
//!   the user is present. Thin Win32 wrappers.
//! * This module runs the loop: poll, feed events, persist segments.
//!
//! Polling is a fixed interval rather than an event hook. `SetWinEventHook` would be
//! lower latency, but it is a per-process global hook and this feature must stay
//! as boring as possible: a poll costs microseconds and cannot destabilise the
//! game. The interval is chosen so AFK detection stays accurate to the configured
//! threshold without waking the CPU often.

pub mod android;
pub mod chapter;
pub mod hook;
pub mod luna;
pub mod manual;
pub mod mapping;
pub mod title_config;

use std::sync::Arc;

use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::{Mutex, RwLock};
use vn_core::afk::AfkConfig;
use vn_core::clock::Timestamp;
use vn_core::ids::{DeviceId, EntityId};
use vn_core::tracker::{Event, Phase, Segment, TrackTarget, Tracker, TrackerConfig};

use crate::db::DbError;
use crate::platform::probe::{self, ProbeError, SystemProbe};

/// How often the loop polls the OS.
///
/// Two seconds is a compromise: fine enough that a chapter change or a blur is
/// attributed to the right second, coarse enough to be invisible in CPU usage.
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// What the UI shows while tracking.
#[derive(Debug, Clone, Serialize)]
pub struct TrackingStatus {
    /// Whether tracking is available on this platform at all.
    pub available: bool,
    /// Why it is unavailable, when it is.
    pub unavailable_reason: Option<String>,
    /// Current phase.
    pub phase: String,
    /// Tracked game, when a run is open.
    pub game_id: Option<String>,
    /// Tracked executable.
    pub exe_path: Option<String>,
    /// Process id of the tracked run, when one is open.
    ///
    /// Exposed so the UI can inspect the running engine without guessing a pid.
    pub pid: Option<u32>,
    /// Active milliseconds in the open run.
    pub active_ms: i64,
    /// Away milliseconds in the open run.
    pub afk_ms: i64,
    /// Chapter, when the adapter knows one.
    pub chapter_id: Option<String>,
    /// Running processes that match a known executable but are not tracked yet.
    pub candidates: Vec<ProcessCandidate>,
}

/// A running executable that is mapped to a game and could be tracked.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessCandidate {
    /// Process id.
    pub pid: u32,
    /// Executable name.
    pub exe_name: String,
    /// Game it is mapped to.
    pub game_id: String,
    /// Whether it is focused right now.
    pub focused: bool,
}

/// Errors from the tracking service.
#[derive(Debug, thiserror::Error)]
pub enum TrackingError {
    /// The platform has no probe.
    #[error("playtime tracking is not supported on this platform yet")]
    Unsupported,
    /// The OS refused.
    #[error("the operating system refused the request: {0}")]
    Denied(String),
    /// A probe call failed.
    #[error("could not read process state: {0}")]
    Probe(String),
    /// Persistence failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<ProbeError> for TrackingError {
    fn from(err: ProbeError) -> Self {
        match err {
            ProbeError::Unsupported => Self::Unsupported,
            ProbeError::Denied(message) => Self::Denied(message),
            ProbeError::Failed(message) => Self::Probe(message),
        }
    }
}

/// Shared tracking service.
///
/// Owns the state machine behind a lock so a command can read status while the
/// loop is mid-tick, and owns the probe so tests can substitute one.
#[derive(Debug)]
pub struct TrackingService {
    tracker: Mutex<Tracker>,
    probe: Arc<dyn SystemProbe>,
    available: bool,
    unavailable_reason: Option<String>,
    /// Last observed run totals, so status reads do not need to lock the machine.
    snapshot: RwLock<RunSnapshot>,
    /// Whether automatic tracking is enabled by the user.
    enabled: RwLock<bool>,
    /// A chapter the user is timing by hand, when there is one.
    ///
    /// Held here rather than in the tracker because it is driven by the user, not
    /// by the OS, and because it must be readable for status without locking the
    /// tracker itself.
    manual: RwLock<Option<manual::ManualRun>>,
    /// Whether the current pause was caused by a manual timer taking over.
    manual_paused: RwLock<bool>,
    /// Explicit per-game launch binding on desktop: when the user clicks Play on
    /// a game in VN Asyik, the spawned PID/executable is bound to that `game_id`
    /// for its lifetime so shared emulators (`Vita3K`, RPCS3, PCSX2, Eden, Citron,
    /// shadPS4, PPSSPP, etc.) credit the launched game immediately even before or
    /// without a distinct window title.
    launched: RwLock<Option<LaunchedBinding>>,
}

/// Explicit per-game launch binding created when the user starts a game from VN Asyik.
#[derive(Debug, Clone)]
struct LaunchedBinding {
    game_id: EntityId,
    pid: u32,
    exe_path: String,
}

/// A copy of the open run's totals for the UI.
#[derive(Debug, Clone)]
struct RunSnapshot {
    game_id: Option<EntityId>,
    exe_path: Option<String>,
    pid: Option<u32>,
    active_ms: i64,
    afk_ms: i64,
    chapter_id: Option<EntityId>,
    phase: Phase,
}

impl Default for RunSnapshot {
    fn default() -> Self {
        Self {
            game_id: None,
            exe_path: None,
            pid: None,
            active_ms: 0,
            afk_ms: 0,
            chapter_id: None,
            phase: Phase::Idle,
        }
    }
}

impl TrackingService {
    /// The platform probe, so a caller can ask the OS a question directly.
    ///
    /// Used by the title-pattern tester, which needs the focused caption without
    /// starting a tracking loop.
    #[must_use]
    pub fn probe(&self) -> &dyn SystemProbe {
        self.probe.as_ref()
    }

    /// Build the service, creating the probe for this platform.
    #[must_use]
    pub fn new(now: Timestamp) -> Self {
        let (probe, available, reason): (Arc<dyn SystemProbe>, bool, Option<String>) =
            match probe::probe() {
                Ok(probe) => (Arc::from(probe), true, None),
                Err(err) => (
                    Arc::new(probe::UnsupportedProbe),
                    false,
                    Some(err.to_string()),
                ),
            };
        Self {
            tracker: Mutex::new(Tracker::new(TrackerConfig::default(), now)),
            probe,
            available,
            unavailable_reason: reason,
            snapshot: RwLock::new(RunSnapshot::default()),
            enabled: RwLock::new(true),
            manual: RwLock::new(None),
            manual_paused: RwLock::new(false),
            launched: RwLock::new(None),
        }
    }

    /// Build the service around an explicit probe. Used by tests.
    #[must_use]
    pub fn with_probe(probe: Arc<dyn SystemProbe>, now: Timestamp) -> Self {
        Self {
            tracker: Mutex::new(Tracker::new(TrackerConfig::default(), now)),
            probe,
            available: true,
            unavailable_reason: None,
            snapshot: RwLock::new(RunSnapshot::default()),
            enabled: RwLock::new(true),
            manual: RwLock::new(None),
            manual_paused: RwLock::new(false),
            launched: RwLock::new(None),
        }
    }

    /// Bind a freshly spawned desktop game or emulator process to `game_id`.
    ///
    /// When the user launches a game via VN Asyik's Play button, the spawned PID
    /// (or child matching `exe_path`) is attributed directly to `game_id` so shared
    /// emulators (`Vita3K`, RPCS3, PCSX2, Eden, Citron, shadPS4, PPSSPP) track per-game
    /// playtime from launch without requiring window-caption heuristics first.
    pub async fn bind_launched_game(&self, game_id: EntityId, pid: u32, exe_path: String) {
        *self.launched.write().await = Some(LaunchedBinding {
            game_id,
            pid,
            exe_path,
        });
    }

    /// Snapshot of the manual timer, for status reads.
    pub async fn manual_state(&self) -> Option<manual::ManualRun> {
        self.manual.read().await.clone()
    }

    /// Start timing a chapter by hand.
    ///
    /// A manual run overrides detection for this game: two sources writing
    /// segments for the same stretch of time would double-count it.
    ///
    /// # Errors
    /// Fails when a timer is already running; the caller should stop it first
    /// rather than have the app silently discard the running total.
    pub async fn manual_start(
        &self,
        chapter_id: EntityId,
        game_id: EntityId,
        now: Timestamp,
    ) -> Result<manual::ManualRun, manual::ManualError> {
        let mut slot = self.manual.write().await;
        if slot.is_some() {
            return Err(manual::ManualError::AlreadyRunning);
        }
        let run = manual::ManualRun::start(chapter_id, game_id, now);
        *slot = Some(run.clone());
        Ok(run)
    }

    /// Start timing a route rather than a chapter.
    ///
    /// The same single slot, for the same reason: two writers for one stretch of time
    /// double-count it.
    ///
    /// # Errors
    /// Fails when a timer is already running.
    pub async fn manual_start_route(
        &self,
        route_id: EntityId,
        game_id: EntityId,
        now: Timestamp,
    ) -> Result<manual::ManualRun, manual::ManualError> {
        let mut slot = self.manual.write().await;
        if slot.is_some() {
            return Err(manual::ManualError::AlreadyRunning);
        }
        let run = manual::ManualRun::start_route(route_id, game_id, now);
        *slot = Some(run.clone());
        Ok(run)
    }

    /// Pause the manual timer.
    ///
    /// # Errors
    /// Fails when nothing is running, or it is already paused.
    pub async fn manual_pause(&self, now: Timestamp) -> Result<(), manual::ManualError> {
        let mut slot = self.manual.write().await;
        let Some(run) = slot.as_mut() else {
            return Err(manual::ManualError::NotRunning);
        };
        run.pause(now)
    }

    /// Resume the manual timer.
    ///
    /// # Errors
    /// Fails when nothing is running, or it is already running.
    pub async fn manual_resume(&self, now: Timestamp) -> Result<(), manual::ManualError> {
        let mut slot = self.manual.write().await;
        let Some(run) = slot.as_mut() else {
            return Err(manual::ManualError::NotRunning);
        };
        run.resume(now)
    }

    /// Stop the manual timer, returning the finished run so its time can be saved.
    ///
    /// # Errors
    /// Fails when nothing is running.
    pub async fn manual_stop(&self) -> Result<manual::ManualRun, manual::ManualError> {
        let mut slot = self.manual.write().await;
        slot.take().ok_or(manual::ManualError::NotRunning)
    }

    /// Apply the user's AFK configuration.
    ///
    /// The config is swapped in place rather than by rebuilding the [`Tracker`]:
    /// a rebuild would discard the open run, and this is called from Settings
    /// while a game is being tracked. The next poll classifies with the new
    /// threshold.
    pub async fn set_afk_config(&self, afk: AfkConfig) {
        self.tracker.lock().await.set_afk_config(afk);
    }

    /// Turn automatic tracking on or off.
    pub async fn set_enabled(&self, enabled: bool) {
        *self.enabled.write().await = enabled;
    }

    /// Whether automatic tracking is enabled.
    pub async fn is_enabled(&self) -> bool {
        *self.enabled.read().await
    }

    /// Current status for the UI.
    pub async fn status(&self, pool: &SqlitePool) -> TrackingStatus {
        let snapshot = self.snapshot.read().await.clone();
        // A failing probe must not make the status call itself fail: the UI still
        // needs to show the phase and any run in progress.
        let candidates = self.candidates(pool).await.unwrap_or_default();
        TrackingStatus {
            available: self.available,
            unavailable_reason: self.unavailable_reason.clone(),
            phase: phase_name(snapshot.phase).to_owned(),
            game_id: snapshot.game_id.map(|id| id.to_string()),
            exe_path: snapshot.exe_path.clone(),
            pid: snapshot.pid,
            active_ms: snapshot.active_ms,
            afk_ms: snapshot.afk_ms,
            chapter_id: snapshot.chapter_id.map(|id| id.to_string()),
            candidates,
        }
    }

    /// Running processes that are already mapped to a game.
    ///
    /// This is what makes "Start tracking" possible without the user hunting for
    /// a pid: the app already knows which executables belong to which game.
    pub async fn candidates(
        &self,
        pool: &SqlitePool,
    ) -> Result<Vec<ProcessCandidate>, TrackingError> {
        if !self.available {
            return Ok(Vec::new());
        }
        let processes = self.probe.list_processes()?;
        let focused_win = self.probe.focused_window().ok().flatten();
        let mut out = Vec::new();
        for process in processes {
            if process.pid == std::process::id() {
                continue;
            }
            let win_title = focused_win
                .as_ref()
                .filter(|w| w.pid == process.pid)
                .map(|w| w.title.as_str());
            if let Some(mapped) = mapping::game_for_executable_with_title(
                pool,
                process.exe_path.as_deref(),
                &process.exe_name,
                win_title,
            )
            .await?
            {
                out.push(ProcessCandidate {
                    pid: process.pid,
                    exe_name: process.exe_name,
                    game_id: mapped.game_id,
                    focused: process.focused,
                });
            }
        }
        Ok(out)
    }

    /// Feed one poll into the state machine.
    ///
    /// Returns the segments that must be persisted, so the caller owns the
    /// database write and this stays testable.
    pub async fn poll(
        &self,
        pool: &SqlitePool,
        device: &DeviceId,
        now: Timestamp,
    ) -> Result<Vec<Segment>, TrackingError> {
        if !self.available || !self.is_enabled().await {
            return Ok(Vec::new());
        }

        let mut tracker = self.tracker.lock().await;
        let mut events: Vec<(Event, Timestamp)> = Vec::new();

        // Is the tracked process still alive?
        if let Some(target) = tracker.target().cloned() {
            let alive = self.probe.is_running(target.pid).unwrap_or(false);
            if !alive {
                events.push((Event::ProcessExited, now));
                *self.launched.write().await = None;
            }
        }

        if tracker.target().is_none() {
            // Look for a mapped, focused process to start tracking.
            if let Some(process) = self.focused_mapped_process(pool).await? {
                let game_id = EntityId::parse(&process.1)?;
                tracker.start(
                    TrackTarget {
                        game_id,
                        pid: process.0,
                        exe_path: process.2,
                    },
                    now,
                );
                events.push((Event::FocusGained, now));
            }
        } else if let Some(target) = tracker.target().cloned()
            && let Some(event) = self.focus_event(&tracker, &target).await
        {
            events.push((event, now));
        }

        // Which chapter this time belongs to. The user's manual timer wins over
        // detection: they stated it explicitly, and running both would write two
        // segments for the same stretch of time.
        if matches!(tracker.phase(), Phase::Tracking)
            && let Some(target) = tracker.target().cloned()
        {
            let manual = self.manual.read().await.clone();
            let chapter_id = match manual::manual_override(manual.as_ref(), target.game_id) {
                Some(chapter_id) => Some(chapter_id),
                None => match self
                    .poll_chapter(pool, target.pid, target.game_id, device, now)
                    .await
                {
                    Ok(detected) => detected,
                    Err(err) => {
                        // Falling back to game-level attribution is correct. Reporting
                        // it is not optional: a failing chapter write otherwise looks
                        // identical to "this engine has no verified profile".
                        eprintln!("chapter detection failed: {err}");
                        None
                    }
                },
            };
            if let Some(chapter_id) = chapter_id {
                events.push((Event::ChapterChanged(Some(chapter_id)), now));
            }
        }

        let mut segments = Vec::new();
        for (event, at) in events {
            segments.extend(tracker.apply(event, at));
        }

        // Publish a snapshot for status reads.
        {
            let mut snapshot = self.snapshot.write().await;
            snapshot.phase = tracker.phase();
            snapshot.game_id = tracker.target().map(|target| target.game_id);
            snapshot.exe_path = tracker.target().map(|target| target.exe_path.clone());
            snapshot.pid = tracker.target().map(|target| target.pid);
            if let Some(open) = tracker.peek(now) {
                snapshot.active_ms = open.active_ms;
                snapshot.afk_ms = open.afk_ms;
                snapshot.chapter_id = open.chapter_id;
            }
        }

        // Persist checkpoints as pending rows so a crash loses at most one interval.
        for segment in &segments {
            if segment.checkpoint {
                mapping::write_pending_session(pool, segment, device, now).await?;
            } else {
                mapping::promote_pending_session(pool, segment, device, now).await?;
            }
        }

        // Record which run is open so `recover` has something to tell a clean close
        // from a crash. A run that ended has no marker: sending Stop/Shutdown clears it.
        self.persist_open_run(pool, &tracker, now).await?;

        Ok(segments)
    }

    /// Resolve the chapter currently playing in `pid`, creating its row on first
    /// sight.
    ///
    /// Returns `None` when the engine has no verified profile, is still loading, or
    /// is at a screen that is not a chapter (a title menu, a map). Those cases are
    /// normal and must not be reported as failures, or a user idling on the title
    /// screen would see errors instead of simply "no chapter".
    /// `game_id` is supplied by the caller rather than read here: `poll` already
    /// holds the tracker lock, and taking it again on the same task deadlocks.
    async fn poll_chapter(
        &self,
        pool: &SqlitePool,
        pid: u32,
        game_id: EntityId,
        device: &DeviceId,
        now: Timestamp,
    ) -> Result<Option<EntityId>, TrackingError> {
        // The order and the availability of each source come from `vn_core::adapter`
        // rather than from `if` statements here, so the policy is testable without
        // a game on the machine and an unsupported platform reports a reason
        // instead of quietly contributing nothing.
        // No engine family can be identified without reading the process, so
        // the window-title adapter is the only source that can ever answer.
        let engine: Option<String> = None;
        let order = vn_core::adapter::source_order(
            engine.as_deref(),
            vn_core::adapter::Platform::current(),
        );

        for source in order {
            let detected = match source {
                vn_core::adapter::ChapterSource::WindowTitle => {
                    self.chapter_from_title(pool, pid, game_id).await
                }
                // Reading an engine's scene table meant reading another
                // process's memory at injected-pattern offsets, which went with
                // the hook. `vn_core::adapter::source_order` still lists the
                // source so the capability report stays honest about it; nothing
                // here can contribute one.
                vn_core::adapter::ChapterSource::MemoryScene => None,
            };

            if let Some(detected) = detected {
                return Ok(Some(
                    mapping::upsert_chapter(pool, game_id, &detected, device, now).await?,
                ));
            }
        }

        // No chapter this poll. That is a normal state (title screen, a menu, an
        // engine with no profile), so it is not an error.
        Ok(None)
    }

    /// Chapter from the focused window's caption.
    ///
    /// Preferred over memory: it needs no injection, so it keeps working when a
    /// game updates and cannot destabilise the game process.
    async fn chapter_from_title(
        &self,
        pool: &SqlitePool,
        pid: u32,
        game_id: EntityId,
    ) -> Option<mapping::DetectedChapter> {
        let window = self.probe.focused_window().ok().flatten()?;
        // The caption belongs to whichever window has focus; if that is not the
        // game we are tracking, it says nothing about this game's chapter.
        if window.pid != pid {
            return None;
        }

        let game_title = mapping::game_title(pool, game_id).await.ok().flatten()?;
        // Which format to read is the user's choice, so the verified Siglus
        // parser is one option rather than the only behaviour.
        let settings = crate::tracking::title_config::load(pool).await;
        let parsed = vn_core::title_settings::parse_with(&settings, &window.title, &game_title)?;

        let name = match &parsed.route {
            Some(route) => format!("{route} {}", parsed.position),
            None => parsed.position.clone(),
        };
        Some(mapping::DetectedChapter {
            // The caption is the identity: re-reading it next poll must match
            // this row rather than create a new chapter.
            raw_name: format!("title:{}", window.title.trim()),
            name,
            order: None,
            route_key: parsed
                .route
                .as_deref()
                .map(|route| format!("route:{route}")),
            route_name: parsed.route,
            // Kept so the UI can show exactly what the engine reported.
            position: Some(parsed.position),
        })
    }

    // Find a focused process whose executable is mapped to a game.
    async fn focused_mapped_process(
        &self,
        pool: &SqlitePool,
    ) -> Result<Option<(u32, String, String)>, TrackingError> {
        let Some(focused_pid) = self.probe.focused_pid()? else {
            return Ok(None);
        };
        let processes = self.probe.list_processes()?;
        let Some(process) = processes.iter().find(|p| p.pid == focused_pid) else {
            return Ok(None);
        };
        // Never track ourselves.
        if process.pid == std::process::id() {
            return Ok(None);
        }
        let title = self
            .probe
            .focused_window()
            .ok()
            .flatten()
            .filter(|w| w.pid == process.pid)
            .map(|w| w.title);

        if let Some(mapped) = mapping::game_for_executable_with_title(
            pool,
            process.exe_path.as_deref(),
            &process.exe_name,
            title.as_deref(),
        )
        .await?
        {
            return Ok(Some((
                process.pid,
                mapped.game_id,
                process.exe_path.clone().unwrap_or_default(),
            )));
        }

        // If the user explicitly launched this game/emulator from VN Asyik, honour
        // the per-game launch binding while that process (or its child at the same
        // executable path) is still running.
        // Only the exact pid qualifies. Matching on the executable path alone would
        // credit a later run of the same shared emulator host, including its own GUI
        // sitting idle, to whichever game was launched most recently.
        let launched = self.launched.read().await.clone();
        if let Some(binding) = launched
            && process.pid == binding.pid
        {
            return Ok(Some((
                process.pid,
                binding.game_id.to_string(),
                process
                    .exe_path
                    .clone()
                    .unwrap_or_else(|| binding.exe_path.clone()),
            )));
        }

        Ok(None)
    }

    /// The focus event for this poll, or none when the window is unfocused and idle.
    ///
    /// One event per poll: `FocusGained` when the window just came back, otherwise a
    /// tick carrying the presence sample. Sending both would advance the machine's
    /// clock twice and lose the gap between them.
    ///
    /// A manual timer owns the stretch it is timing, so detected accrual hands off
    /// rather than recording the same wall-clock minutes twice.
    async fn focus_event(&self, tracker: &Tracker, target: &TrackTarget) -> Option<Event> {
        let is_focused = self.probe.focused_pid().unwrap_or(None) == Some(target.pid);
        if !is_focused {
            return matches!(tracker.phase(), Phase::Tracking).then_some(Event::FocusLost);
        }

        let manual_owns = {
            let manual = self.manual.read().await;
            manual
                .as_ref()
                .is_some_and(|run| run.game_id == target.game_id)
        };
        let resume_from_manual = !manual_owns && *self.manual_paused.read().await;
        if resume_from_manual {
            *self.manual_paused.write().await = false;
        }
        if manual_owns && matches!(tracker.phase(), Phase::Tracking) {
            *self.manual_paused.write().await = true;
        }

        Some(match tracker.phase() {
            // The timer writes this stretch, and the gap since the last poll belongs
            // to it, so the detected segment is closed where it stands.
            Phase::Tracking if manual_owns => Event::HandoffToManual,
            // The timer that paused us has ended. A user-initiated pause is not this:
            // its phase was already Paused before any timer existed.
            Phase::Paused if resume_from_manual => Event::Resume,
            Phase::Tracking => Event::Tick {
                idle_for_ms: self
                    .probe
                    .presence()
                    .map_or(0, |presence| presence.idle_for_ms),
            },
            _ => Event::FocusGained,
        })
    }

    /// Send an explicit event to the machine, e.g. from a Pause button.
    pub async fn send(
        &self,
        pool: &SqlitePool,
        device: &DeviceId,
        event: Event,
        now: Timestamp,
    ) -> Result<Vec<Segment>, TrackingError> {
        let mut tracker = self.tracker.lock().await;
        let releases = matches!(event, Event::Stop | Event::Shutdown | Event::ProcessExited);
        let segments = tracker.apply(event, now);
        for segment in &segments {
            mapping::promote_pending_session(pool, segment, device, now).await?;
        }
        if releases {
            // A binding that outlived its run would attribute a later process to a
            // game the user already stopped.
            *self.launched.write().await = None;
        }
        let mut snapshot = self.snapshot.write().await;
        snapshot.phase = tracker.phase();
        if tracker.target().is_none() {
            snapshot.game_id = None;
            snapshot.exe_path = None;
            snapshot.pid = None;
            snapshot.active_ms = 0;
            snapshot.afk_ms = 0;
            snapshot.chapter_id = None;
        }
        Ok(segments)
    }

    /// Write or clear the open-run marker to match the machine's current state.
    async fn persist_open_run(
        &self,
        pool: &SqlitePool,
        tracker: &Tracker,
        now: Timestamp,
    ) -> Result<(), TrackingError> {
        let (Some(target), Some(open)) = (tracker.target(), tracker.peek(now)) else {
            mapping::clear_open_run(pool).await?;
            return Ok(());
        };
        let state = mapping::OpenRunState {
            game_id: target.game_id.to_string(),
            session_id: open.session_id.to_string(),
            chapter_id: open.chapter_id.map(|id| id.to_string()),
            started_at: open.started_at.as_millis(),
            active_ms: open.active_ms,
            afk_ms: open.afk_ms,
            exe_path: target.exe_path.clone(),
            checkpoint_at: now.as_millis(),
        };
        mapping::save_open_run(pool, &state).await?;
        Ok(())
    }

    /// Make sure a clean shutdown flushes the open run.
    pub async fn shutdown(
        &self,
        pool: &SqlitePool,
        device: &DeviceId,
        now: Timestamp,
    ) -> Result<Vec<Segment>, TrackingError> {
        let segments = self.send(pool, device, Event::Shutdown, now).await?;
        mapping::clear_open_run(pool).await?;
        Ok(segments)
    }
}

impl From<vn_core::model::ModelError> for TrackingError {
    fn from(err: vn_core::model::ModelError) -> Self {
        Self::Db(DbError::Corrupt(err.to_string()))
    }
}

pub(crate) fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle => "idle",
        Phase::Tracking => "tracking",
        Phase::Paused => "paused",
        Phase::Unfocused => "unfocused",
    }
}

/// Recover after an unclean shutdown.
///
/// Pending checkpoint rows are promoted into real sessions and the stale open-run
/// row is cleared. Nothing is discarded: a crash costs at most one poll interval,
/// which is what AGENTS.md §7 asks for.
pub async fn recover(
    pool: &SqlitePool,
    device: &DeviceId,
    now: Timestamp,
) -> Result<usize, DbError> {
    let pending = mapping::list_pending_sessions(pool).await?;
    let mut recovered = 0;
    for segment in pending {
        mapping::promote_pending_session(pool, &segment, device, now).await?;
        recovered += 1;
    }
    mapping::clear_open_run(pool).await?;
    Ok(recovered)
}

#[cfg(test)]
mod tests;
