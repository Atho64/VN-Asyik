use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Text mode determining how Details and State fields are generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PresenceDetailsMode {
    #[default]
    Vndb,
    Custom,
    Manual,
}

/// User-configurable settings for Discord Rich Presence.
///
/// `#[serde(default)]` is load-bearing: a stored row can predate a field added
/// later, and without it that row fails to decode and every saved preference is
/// silently replaced by the defaults.
/// A per-field text transform, ported from `EwE`'s `*_transform` dictionaries.
///
/// Applied **after** template substitution and **before** Discord sanitizing, in
/// this order: `remove` substrings, then literal `replace` pairs, then `prefix`
/// and `suffix`, then `trim`. Order matters: prefixing before replacing would let
/// a replacement rule rewrite text the user added deliberately as decoration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TextTransform {
    pub prefix: String,
    pub suffix: String,
    /// Substrings removed wherever they appear.
    pub remove: Vec<String>,
    /// Literal replacements, applied in vector order (not `HashMap` order) so a
    /// rule that runs before another cannot swap places between saves.
    pub replace: Vec<ReplaceRule>,
    pub trim: bool,
}

/// One `old` → `new` literal replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaceRule {
    pub old: String,
    pub new: String,
}

impl TextTransform {
    /// Applies the transform.
    ///
    /// Order, taken from `EwE`'s `apply_text_transform`: remove, replace, then
    /// wrap with prefix/suffix, then trim. `trim` trims the **body** before it is
    /// wrapped, because trimming the wrapped line would eat the padding that a
    /// prefix like `"[ "` deliberately carries, turning `"[ x ]"` into `"[ x ]"`
    /// only by accident of spacing.
    ///
    /// A no-op transform returns the text byte-for-byte, so a settings row that
    /// predates this struct cannot silently restyle a template.
    #[must_use]
    pub fn apply(&self, text: &str) -> String {
        if self.is_noop() {
            return text.to_string();
        }
        let mut out = text.to_string();
        for sub in &self.remove {
            if !sub.is_empty() {
                out = out.replace(sub.as_str(), "");
            }
        }
        for rule in &self.replace {
            if !rule.old.is_empty() {
                out = out.replace(rule.old.as_str(), rule.new.as_str());
            }
        }
        if self.trim {
            out = out.trim().to_string();
        }
        if self.prefix.is_empty() && self.suffix.is_empty() {
            return out;
        }
        format!("{}{}{}", self.prefix, out, self.suffix)
    }

    /// Whether this transform can change anything at all.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.prefix.is_empty()
            && self.suffix.is_empty()
            && !self.trim
            && self.remove.iter().all(String::is_empty)
            && self.replace.iter().all(|r| r.old.is_empty())
    }
}

/// Which of the two rotating text fields a [`PresenceContext::cycle`] index
/// belongs to.
///
/// `EwE` keeps one independent cursor per field; a single shared counter made the
/// details and state lines advance in lockstep, so a two-item list on one field
/// and a four-item list on the other drifted apart and repeated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CycleField {
    #[default]
    Details,
    State,
}

/// Which image host a blurred or local cover may be uploaded to, so the card
/// can show art that Discord cannot fetch as a plain asset.
///
/// `EwE` uploads covers to an image host and sends the hosted URL; `None` (the
/// default) keeps every image off the network entirely, which is why this is
/// opt-in rather than a toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RpcImageHost {
    #[default]
    None,
    /// catbox.moe: no account or key required.
    Catbox,
    /// imgbb.com: requires the user's own API key.
    Imgbb,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresenceSettings {
    pub enabled: bool,
    pub client_id: String,
    pub details_mode: PresenceDetailsMode,
    pub manual_details: String,
    pub manual_state: String,
    /// The custom details template, used when [`Self::custom_details_items`] is
    /// empty. Kept so a settings row written before rotation existed still means
    /// what it meant.
    pub custom_details: String,
    pub custom_state: String,
    /// Templates the custom details line rotates through, one per entry.
    /// An empty list falls back to [`Self::custom_details`].
    pub custom_details_items: Vec<String>,
    pub custom_state_items: Vec<String>,
    pub custom_details_rotate: bool,
    pub custom_state_rotate: bool,
    pub custom_details_cycle_seconds: u32,
    pub custom_state_cycle_seconds: u32,
    /// Templates the VNDB details line rotates through. Empty means the built-in
    /// four facts (developer → rating → length → year), which is the phase-16
    /// behaviour and keeps working until the user edits the list.
    pub vndb_details_items: Vec<String>,
    /// Template for the VNDB **state** line. Empty means the built-in
    /// route/chapter/status logic.
    pub vndb_state_template: String,
    /// Swap which rendered line lands on `details` and which on `state`.
    pub vndb_swap_details_state: bool,
    pub custom_details_transform: TextTransform,
    pub custom_state_transform: TextTransform,
    pub vndb_details_transform: TextTransform,
    pub vndb_state_transform: TextTransform,
    pub show_cover: bool,
    pub show_buttons: bool,
    pub show_list_button: bool,
    /// The user's own VNDB profile page, shown as the second button.
    /// Empty leaves [`Self::show_list_button`] with nothing to link to.
    pub my_vndb_link: String,
    pub show_timestamps: bool,
    /// Keep the card on Discord while the game is running but another window has
    /// focus.
    ///
    /// On by default. A reader alt-tabs to a walkthrough, a notes file or Discord
    /// constantly, and clearing the card on every switch made it blink out for
    /// most of a session. The card still disappears when the game itself stops,
    /// so this never advertises a novel that is closed: the threshold is "the
    /// process is running", not "the window has focus".
    ///
    /// Turning it off restores the strict behaviour, where an unfocused game
    /// clears the card immediately.
    pub show_when_unfocused: bool,
    /// Template for the card **title**. `None` means "the VN's title", which is
    /// the `EwE` behaviour and the default.
    pub activity_name: Option<String>,
    /// Rotate the VNDB details line through developer → rating → length → year.
    /// `EwE` calls this the VNDB details cycle; it is on by default there.
    pub cycle_vndb_details: bool,
    /// Seconds each VNDB fact stays on screen. Floored at [`MIN_CYCLE_SECONDS`].
    pub cycle_seconds: u32,
    /// `EwE`'s `details` for a game with no VNDB link: a bare "Reading" reads as a
    /// status, where a generic word reads as a placeholder.
    pub vndb_loading_details: String,
    pub vndb_loading_state: String,
    /// Where a cover that Discord cannot fetch directly may be uploaded.
    /// [`RpcImageHost::None`] means nothing ever leaves the device.
    pub rpc_image_host: RpcImageHost,
}

/// Floor for the details cycle. Discord throttles presence updates to roughly one
/// per 15 s, so anything faster than this would be swallowed by the debouncer and
/// appear to skip entries rather than speed up.
pub const MIN_CYCLE_SECONDS: u32 = 5;

impl Default for PresenceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            // Default placeholder or shared client ID (user can override in settings)
            client_id: String::new(),
            details_mode: PresenceDetailsMode::Vndb,
            manual_details: String::new(),
            manual_state: String::new(),
            custom_details: "{vn_name}".to_string(),
            custom_state: "{chapter}".to_string(),
            custom_details_items: Vec::new(),
            custom_state_items: Vec::new(),
            custom_details_rotate: false,
            custom_state_rotate: false,
            custom_details_cycle_seconds: 5,
            custom_state_cycle_seconds: 5,
            vndb_details_items: Vec::new(),
            vndb_state_template: String::new(),
            vndb_swap_details_state: false,
            custom_details_transform: TextTransform::default(),
            custom_state_transform: TextTransform::default(),
            vndb_details_transform: TextTransform::default(),
            vndb_state_transform: TextTransform::default(),
            show_cover: true,
            show_buttons: true,
            show_list_button: true,
            my_vndb_link: String::new(),
            show_timestamps: true,
            show_when_unfocused: true,
            activity_name: None,
            cycle_vndb_details: true,
            cycle_seconds: 5,
            vndb_loading_details: "Reading".to_string(),
            vndb_loading_state: "Loading VNDB data...".to_string(),
            rpc_image_host: RpcImageHost::None,
        }
    }
}

