//! Session-bound, opt-in selected-window vision. Pixels are transient only.
mod service;
mod store;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

use crate::ai::{chat::ChatConfig, vision::Image};
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use vn_core::companion::vision::{Budget, Crop, Scenes};

#[cfg(not(windows))]
mod windows {
    #[derive(Debug, Clone)]
    pub struct Target {
        pub title: String,
        #[cfg(test)]
        pub synthetic: bool,
    }
    impl Target {
        pub fn validate(&self) -> Result<(), &'static str> {
            #[cfg(test)]
            if self.synthetic {
                return Ok(());
            }
            Err("vision_unsupported")
        }
        pub fn foreground(&self) -> bool {
            #[cfg(test)]
            if self.synthetic {
                return true;
            }
            false
        }
    }
    pub fn list() -> Result<Vec<Target>, &'static str> {
        Err("vision_unsupported")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Identity {
    config: ChatConfig,
    profile: Option<String>,
    credential: Vec<u8>,
}
impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The credential is a key digest; Debug output must never carry it,
        // same redaction rule as the neighbouring transient types.
        f.debug_struct("Identity")
            .field("config", &self.config)
            .field("profile", &self.profile)
            .field("credential", &"[redacted]")
            .finish()
    }
}
impl Identity {
    fn of(provider: &crate::ai::profiles::Resolved) -> Self {
        use sha2::{Digest, Sha256};
        Self {
            config: provider.config.clone(),
            profile: provider.profile.as_ref().map(|p| p.id.clone()),
            credential: provider
                .key
                .as_ref()
                .map_or_else(Vec::new, |k| Sha256::digest(k.expose().as_bytes()).to_vec()),
        }
    }
}

pub(super) struct Frame {
    image: Image,
    width: u32,
    height: u32,
    at: i64,
    signature: Vec<u8>,
}
impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Frame(<transient>)")
    }
}

#[derive(Debug, Default)]
struct FeedState {
    latest: Option<Arc<Frame>>,
    error: Option<String>,
}
#[derive(Debug)]
struct Feed {
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<FeedState>>,
}
impl Drop for Feed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut s) = self.state.lock() {
            s.latest = None;
        }
    }
}

#[derive(Debug)]
struct Binding {
    session: String,
    playthrough: String,
    identity: Identity,
    target: windows::Target,
    crop: Crop,
    feed: Feed,
    preview: Option<(String, Arc<Frame>, std::time::Instant)>,
    armed: bool,
    quiet: bool,
    focused: bool,
    scenes: Scenes,
}

#[derive(Debug)]
pub(super) struct State {
    generation: u64,
    origin: std::time::Instant,
    budget_session: String,
    budget: Budget,
    candidates: Vec<(String, windows::Target)>,
    binding: Option<Binding>,
    accepted: Option<Identity>,
    pub(super) probing: bool,
    probe_cancel: Option<tokio::sync::watch::Sender<bool>>,
    status: String,
    error: Option<String>,
    pub(super) automatic_turn: bool,
    pub(super) visual_turn: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            generation: 0,
            origin: std::time::Instant::now(),
            budget_session: String::new(),
            budget: Budget::default(),
            candidates: Vec::new(),
            binding: None,
            accepted: None,
            probing: false,
            probe_cancel: None,
            status: "off".into(),
            error: None,
            automatic_turn: false,
            visual_turn: false,
        }
    }
}
impl State {
    pub(super) fn stop(&mut self) {
        self.generation += 1;
        self.binding = None;
        self.status = "off".into();
        self.error = None;
        if let Some(cancel) = self.probe_cancel.take() {
            cancel.send_replace(true);
        }
    }
    fn now(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
    pub(super) fn view(&self) -> View {
        View {
            status: self.status.clone(),
            error: self.error.clone(),
            window: self.binding.as_ref().map(|b| b.target.title.clone()),
            provider: self.binding.as_ref().map(|b| b.identity.config.clone()),
            generation: self.generation,
            support: if self.probing {
                "testing"
            } else if self
                .binding
                .as_ref()
                .is_some_and(|b| self.accepted.as_ref() == Some(&b.identity))
            {
                "accepted"
            } else if self.error.is_some() {
                "unavailable"
            } else {
                "unknown"
            }
            .into(),
            hourly: self.budget.hourly(self.now()),
            total: self.budget.total,
            watching: self.binding.as_ref().is_some_and(|b| b.armed),
            automatic: self.automatic_turn,
            quiet: self.binding.as_ref().is_some_and(|b| b.quiet),
        }
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub status: String,
    pub error: Option<String>,
    pub window: Option<String>,
    pub provider: Option<ChatConfig>,
    pub generation: u64,
    pub support: String,
    pub hourly: usize,
    pub total: u32,
    pub watching: bool,
    pub quiet: bool,
    pub automatic: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub id: String,
    pub title: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub token: String,
    pub data_url: String,
    pub width: u32,
    pub height: u32,
    pub crop: Crop,
    pub generation: u64,
    pub view: View,
}

fn encode(width: u32, height: u32, rgba: Vec<u8>, crop: Crop) -> Result<Frame, String> {
    if !crop.valid() {
        return Err("vision_invalid_crop".into());
    }
    let original = image::RgbaImage::from_raw(width, height, rgba).ok_or("vision_invalid_image")?;
    let x = width * crop.x / 1000;
    let y = height * crop.y / 1000;
    let w = width * crop.width / 1000;
    let h = height * crop.height / 1000;
    if w == 0 || h == 0 {
        return Err("vision_invalid_crop".into());
    }
    let img = image::DynamicImage::ImageRgba8(original)
        .crop_imm(x, y, w, h)
        .thumbnail(w.min(1280), h.min(1280))
        .to_rgb8();
    let signature =
        image::imageops::resize(&img, 96, 54, image::imageops::FilterType::Triangle).into_raw();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 80)
        .encode_image(&img)
        .map_err(|_| "vision_invalid_image")?;
    let image = Image::jpeg(bytes)?;
    Ok(Frame {
        image,
        width: img.width(),
        height: img.height(),
        at: crate::ai::conversation::store::now(),
        signature,
    })
}

#[cfg(windows)]
fn spawn_capture(target: windows::Target, crop: Crop) -> Result<Feed, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let state = Arc::new(Mutex::new(FeedState::default()));
    let shared = state.clone();
    let stopped = stop.clone();
    std::thread::Builder::new()
        .name("companion-capture".into())
        .spawn(move || {
            let run = || -> Result<(), String> {
                let mut capture = windows::Capture::start(target).map_err(str::to_owned)?;
                let start = std::time::Instant::now();
                let mut received = false;
                while !stopped.load(Ordering::Acquire) {
                    if let Some(p) = capture.next().map_err(str::to_owned)? {
                        let frame = encode(p.width, p.height, p.rgba, crop)?;
                        received = true;
                        if let Ok(mut s) = shared.lock()
                            && !stopped.load(Ordering::Acquire)
                        {
                            s.latest = Some(Arc::new(frame));
                        }
                    } else if !received && start.elapsed() > std::time::Duration::from_secs(4) {
                        return Err("vision_no_frame".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Ok(())
            };
            if let Err(error) = run()
                && let Ok(mut s) = shared.lock()
            {
                s.latest = None;
                s.error = Some(error);
            }
        })
        .map_err(|_| "vision_unavailable")?;
    Ok(Feed { stop, state })
}
#[cfg(not(windows))]
fn spawn_capture(_: windows::Target, _: Crop) -> Result<Feed, String> {
    Err("vision_unsupported".into())
}

impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preview")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}