/// Rich context collected from the running session, game, and VNDB facts.
#[derive(Debug, Clone, Default)]
pub struct PresenceContext {
    pub vn_name: String,
    pub activity_name: String,
    pub exe_name: String,
    pub game_key: String,
    pub window_title: String,
    pub large_text: String,
    pub vndb_id: Option<String>,
    pub vndb_url: Option<String>,
    /// VNDB's community rating, e.g. "8.86". Never the user's own — that is
    /// [`Self::my_rating`]; the two used to share one tag, which made
    /// `{vndb_rating}` mean different games depending on whether the user had
    /// rated it.
    pub vndb_rating: Option<String>,
    /// The user's own rating of the game, one decimal like the library UI
    /// ("7.5"). Empty when the user has not rated it.
    pub my_rating: Option<String>,
    pub vndb_developer: Option<String>,
    pub vndb_status: Option<String>,
    pub vndb_length: Option<String>,
    pub vndb_year: Option<String>,
    pub vndb_released: Option<String>,
    /// The game's original-script title, when it differs from the shown one.
    /// A local game can carry this too, so it is not a VNDB fact.
    pub title_original: Option<String>,
    /// The user's own library status ("Reading", "Completed", ...). Works for
    /// imports and hand-added games alike, unlike `vndb_status`, which is what
    /// the VNDB list says and is empty when the list is not linked.
    pub status: Option<String>,
    /// Total time recorded for this game across every session, formatted like
    /// `{elapsed_time}` ("12:34"). Empty until something has been played.
    pub playtime: Option<String>,
    pub elapsed_time: String,
    pub date: String,
    pub time: String,
    pub chapter: Option<String>,
    pub route: Option<String>,
    pub cover_url: Option<String>,
    pub is_cover_explicit: bool,
    /// A cover URL that already went through the concealment policy upstream —
    /// a blurred upload for explicit art, a hosted copy of a local custom cover.
    /// When present it wins over [`Self::cover_url`], which only ever carries a
    /// public URL Discord can fetch as-is.
    pub rpc_image: Option<String>,
    /// Whether the game has VNDB facts loaded yet. `EwE` shows "Reading" /
    /// "Loading VNDB data..." until they arrive, instead of a generic placeholder
    /// that looks like a rendering failure.
    pub vndb_loaded: bool,
    /// The game is running but another window has focus.
    ///
    /// The card is still shown when the settings allow it; this only lets a
    /// template say so, via `{game_state}`.
    pub is_unfocused: bool,
    pub session_start_epoch: Option<i64>,
    /// Which VNDB fact the rotating details line shows.
    ///
    /// Advanced by the poll loop rather than computed from a clock here: the
    /// domain crate stays pure, and a wall-clock read would make every call
    /// disagree with the debouncer's idea of "the same activity".
    pub cycle_index: u32,
    /// Which rotating entry the **state** line shows, when the state field is the
    /// one rotating. Tracked apart from [`Self::cycle_index`] so the two lines do
    /// not march in lockstep (`EwE` keeps one cursor per field for the same
    /// reason).
    pub state_cycle_index: u32,
}

/// Single button on Discord Rich Presence activity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PresenceButton {
    pub label: String,
    pub url: String,
}

/// Fully rendered Discord activity payload ready to dispatch to IPC or Gateway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceActivity {
    /// The line Discord renders as the card's **title**, above `details`.
    ///
    /// Without it Discord falls back to the Discord *application* name ("VN
    /// Asyik"), which buries the novel the user is actually reading. `EwE` always
    /// sends this, and it is what makes its card read "Steins;Gate" with the
    /// details underneath.
    pub name: Option<String>,
    pub details: Option<String>,
    pub state: Option<String>,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    pub start_timestamp: Option<i64>,
    pub buttons: Vec<PresenceButton>,
}

/// Renders a template string containing `{token}` placeholders.
/// Unknown tokens are preserved as-is.
pub fn render_template<S: std::hash::BuildHasher>(
    template: &str,
    tokens: &HashMap<&str, &str, S>,
) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start_idx) = rest.find('{') {
        out.push_str(&rest[..start_idx]);
        let after_start = &rest[start_idx + 1..];
        if let Some(end_idx) = after_start.find('}') {
            let key = &after_start[..end_idx];
            if let Some(val) = tokens.get(key) {
                out.push_str(val);
            } else {
                out.push('{');
                out.push_str(key);
                out.push('}');
            }
            rest = &after_start[end_idx + 1..];
        } else {
            out.push('{');
            rest = after_start;
        }
    }
    out.push_str(rest);
    out
}

/// Discord requires activity text of at least this many characters.
const DISCORD_TEXT_MIN_CHARS: usize = 2;
/// Discord allows activity text up to 128. Counted here in UTF-8 **bytes**, the
/// stricter reading: the documentation says "characters" but it has not been
/// verified against a live client for CJK text, and a Discord card shows only
/// about thirty characters anyway, so the cap costs nothing visible.
const DISCORD_TEXT_MAX_BYTES: usize = 128;

/// Sanitizes a string for Discord: trims it and caps it at
/// [`DISCORD_TEXT_MAX_BYTES`] on a character boundary.
///
/// Returns `None` when what is left is shorter than [`DISCORD_TEXT_MIN_CHARS`].
/// Discord rejects the **whole** activity for a one-character field, so the
/// field is omitted rather than padded with filler whose acceptance is unverified.
#[must_use]
pub fn sanitize_discord_string(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.chars().count() < DISCORD_TEXT_MIN_CHARS {
        return None;
    }
    if trimmed.len() <= DISCORD_TEXT_MAX_BYTES {
        return Some(trimmed.to_string());
    }
    let mut end = DISCORD_TEXT_MAX_BYTES;
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    let capped = trimmed[..end].trim_end();
    (capped.chars().count() >= DISCORD_TEXT_MIN_CHARS).then(|| capped.to_string())
}

/// Total playtime as `H:MM`, or minutes under an hour ("42 min").
///
/// Hours are not capped: a 300-hour library reads as `300:15`, which is more
/// useful on a presence card than wrapping at 24.
#[must_use]
pub fn playtime_label(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    if minutes < 60 {
        return format!("{minutes} min");
    }
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// Extracts tokens map from a `PresenceContext`.
///
/// Every known token is inserted, using `""` when the context has no value for
/// it. Leaving a known token out of the map would make [`render_template`] treat
/// it as an unknown token and print the literal `{chapter}` on Discord whenever
/// no chapter is detected.
#[must_use]
pub fn build_tokens(ctx: &PresenceContext) -> HashMap<&str, &str> {
    let mut map = HashMap::new();
    map.insert("vn_name", ctx.vn_name.as_str());
    map.insert("activity_name", ctx.activity_name.as_str());
    map.insert("exe_name", ctx.exe_name.as_str());
    map.insert("game_key", ctx.game_key.as_str());
    map.insert("window_title", ctx.window_title.as_str());
    map.insert("large_text", ctx.large_text.as_str());
    map.insert("vndb_id", ctx.vndb_id.as_deref().unwrap_or(""));
    map.insert("vndb_url", ctx.vndb_url.as_deref().unwrap_or(""));
    map.insert("vndb_rating", ctx.vndb_rating.as_deref().unwrap_or(""));
    map.insert("my_rating", ctx.my_rating.as_deref().unwrap_or(""));
    map.insert(
        "vndb_developer",
        ctx.vndb_developer.as_deref().unwrap_or(""),
    );
    // `EwE` normalizes VNDB's list status to "Reading" and defaults to it when the
    // list is unlinked, so `{vndb_status}` never renders as an empty line.
    map.insert("vndb_status", normalize_reading(ctx.vndb_status.as_deref()));
    map.insert("vndb_length", ctx.vndb_length.as_deref().unwrap_or(""));
    map.insert("vndb_year", ctx.vndb_year.as_deref().unwrap_or(""));
    map.insert("vndb_released", ctx.vndb_released.as_deref().unwrap_or(""));
    map.insert(
        "title_original",
        ctx.title_original.as_deref().unwrap_or(""),
    );
    map.insert("status", ctx.status.as_deref().unwrap_or(""));
    map.insert("playtime", ctx.playtime.as_deref().unwrap_or(""));
    map.insert("elapsed_time", ctx.elapsed_time.as_str());
    map.insert("date", ctx.date.as_str());
    map.insert("time", ctx.time.as_str());
    map.insert("chapter", ctx.chapter.as_deref().unwrap_or(""));
    map.insert("route", ctx.route.as_deref().unwrap_or(""));
    map.insert("game_state", game_state(ctx.is_unfocused));

    map
}

/// The VNDB facts a cycling details line rotates through, in order.
///
/// [`vndb_fact`] renders one; gaps (a VN with no rating yet) are skipped by the
/// caller rather than shown as a blank line.
pub const VNDB_FACT_COUNT: u32 = 4;

/// Renders the `index`-th rotating VNDB fact, or `None` when the context has no
/// value for it.
///
/// This is the built-in list used when [`PresenceSettings::vndb_details_items`]
/// is empty. A user list is rendered through [`render_rotating`] instead.
fn vndb_fact(index: u32, ctx: &PresenceContext) -> Option<String> {
    let text = match index % VNDB_FACT_COUNT {
        0 => ctx.vndb_developer.clone(),
        1 => ctx.vndb_rating.clone().map(|r| format!("{r} / 10")),
        2 => ctx.vndb_length.clone(),
        _ => ctx.vndb_year.clone(),
    };
    text.filter(|t| !t.trim().is_empty())
}

/// The first available VNDB fact at or after `start`, wrapping once.
///
/// A VN often lacks one of the four facts, and showing nothing for that fifth of
/// the cycle would make the details line blink out of existence.
fn cycling_vndb_fact(start: u32, ctx: &PresenceContext) -> Option<String> {
    (0..VNDB_FACT_COUNT).find_map(|offset| vndb_fact(start.wrapping_add(offset), ctx))
}

/// Renders the entry at `index` from a user-supplied rotation list.
///
/// Returns `None` when the list is empty, when the selected entry renders to
/// nothing, or when its result is too short for Discord. Gaps are skipped by
/// [`rotating_list_item`], the same way the built-in facts are.
fn render_list_item(
    items: &[String],
    index: u32,
    tokens: &HashMap<&str, &str>,
    transform: &TextTransform,
) -> Option<String> {
    let template = items.get(usize::try_from(index).ok()?)?;
    sanitize_discord_string(&transform.apply(&render_template(template, tokens)))
}

/// The first entry at or after `start` that actually renders, wrapping once.
///
/// A rotation list of `["{chapter}", "{route}"]` on a game with no detected
/// chapter would otherwise blank the line on every other tick.
fn rotating_list_item(
    items: &[String],
    start: u32,
    tokens: &HashMap<&str, &str>,
    transform: &TextTransform,
) -> Option<String> {
    let len = u32::try_from(items.len()).unwrap_or(0);
    (0..len).find_map(|offset| {
        render_list_item(
            items,
            start.wrapping_add(offset) % len.max(1),
            tokens,
            transform,
        )
    })
}

/// What `{game_state}` renders: whether the game currently has focus.
///
/// A template can use this to say `Reading` / `Idle` beside the title when the
/// user has turned "keep the card in the background" on.
const fn game_state(is_unfocused: bool) -> &'static str {
    if is_unfocused { "Idle" } else { "Reading" }
}

/// `EwE`'s phrase for a watched binary that has not reported a window title yet.
const DEFAULT_READING: &str = "Reading";

/// VNDB labels a list entry `Playing`; its own wording for a novel you are part
/// way through is `Reading`, which is what `EwE` shows on the card.
fn normalize_reading(status: Option<&str>) -> &str {
    match status.map(str::trim) {
        None | Some("Playing" | "") => DEFAULT_READING,
        Some(other) => other,
    }
}

/// Resolves the card title: the user's template when set, else the VN's title.
///
/// Falls back through the context's other names because a game row can have an
/// empty `title` in the wild (an import with a blank name), and Discord would
/// otherwise show the application name again.
fn activity_name(settings: &PresenceSettings, tokens: &HashMap<&str, &str>) -> Option<String> {
    let from_template = settings
        .activity_name
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .map(|t| render_template(t, tokens));

    from_template
        .as_deref()
        .and_then(sanitize_discord_string)
        .or_else(|| sanitize_discord_string(tokens.get("vn_name").copied().unwrap_or("")))
        .or_else(|| sanitize_discord_string(tokens.get("exe_name").copied().unwrap_or("")))
}

/// Renders one rotating field from a user list, or its fixed template.
///
/// `rotate` selects between the two; `index` is the cursor the caller advances.
fn rotating_field(
    items: &[String],
    rotate: bool,
    fixed: &str,
    index: u32,
    tokens: &HashMap<&str, &str>,
    transform: &TextTransform,
) -> Option<String> {
    if rotate && !items.is_empty() {
        return rotating_list_item(items, index, tokens, transform);
    }
    if fixed.trim().is_empty() {
        return None;
    }
    SANITIZE(&transform.apply(&render_template(fixed, tokens)))
}

/// Renders the built-in four-fact VNDB details rotation.
fn builtin_vndb_details(settings: &PresenceSettings, ctx: &PresenceContext) -> Option<String> {
    if settings.cycle_vndb_details {
        cycling_vndb_fact(ctx.cycle_index, ctx)
    } else {
        ctx.vndb_developer.clone()
    }
}

/// The default VNDB state line: route, then chapter, else the list status.
fn builtin_vndb_state(ctx: &PresenceContext) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(ref route) = ctx.route {
        parts.push(route.clone());
    }
    if let Some(ref chapter) = ctx.chapter {
        parts.push(chapter.clone());
    }
    if parts.is_empty() {
        return ctx.vndb_status.clone();
    }
    Some(parts.join(" - "))
}

/// `details` and `state` before sanitizing, plus the pair after a swap.
fn render_lines(
    settings: &PresenceSettings,
    ctx: &PresenceContext,
) -> (Option<String>, Option<String>) {
    let tokens = build_tokens(ctx);
    match settings.details_mode {
        PresenceDetailsMode::Manual => (
            sanitize_discord_string(&settings.manual_details),
            sanitize_discord_string(&settings.manual_state),
        ),
        PresenceDetailsMode::Custom => (
            rotating_field(
                &settings.custom_details_items,
                settings.custom_details_rotate,
                &settings.custom_details,
                ctx.cycle_index,
                &tokens,
                &settings.custom_details_transform,
            ),
            rotating_field(
                &settings.custom_state_items,
                settings.custom_state_rotate,
                &settings.custom_state,
                ctx.state_cycle_index,
                &tokens,
                &settings.custom_state_transform,
            ),
        ),
        PresenceDetailsMode::Vndb => {
            // No VNDB link means no facts to draw on. `EwE` shows placeholders
            // here rather than an empty card, and the card must not fall back to
            // the game title, which the title line already carries.
            if !ctx.vndb_loaded {
                let (details, state) = (
                    sanitize_discord_string(&settings.vndb_loading_details),
                    sanitize_discord_string(&settings.vndb_loading_state),
                );
                return if settings.vndb_swap_details_state {
                    (state, details)
                } else {
                    (details, state)
                };
            }

            let details = if settings.vndb_details_items.is_empty() {
                builtin_vndb_details(settings, ctx)
                    .as_deref()
                    .map(|t| settings.vndb_details_transform.apply(t))
            } else {
                rotating_field(
                    &settings.vndb_details_items,
                    true,
                    "",
                    ctx.cycle_index,
                    &tokens,
                    &settings.vndb_details_transform,
                )
            };

            let state = if settings.vndb_state_template.trim().is_empty() {
                builtin_vndb_state(ctx)
                    .as_deref()
                    .map(|t| settings.vndb_state_transform.apply(t))
            } else {
                rotating_field(
                    std::slice::from_ref(&settings.vndb_state_template),
                    false,
                    &settings.vndb_state_template,
                    ctx.state_cycle_index,
                    &tokens,
                    &settings.vndb_state_transform,
                )
            };

            let details = details.and_then(|t| sanitize_discord_string(&t));
            let state = state.and_then(|t| sanitize_discord_string(&t));
            if settings.vndb_swap_details_state {
                (state, details)
            } else {
                (details, state)
            }
        }
    }
}

/// Wraps [`sanitize_discord_string`] for use as a function value.
const SANITIZE: fn(&str) -> Option<String> = sanitize_discord_string;

/// A button's URL is only accepted when the OS could actually open it.
///
/// The link arrives from a settings row the user typed by hand, but a stored row
/// is also a file other tooling can write, so the scheme is checked here rather
/// than trusting the webview that rendered the settings form.
fn push_button(buttons: &mut Vec<PresenceButton>, label: &str, url: &str) {
    let url = url.trim();
    let ok_scheme = url.starts_with("https://") || url.starts_with("http://");
    let has_space = url.chars().any(|c| c.is_whitespace() || c.is_control());
    if ok_scheme && !has_space && buttons.len() < MAX_BUTTONS {
        buttons.push(PresenceButton {
            label: label.to_string(),
            url: url.to_string(),
        });
    }
}

/// Discord renders at most two buttons on an activity.
pub const MAX_BUTTONS: usize = 2;

/// Builds a `PresenceActivity` from settings and current session context.
pub fn build_activity(settings: &PresenceSettings, ctx: &PresenceContext) -> PresenceActivity {
    let tokens = build_tokens(ctx);
    let name = activity_name(settings, &tokens);

    let (raw_details, raw_state) = render_lines(settings, ctx);

    // `details` is the line Discord always shows, so it falls back instead of
    // vanishing when the configured text is empty or a single character.
    let details = raw_details.or_else(|| Some("Visual Novel".to_string()));
    let state = raw_state;

    // Artwork policy:
    // Only send public HTTPS URLs for large_image if NOT explicit.
    // Local files or explicit art are NEVER sent to Discord — unless they arrive
    // as `rpc_image`, which the Tauri layer already ran through that same policy
    // (explicit art only ever arrives there as a blurred upload, never the
    // original bytes).
    let large_image = if let Some(ref rpc_img) = ctx.rpc_image {
        Some(rpc_img.clone()).filter(|u| u.starts_with("https://") || u.starts_with("http://"))
    } else if settings.show_cover && !ctx.is_cover_explicit {
        ctx.cover_url
            .as_ref()
            .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
            .cloned()
    } else {
        None
    };

    let large_text = if large_image.is_some() {
        sanitize_discord_string(&ctx.vn_name)
    } else {
        None
    };

    let start_timestamp = if settings.show_timestamps {
        ctx.session_start_epoch
    } else {
        None
    };

    // Two slots, in `EwE`'s order: the novel's VNDB page, then the user's own
    // list. Discord renders at most two, so a third would be dropped silently.
    let mut buttons = Vec::new();
    if settings.show_buttons
        && let Some(ref url) = ctx.vndb_url
    {
        push_button(&mut buttons, "View on VNDB", url);
    }
    if settings.show_list_button {
        push_button(&mut buttons, "My VNDB List", &settings.my_vndb_link);
    }

    PresenceActivity {
        name,
        details,
        state,
        large_image,
        large_text,
        small_image: None,
        small_text: None,
        start_timestamp,
        buttons,
    }
}

/// Activity key used for deduplicating identical updates to avoid spamming Discord.
///
/// `start_timestamp` is deliberately absent: it is derived from "now minus active
/// time", so it wobbles by a second between polls and would make every poll look
/// like a changed activity. The debouncer compares it separately, with tolerance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActivityKey {
    name: Option<String>,
    details: Option<String>,
    state: Option<String>,
    large_image: Option<String>,
    buttons: Vec<PresenceButton>,
}

impl From<&PresenceActivity> for ActivityKey {
    fn from(act: &PresenceActivity) -> Self {
        Self {
            // Part of the key because the title is the one line the user reads
            // first: switching games must not be mistaken for "same activity".
            name: act.name.clone(),
            details: act.details.clone(),
            state: act.state.clone(),
            large_image: act.large_image.clone(),
            buttons: act.buttons.clone(),
        }
    }
}

/// How far the start timestamp may drift between polls before it counts as a
/// different activity. Real changes (a new session) move it by far more.
const START_DRIFT_TOLERANCE_SECS: i64 = 5;

fn start_is_close(a: Option<i64>, b: Option<i64>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => (x - y).abs() <= START_DRIFT_TOLERANCE_SECS,
        (None, None) => true,
        _ => false,
    }
}

/// Throttler and debouncer for Discord activity updates.
///
/// Discord's guidance is at most one presence update per 15 seconds, which is
/// the default minimum interval.
#[derive(Debug, Clone)]
pub struct PresenceDebouncer {
    last_key: Option<ActivityKey>,
    last_start: Option<i64>,
    last_sent_time: i64,
    /// Minimum seconds between updates (default 15s)
    min_interval_secs: i64,
    /// Maximum seconds before re-sending identical presence (default 30s)
    max_suppress_secs: i64,
}

impl Default for PresenceDebouncer {
    fn default() -> Self {
        Self::new(15, 30)
    }
}

impl PresenceDebouncer {
    #[must_use]
    pub fn new(min_interval_secs: i64, max_suppress_secs: i64) -> Self {
        Self {
            last_key: None,
            last_start: None,
            last_sent_time: 0,
            min_interval_secs,
            max_suppress_secs,
        }
    }

    /// Determines whether the new activity should be sent at `now_secs`.
    pub fn should_send(&mut self, activity: &PresenceActivity, now_secs: i64) -> bool {
        let key = ActivityKey::from(activity);
        if let Some(ref last) = self.last_key
            && *last == key
            && start_is_close(self.last_start, activity.start_timestamp)
        {
            // Same payload: suppress until max_suppress_secs elapsed
            if now_secs - self.last_sent_time >= self.max_suppress_secs {
                self.last_sent_time = now_secs;
                self.last_start = activity.start_timestamp;
                return true;
            }
            return false;
        }

        // Payload changed: check minimum throttle interval
        if now_secs - self.last_sent_time < self.min_interval_secs {
            return false;
        }

        self.last_key = Some(key);
        self.last_start = activity.start_timestamp;
        self.last_sent_time = now_secs;
        true
    }

    /// Forgets what was last shown, so the next activity is treated as new.
    ///
    /// Used when the game stops, and also when a send did not reach Discord: an
    /// identical payload is then retried at the minimum interval instead of
    /// waiting out the suppression window as if Discord had received it. The send
    /// clock is kept, so a game that stops and starts again, or a failing
    /// connection, cannot bypass the minimum interval.
    pub fn reset(&mut self) {
        self.last_key = None;
        self.last_start = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_title_is_the_vn_title_not_the_app_name() {
        let ctx = PresenceContext {
            vn_name: "Steins;Gate".to_string(),
            ..Default::default()
        };
        let act = build_activity(&PresenceSettings::default(), &ctx);
        assert_eq!(act.name.as_deref(), Some("Steins;Gate"));
    }

    #[test]
    fn the_vndb_details_line_does_not_repeat_the_title() {
        // The regression this guards: title and details both showing the VN name.
        let ctx = PresenceContext {
            vn_name: "Steins;Gate".to_string(),
            vndb_developer: Some("5pb. / Nitroplus".to_string()),
            vndb_loaded: true,
            ..Default::default()
        };
        let act = build_activity(&PresenceSettings::default(), &ctx);
        assert_eq!(act.name.as_deref(), Some("Steins;Gate"));
        assert_eq!(act.details.as_deref(), Some("5pb. / Nitroplus"));
        assert_ne!(
            act.details, act.name,
            "the VN name must not appear on both lines"
        );
    }

    fn cycling_context() -> PresenceContext {
        PresenceContext {
            vn_name: "Steins;Gate".to_string(),
            vndb_loaded: true,
            vndb_developer: Some("5pb. / Nitroplus".to_string()),
            vndb_rating: Some("8.86".to_string()),
            my_rating: None,
            vndb_length: Some("Very long".to_string()),
            vndb_year: Some("2009".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn the_details_line_cycles_through_every_available_vndb_fact() {
        let settings = PresenceSettings::default();
        let seen: Vec<String> = (0..VNDB_FACT_COUNT)
            .map(|i| {
                let ctx = PresenceContext {
                    cycle_index: i,
                    ..cycling_context()
                };
                build_activity(&settings, &ctx)
                    .details
                    .expect("every fact is present")
            })
            .collect();
        assert_eq!(
            seen,
            vec!["5pb. / Nitroplus", "8.86 / 10", "Very long", "2009"]
        );
    }

    #[test]
    fn a_missing_fact_is_skipped_instead_of_blanking_the_details_line() {
        // A VN with no rating yet must not blink out on the second tick.
        let mut ctx = cycling_context();
        ctx.vndb_rating = None;
        ctx.cycle_index = 1;
        let act = build_activity(&PresenceSettings::default(), &ctx);
        assert_eq!(act.details.as_deref(), Some("Very long"));
    }

    #[test]
    fn a_game_with_no_vndb_facts_still_gets_a_details_line() {
        let ctx = PresenceContext {
            vn_name: "X".to_string(),
            ..Default::default()
        };
        let act = build_activity(&PresenceSettings::default(), &ctx);
        // No VNDB link: EwE shows a status phrase, not a generic placeholder that
        // reads like a rendering failure.
        assert_eq!(act.details.as_deref(), Some("Reading"));
        assert_eq!(act.state.as_deref(), Some("Loading VNDB data..."));
    }

    #[test]
    fn cycling_can_be_switched_off_to_a_fixed_developer_line() {
        let settings = PresenceSettings {
            cycle_vndb_details: false,
            ..Default::default()
        };
        let ctx = PresenceContext {
            cycle_index: 2,
            ..cycling_context()
        };
        let act = build_activity(&settings, &ctx);
        assert_eq!(act.details.as_deref(), Some("5pb. / Nitroplus"));
    }

    #[test]
    fn the_three_library_tags_render_from_the_context() {
        let ctx = PresenceContext {
            title_original: Some("CLANNAD".to_string()),
            status: Some("Completed".to_string()),
            playtime: Some("61:20".to_string()),
            ..Default::default()
        };
        let tokens = build_tokens(&ctx);
        assert_eq!(tokens["title_original"], "CLANNAD");
        assert_eq!(tokens["status"], "Completed");
        assert_eq!(tokens["playtime"], "61:20");
    }

    #[test]
    fn a_local_game_without_vndb_still_gets_the_library_tags() {
        // The point of `{status}` and `{playtime}` having no `vndb_` prefix: a
        // hand-added game has both, and must not render two blank lines because
        // it has no VNDB link.
        let ctx = PresenceContext {
            vn_name: "My Local VN".to_string(),
            status: Some("Reading".to_string()),
            playtime: Some("3 min".to_string()),
            vndb_loaded: false,
            ..Default::default()
        };
        let tokens = build_tokens(&ctx);
        assert_eq!(tokens["vn_name"], "My Local VN");
        assert_eq!(tokens["status"], "Reading");
        assert_eq!(tokens["playtime"], "3 min");
        // VNDB facts are empty, not literal braces.
        assert_eq!(tokens["vndb_rating"], "");
        assert_eq!(tokens["title_original"], "");
    }

    #[test]
    fn playtime_formats_minutes_below_an_hour_and_hours_above() {
        let settings = PresenceSettings {
            details_mode: PresenceDetailsMode::Custom,
            custom_details: "{playtime}".to_string(),
            ..Default::default()
        };
        let at = |ms: i64| {
            let ctx = PresenceContext {
                playtime: Some(playtime_label(ms)),
                ..Default::default()
            };
            build_activity(&settings, &ctx).details
        };
        assert_eq!(at(0).as_deref(), Some("0 min"));
        assert_eq!(at(59 * 60_000).as_deref(), Some("59 min"));
        assert_eq!(at(60 * 60_000).as_deref(), Some("1:00"));
        assert_eq!(at(61 * 60_000 + 30_000).as_deref(), Some("1:01"));
        assert_eq!(at(300 * 3_600_000).as_deref(), Some("300:00"));
    }

    #[test]
    fn a_title_template_overrides_the_plain_vn_name() {
        let settings = PresenceSettings {
            activity_name: Some("Reading {vn_name}".to_string()),
            ..Default::default()
        };
        let ctx = PresenceContext {
            vn_name: "Clannad".to_string(),
            ..Default::default()
        };
        let act = build_activity(&settings, &ctx);
        assert_eq!(act.name.as_deref(), Some("Reading Clannad"));
    }

    #[test]
    fn an_empty_or_one_character_title_falls_back_to_the_exe_name() {
        // Discord rejects a whole activity for a one-character name, so the title
        // has to come from somewhere else rather than vanish.
        let settings = PresenceSettings::default();
        let ctx = PresenceContext {
            vn_name: "Z".to_string(),
            exe_name: "gate.exe".to_string(),
            ..Default::default()
        };
        let act = build_activity(&settings, &ctx);
        assert_eq!(act.name.as_deref(), Some("gate.exe"));
    }

    #[test]
    fn a_changed_title_counts_as_a_new_activity() {
        let mut debouncer = PresenceDebouncer::default();
        let mut act = activity_started_at(1_000);
        assert!(debouncer.should_send(&act, 100));
        act.name = Some("Another VN".to_string());
        assert!(
            debouncer.should_send(&act, 115),
            "switching games must reach Discord"
        );
    }

    #[test]
    fn test_template_rendering() {
        let mut tokens = HashMap::new();
        tokens.insert("vn_name", "Steins;Gate");
        tokens.insert("chapter", "Chapter 1");

        let rendered = render_template("Reading {vn_name}: {chapter} ({unknown})", &tokens);
        assert_eq!(rendered, "Reading Steins;Gate: Chapter 1 ({unknown})");
    }

    #[test]
    fn a_missing_optional_token_renders_empty_not_its_literal_placeholder() {
        let settings = PresenceSettings {
            details_mode: PresenceDetailsMode::Custom,
            custom_details: "{vn_name}".to_string(),
            custom_state: "{chapter}".to_string(),
            ..Default::default()
        };
        let ctx = PresenceContext {
            vn_name: "Steins;Gate".to_string(),
            chapter: None,
            ..Default::default()
        };
        let act = build_activity(&settings, &ctx);
        assert_eq!(act.details.as_deref(), Some("Steins;Gate"));
        assert_eq!(
            act.state, None,
            "missing chapter must omit the state line, not show literal {{chapter}}"
        );
    }

    #[test]
    fn test_discord_sanitization() {
        assert_eq!(sanitize_discord_string(""), None);
        assert_eq!(sanitize_discord_string("   "), None);
        assert_eq!(
            sanitize_discord_string("Fate/Stay Night").as_deref(),
            Some("Fate/Stay Night")
        );

        let long_str = "a".repeat(200);
        let sanitized = sanitize_discord_string(&long_str).expect("long text is kept, capped");
        assert_eq!(sanitized.len(), 128);
    }

    #[test]
    fn a_one_character_field_is_omitted_because_discord_rejects_the_whole_activity() {
        assert_eq!(sanitize_discord_string("Z"), None);
        assert_eq!(sanitize_discord_string("  終  "), None);
        assert_eq!(sanitize_discord_string("終幕").as_deref(), Some("終幕"));

        // Custom mode, so the one-character chapter is what reaches the card.
        let settings = PresenceSettings {
            details_mode: PresenceDetailsMode::Custom,
            custom_details: "{vn_name}".to_string(),
            custom_state: "{chapter}".to_string(),
            ..Default::default()
        };
        let ctx = PresenceContext {
            vn_name: "Z".to_string(),
            chapter: Some("終".to_string()),
            ..Default::default()
        };
        let act = build_activity(&settings, &ctx);
        assert_eq!(act.details.as_deref(), Some("Visual Novel"));
        assert_eq!(
            act.state, None,
            "a one-character chapter must be omitted, not padded"
        );
    }

    #[test]
    fn japanese_text_is_capped_on_a_char_boundary() {
        // 3 bytes per character: 128 bytes would split the 43rd character.
        let title = "あ".repeat(100);
        let out = sanitize_discord_string(&title).expect("kept");
        assert!(out.len() <= 128);
        assert_eq!(out.chars().count(), 42);
        assert!(out.chars().all(|c| c == 'あ'));
    }

    #[test]
    fn settings_saved_before_a_field_existed_still_load() {
        let old = r#"{"enabled":true,"client_id":"123456789012345678"}"#;
        let loaded: PresenceSettings = serde_json::from_str(old).expect("old row decodes");
        assert!(loaded.enabled);
        assert_eq!(loaded.client_id, "123456789012345678");
        assert_eq!(loaded.custom_details, "{vn_name}");
        assert!(loaded.show_timestamps);
    }

    #[test]
    fn test_explicit_cover_is_never_sent() {
        let settings = PresenceSettings {
            show_cover: true,
            ..Default::default()
        };
        let ctx = PresenceContext {
            vn_name: "Test VN".to_string(),
            cover_url: Some("https://t.vndb.org/cv/123.jpg".to_string()),
            is_cover_explicit: true,
            rpc_image: None,
            ..Default::default()
        };

        let activity = build_activity(&settings, &ctx);
        assert_eq!(activity.large_image, None);
    }

    #[test]
    fn test_local_cover_is_never_sent() {
        let settings = PresenceSettings {
            show_cover: true,
            ..Default::default()
        };
        let ctx = PresenceContext {
            vn_name: "Test VN".to_string(),
            cover_url: Some("covers/custom/local.jpg".to_string()),
            is_cover_explicit: false,
            rpc_image: None,
            ..Default::default()
        };

        let activity = build_activity(&settings, &ctx);
        assert_eq!(activity.large_image, None);
    }

    #[test]
    fn my_rating_and_vndb_rating_are_two_different_tokens() {
        // The regression this guards: `{vndb_rating}` used to carry the user's
        // own rating first and only fell back to VNDB's, so the same tag meant
        // different games depending on whether the user had rated it.
        let ctx = PresenceContext {
            my_rating: Some("7.5".to_string()),
            ..vn_with_facts()
        };
        let tokens = build_tokens(&ctx);
        assert_eq!(tokens["my_rating"], "7.5", "the user's own rating");
        assert_eq!(tokens["vndb_rating"], "8.20", "VNDB's community rating");

        // An unrated game renders `{my_rating}` empty, not as VNDB's number.
        let unrated = PresenceContext {
            my_rating: None,
            ..vn_with_facts()
        };
        assert_eq!(build_tokens(&unrated)["my_rating"], "");
        assert_eq!(build_tokens(&unrated)["vndb_rating"], "8.20");
    }

    // ---- ported from EwE: rotation lists, transforms, buttons, fallbacks ----

    fn vn_with_facts() -> PresenceContext {
        PresenceContext {
            vn_name: "Sakura no Uta".to_string(),
            vndb_id: Some("v19389".to_string()),
            vndb_url: Some("https://vndb.org/v19389".to_string()),
            vndb_rating: Some("8.20".to_string()),
            my_rating: None,
            vndb_developer: Some("Makura".to_string()),
            vndb_status: Some("Playing".to_string()),
            vndb_length: Some("Long (30-50h)".to_string()),
            vndb_year: Some("2015".to_string()),
            vndb_loaded: true,
            ..Default::default()
        }
    }

    fn custom_settings() -> PresenceSettings {
        PresenceSettings {
            details_mode: PresenceDetailsMode::Custom,
            ..Default::default()
        }
    }

    #[test]
    fn a_custom_rotation_list_advances_one_entry_per_index() {
        // EwE's per-game "cycle items": several templates, shown in turn.
        let settings = PresenceSettings {
            custom_details: String::new(),
            custom_details_items: vec![
                "{vndb_status}".to_string(),
                "By {vndb_developer}".to_string(),
                "{vndb_year}".to_string(),
            ],
            custom_details_rotate: true,
            ..custom_settings()
        };
        let seen: Vec<String> = (0..3)
            .map(|i| {
                build_activity(
                    &settings,
                    &PresenceContext {
                        cycle_index: i,
                        ..vn_with_facts()
                    },
                )
                .details
                .expect("every entry renders")
            })
            .collect();
        // "Playing" is normalized to "Reading", as EwE does, so the status label
        // reads the way VNDB phrases the act of reading rather than playing.
        assert_eq!(seen, vec!["Reading", "By Makura", "2015"]);
    }

    #[test]
    fn a_rotation_list_entry_that_renders_empty_is_skipped_not_shown_blank() {
        // A game with no detected chapter would otherwise blank the line on every
        // other tick and look like a broken card.
        let settings = PresenceSettings {
            custom_details: String::new(),
            custom_details_items: vec!["{chapter}".to_string(), "{vndb_developer}".to_string()],
            custom_details_rotate: true,
            ..custom_settings()
        };
        let act = build_activity(
            &settings,
            &PresenceContext {
                cycle_index: 0,
                chapter: None,
                ..vn_with_facts()
            },
        );
        assert_eq!(act.details.as_deref(), Some("Makura"));
    }

    #[test]
    fn switching_rotation_off_uses_the_single_template_even_with_a_list_present() {
        // The list is what the user edited; the toggle is what decides. Keeping
        // the list lets them turn rotation back on without retyping it.
        let settings = PresenceSettings {
            custom_details: "{vn_name}".to_string(),
            custom_details_items: vec!["A".to_string(), "B".to_string()],
            custom_details_rotate: false,
            ..custom_settings()
        };
        let act = build_activity(
            &settings,
            &PresenceContext {
                cycle_index: 1,
                ..vn_with_facts()
            },
        );
        assert_eq!(act.details.as_deref(), Some("Sakura no Uta"));
    }

    #[test]
    fn the_two_rotating_fields_keep_separate_cursors() {
        // A two-entry details list and a three-entry state list must not march in
        // lockstep; a single shared counter made them repeat out of phase.
        let settings = PresenceSettings {
            custom_details: String::new(),
            custom_details_items: vec!["{vndb_developer}".to_string(), "{vndb_year}".to_string()],
            custom_details_rotate: true,
            custom_state: String::new(),
            custom_state_items: vec![
                "{vndb_status}".to_string(),
                "{vndb_length}".to_string(),
                "{vn_name}".to_string(),
            ],
            custom_state_rotate: true,
            ..custom_settings()
        };
        let act = build_activity(
            &settings,
            &PresenceContext {
                cycle_index: 1,
                state_cycle_index: 2,
                ..vn_with_facts()
            },
        );
        assert_eq!(act.details.as_deref(), Some("2015"));
        assert_eq!(act.state.as_deref(), Some("Sakura no Uta"));
    }

    #[test]
    fn a_transform_runs_remove_then_replace_then_prefix_suffix_then_trim() {
        // Order is load-bearing: prefixing first would let a replace rule rewrite
        // decoration the user added on purpose. `trim` trims the inside text, then
        // the whole line, so the prefix and suffix stay flush against the result.
        let transform = TextTransform {
            prefix: "[ ".to_string(),
            suffix: " ]".to_string(),
            remove: vec!["Makura".to_string()],
            replace: vec![ReplaceRule {
                old: "By ".to_string(),
                new: "from ".to_string(),
            }],
            trim: true,
        };
        assert_eq!(transform.apply("  By Makura  "), "[ from ]");
    }

    #[test]
    fn an_empty_transform_leaves_text_alone() {
        let out = TextTransform::default().apply("  spaced  ");
        assert_eq!(
            out, "  spaced  ",
            "trim must be opt-in or an old settings row silently eats spaces"
        );
        assert!(TextTransform::default().is_noop());
    }

    #[test]
    fn a_transform_applies_to_the_rendered_details_line() {
        let settings = PresenceSettings {
            custom_details: "{vn_name}".to_string(),
            custom_details_transform: TextTransform {
                suffix: " ".to_string(),
                remove: vec!["Sakura ".to_string()],
                trim: true,
                ..Default::default()
            },
            ..custom_settings()
        };
        let act = build_activity(&settings, &vn_with_facts());
        assert_eq!(act.details.as_deref(), Some("no Uta"));
    }

    #[test]
    fn a_transform_that_empties_the_line_falls_back_instead_of_sending_nothing() {
        let settings = PresenceSettings {
            custom_details: "{vndb_status}".to_string(),
            custom_details_transform: TextTransform {
                remove: vec!["Reading".to_string()],
                trim: true,
                ..Default::default()
            },
            ..custom_settings()
        };
        let act = build_activity(&settings, &vn_with_facts());
        assert_eq!(act.details.as_deref(), Some("Visual Novel"));
    }

    #[test]
    fn the_second_button_points_at_the_users_own_vndb_list() {
        // `show_list_button` existed but nothing read it, so the switch was inert.
        let settings = PresenceSettings {
            show_list_button: true,
            my_vndb_link: "https://vndb.org/u123456".to_string(),
            ..Default::default()
        };
        let act = build_activity(&settings, &vn_with_facts());
        let labels: Vec<&str> = act.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, vec!["View on VNDB", "My VNDB List"]);
        assert_eq!(act.buttons[1].url, "https://vndb.org/u123456");
    }

    #[test]
    fn a_list_button_without_a_link_adds_no_second_button() {
        let settings = PresenceSettings {
            show_list_button: true,
            my_vndb_link: "   ".to_string(),
            ..Default::default()
        };
        let act = build_activity(&settings, &vn_with_facts());
        assert_eq!(act.buttons.len(), 1);
        assert_eq!(act.buttons[0].label, "View on VNDB");
    }

    #[test]
    fn a_non_http_link_never_becomes_a_button() {
        // The link is a free-text settings field; a `file:` or `javascript:` URL
        // must not reach the Discord client.
        for bad in [
            "javascript:alert(1)",
            "file:///C:/secrets.txt",
            "https://vndb.org/u1\nrm -rf /",
        ] {
            let settings = PresenceSettings {
                show_list_button: true,
                my_vndb_link: bad.to_string(),
                ..Default::default()
            };
            let act = build_activity(&settings, &vn_with_facts());
            assert_eq!(act.buttons.len(), 1, "{bad:?} must be rejected");
        }
    }

    #[test]
    fn discord_renders_at_most_two_buttons() {
        let settings = PresenceSettings {
            show_list_button: true,
            my_vndb_link: "https://vndb.org/u1".to_string(),
            ..Default::default()
        };
        let act = build_activity(&settings, &vn_with_facts());
        assert_eq!(act.buttons.len(), MAX_BUTTONS);
    }

    #[test]
    fn the_vndb_state_template_takes_over_from_the_builtin_state_line() {
        let settings = PresenceSettings {
            vndb_state_template: "★ {vndb_rating}/10 · {vndb_year} · {vndb_length}".to_string(),
            ..Default::default()
        };
        let act = build_activity(&settings, &vn_with_facts());
        assert_eq!(
            act.state.as_deref(),
            Some("★ 8.20/10 · 2015 · Long (30-50h)")
        );
    }

    #[test]
    fn an_empty_vndb_state_template_keeps_the_route_and_chapter_line() {
        let ctx = PresenceContext {
            route: Some("Main".to_string()),
            chapter: Some("Chapter 3".to_string()),
            ..vn_with_facts()
        };
        let act = build_activity(&PresenceSettings::default(), &ctx);
        assert_eq!(act.state.as_deref(), Some("Main - Chapter 3"));
    }

    #[test]
    fn swapping_details_and_state_reverses_the_two_lines() {
        let plain = PresenceSettings {
            vndb_state_template: "State {vndb_year}".to_string(),
            ..Default::default()
        };
        let swapped = PresenceSettings {
            vndb_swap_details_state: true,
            ..plain.clone()
        };
        let a = build_activity(&plain, &vn_with_facts());
        let b = build_activity(&swapped, &vn_with_facts());
        assert_eq!(a.details, b.state);
        assert_eq!(a.state, b.details);
    }

    #[test]
    fn a_vndb_template_list_rotates_instead_of_the_builtin_facts() {
        let settings = PresenceSettings {
            vndb_details_items: vec![
                "{vndb_status}".to_string(),
                "By {vndb_developer}".to_string(),
            ],
            ..Default::default()
        };
        let first = build_activity(
            &settings,
            &PresenceContext {
                cycle_index: 0,
                ..vn_with_facts()
            },
        );
        let second = build_activity(
            &settings,
            &PresenceContext {
                cycle_index: 1,
                ..vn_with_facts()
            },
        );
        assert_eq!(first.details.as_deref(), Some("Reading"));
        assert_eq!(second.details.as_deref(), Some("By Makura"));
    }

    #[test]
    fn an_unlinked_game_shows_the_reading_placeholders_not_a_generic_word() {
        let ctx = PresenceContext {
            vn_name: "Unknown VN".to_string(),
            ..Default::default()
        };
        let act = build_activity(&PresenceSettings::default(), &ctx);
        assert_eq!(act.details.as_deref(), Some("Reading"));
        assert_eq!(act.state.as_deref(), Some("Loading VNDB data..."));
    }

    #[test]
    fn the_placeholder_texts_are_configurable_and_blank_omits_the_state() {
        let settings = PresenceSettings {
            vndb_loading_details: "Indexing".to_string(),
            vndb_loading_state: String::new(),
            ..Default::default()
        };
        let act = build_activity(&settings, &PresenceContext::default());
        assert_eq!(act.details.as_deref(), Some("Indexing"));
        assert_eq!(act.state, None);
    }

    #[test]
    fn the_card_stays_in_the_background_by_default() {
        // A reader alt-tabs constantly; the default has to match that or every
        // existing user suddenly gets a card that blinks out.
        assert!(PresenceSettings::default().show_when_unfocused);
    }

    #[test]
    fn a_settings_row_saved_before_the_background_switch_existed_keeps_the_card() {
        // `serde(default)` fills the field with the *struct* default, so an older
        // row gets the same behaviour as a fresh install rather than the opposite.
        let old = r#"{"enabled":true,"client_id":"123456789012345678"}"#;
        let loaded: PresenceSettings = serde_json::from_str(old).expect("old row decodes");
        assert!(loaded.show_when_unfocused);
    }

    #[test]
    fn the_game_state_token_distinguishes_reading_from_tabbed_away() {
        let settings = PresenceSettings {
            custom_details: "{vn_name} - {game_state}".to_string(),
            ..custom_settings()
        };
        let focused = build_activity(
            &settings,
            &PresenceContext {
                is_unfocused: false,
                ..vn_with_facts()
            },
        );
        assert_eq!(focused.details.as_deref(), Some("Sakura no Uta - Reading"));

        let unfocused = build_activity(
            &settings,
            &PresenceContext {
                is_unfocused: true,
                ..vn_with_facts()
            },
        );
        assert_eq!(unfocused.details.as_deref(), Some("Sakura no Uta - Idle"));
    }

    #[test]
    fn a_playing_status_is_shown_as_reading() {
        let settings = PresenceSettings {
            custom_details: "{vndb_status}".to_string(),
            ..custom_settings()
        };
        let mut ctx = vn_with_facts();
        ctx.vndb_status = Some("Playing".to_string());
        assert_eq!(
            build_activity(&settings, &ctx).details.as_deref(),
            Some("Reading")
        );
        ctx.vndb_status = Some("Finished".to_string());
        assert_eq!(
            build_activity(&settings, &ctx).details.as_deref(),
            Some("Finished")
        );
        ctx.vndb_status = None;
        assert_eq!(
            build_activity(&settings, &ctx).details.as_deref(),
            Some("Reading"),
            "an unlinked list still reads as reading"
        );
    }

    #[test]
    fn test_safe_public_cover_is_sent() {
        let settings = PresenceSettings {
            show_cover: true,
            ..Default::default()
        };
        let ctx = PresenceContext {
            vn_name: "Test VN".to_string(),
            cover_url: Some("https://t.vndb.org/cv/safe.jpg".to_string()),
            is_cover_explicit: false,
            rpc_image: None,
            ..Default::default()
        };

        let activity = build_activity(&settings, &ctx);
        assert_eq!(
            activity.large_image,
            Some("https://t.vndb.org/cv/safe.jpg".to_string())
        );
    }

    #[test]
    fn an_uploaded_blurred_image_shows_even_where_the_cover_was_explicit() {
        let settings = PresenceSettings {
            show_cover: true,
            ..Default::default()
        };
        // Explicit art itself must never leave, but its blurred upload already
        // went through the concealment policy, so it is the art that shows.
        let ctx = PresenceContext {
            vn_name: "Test VN".to_string(),
            cover_url: Some("https://t.vndb.org/cv/explicit.jpg".to_string()),
            is_cover_explicit: true,
            rpc_image: Some("https://catbox.moe/blurred.jpg".to_string()),
            ..Default::default()
        };
        assert_eq!(
            build_activity(&settings, &ctx).large_image,
            Some("https://catbox.moe/blurred.jpg".to_string())
        );
    }

    #[test]
    fn a_non_public_rpc_image_is_dropped_like_any_other_local_path() {
        let settings = PresenceSettings {
            show_cover: true,
            ..Default::default()
        };
        let ctx = PresenceContext {
            vn_name: "Test VN".to_string(),
            is_cover_explicit: true,
            rpc_image: Some("file:///C:/Users/me/art.jpg".to_string()),
            ..Default::default()
        };
        assert_eq!(build_activity(&settings, &ctx).large_image, None);
    }

    #[test]
    fn test_debouncer_throttling_and_suppression() {
        let mut debouncer = PresenceDebouncer::new(5, 30);
        let act1 = PresenceActivity {
            name: Some("VN 1".to_string()),
            details: Some("VN 1".to_string()),
            state: None,
            large_image: None,
            large_text: None,
            small_image: None,
            small_text: None,
            start_timestamp: Some(1000),
            buttons: vec![],
        };

        // First call should send
        assert!(debouncer.should_send(&act1, 10));

        // Same call immediately after should NOT send
        assert!(!debouncer.should_send(&act1, 12));

        // Different call before min_interval (5s) should NOT send (throttled)
        let mut act2 = act1.clone();
        act2.details = Some("VN 2".to_string());
        assert!(!debouncer.should_send(&act2, 13));

        // Different call after min_interval (10 + 5 = 15) should send
        assert!(debouncer.should_send(&act2, 16));

        // Same call at 30s later (16 + 30 = 46) should re-send (max suppress reached)
        assert!(debouncer.should_send(&act2, 47));
    }

    fn activity_started_at(start: i64) -> PresenceActivity {
        PresenceActivity {
            name: Some("VN".to_string()),
            details: Some("VN".to_string()),
            state: Some("Chapter 1".to_string()),
            large_image: None,
            large_text: None,
            small_image: None,
            small_text: None,
            start_timestamp: Some(start),
            buttons: vec![],
        }
    }

    #[test]
    fn a_wobbling_start_timestamp_is_not_a_new_activity() {
        // start = now - active_time truncated to seconds, so it flips by one between
        // polls. That must not look like a change, or every poll would resend.
        let mut debouncer = PresenceDebouncer::default();
        assert!(debouncer.should_send(&activity_started_at(1_000), 100));
        assert!(!debouncer.should_send(&activity_started_at(1_001), 102));
        assert!(!debouncer.should_send(&activity_started_at(999), 104));
        assert!(!debouncer.should_send(&activity_started_at(1_001), 106));
    }

    #[test]
    fn a_real_start_change_is_sent_once_the_interval_allows() {
        let mut debouncer = PresenceDebouncer::default();
        assert!(debouncer.should_send(&activity_started_at(1_000), 100));
        // A genuinely new session, but inside the 15 s minimum interval.
        assert!(!debouncer.should_send(&activity_started_at(5_000), 110));
        assert!(debouncer.should_send(&activity_started_at(5_000), 115));
    }

    #[test]
    fn the_default_interval_follows_discords_one_update_per_fifteen_seconds() {
        let mut debouncer = PresenceDebouncer::default();
        let mut next = activity_started_at(1_000);
        assert!(debouncer.should_send(&next, 100));
        next.state = Some("Chapter 2".to_string());
        assert!(!debouncer.should_send(&next, 114));
        assert!(debouncer.should_send(&next, 115));
    }

    #[test]
    fn a_failed_send_is_retried_at_the_interval_not_after_the_suppression_window() {
        let mut debouncer = PresenceDebouncer::default();
        let act = activity_started_at(1_000);
        assert!(debouncer.should_send(&act, 100));
        // Discord was not running, so the caller reports the send as lost.
        debouncer.reset();
        assert!(
            !debouncer.should_send(&act, 105),
            "still inside the interval"
        );
        assert!(debouncer.should_send(&act, 115), "retried well before 30 s");
    }

    #[test]
    fn stopping_and_restarting_a_game_cannot_bypass_the_interval() {
        let mut debouncer = PresenceDebouncer::default();
        let act = activity_started_at(1_000);
        assert!(debouncer.should_send(&act, 100));
        debouncer.reset(); // alt-tab away: presence cleared
        assert!(
            !debouncer.should_send(&act, 102),
            "alt-tab back, two seconds later"
        );
        assert!(debouncer.should_send(&act, 115));
    }
}
