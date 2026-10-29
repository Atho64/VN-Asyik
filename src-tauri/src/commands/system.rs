//! System commands: platform capability reporting.

use crate::platform;

/// Platform capability matrix for the "what works here" screen.
#[tauri::command]
pub fn platform_info() -> platform::PlatformInfo {
    platform::info()
}

/// Schemes an "open in browser" request may use.
///
/// An explicit allowlist, not a blocklist, because the URL arrives from an API
/// response and from a cached database row: anything else (a `javascript:` URL, a
/// `file:` path, a deep link into a game launcher) must never be handed to the OS
/// shell. `mailto` is included because a developer's contact address is a real link
/// on a VN page.
const EXTERNAL_SCHEMES: [&str; 3] = ["https", "http", "mailto"];

/// Whether a URL is safe to open outside the app.
///
/// Pure and public so the rule can be unit-tested without spawning a process, and so
/// the frontend can apply the same check before offering a link as clickable.
#[must_use]
pub fn external_url_allowed(url: &str) -> bool {
    let trimmed = url.trim_matches(|ch: char| ch.is_whitespace() || ch.is_control());
    if trimmed.is_empty() || trimmed.len() > 2048 {
        return false;
    }
    // A newline splits a URL for Windows' shell and for some browsers, so
    // `https://ok\njavascript:…` would pass a prefix check and then hand the second
    // half to the OS. Any interior whitespace or control character is refused
    // outright rather than escaped: no legitimate external URL needs one.
    if trimmed
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return false;
    }
    let Some((scheme, rest)) = trimmed.split_once(':') else {
        return false;
    };
    let scheme = scheme.to_ascii_lowercase();
    if !EXTERNAL_SCHEMES.contains(&scheme.as_str()) {
        return false;
    }
    // `mailto:` is followed by an address, the others by `//authority`.
    if scheme == "mailto" {
        return !rest.is_empty();
    }
    rest.starts_with("//") && rest.len() > 2
}

/// Open a URL in the user's default browser.
///
/// The app never renders third-party pages inside its own `WebView`: a link from a
/// VN's VNDB page belongs in the browser the user already trusts, with their
/// extensions and their address bar.
///
/// # Errors
/// `invalid_url` when the scheme is not on the allowlist, and `open_failed` when the
/// OS refuses.
// Tauri's IPC layer deserialises command arguments into owned values, so both the
// handle and the URL arrive by value regardless of what this body needs. Taking the
// URL as `String` is also the contract the frontend sends.
#[allow(
    clippy::needless_pass_by_value,
    reason = "tauri::command requires owned argument types for deserialisation"
)]
#[tauri::command]
pub fn open_external(
    app: tauri::AppHandle,
    url: String,
) -> Result<(), crate::commands::CommandError> {
    use tauri_plugin_opener::OpenerExt as _;

    if !external_url_allowed(&url) {
        return Err(crate::commands::CommandError::new(
            "invalid_url",
            "that link cannot be opened",
        ));
    }
    app.opener()
        .open_url(url.trim(), None::<&str>)
        .map_err(|err| {
            crate::commands::CommandError::new(
                "open_failed",
                format!("could not open the link: {err}"),
            )
        })
}

/// Hide the main application window (desktop only).
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn hide_window(window: tauri::WebviewWindow) -> Result<(), crate::commands::CommandError> {
    window.hide().map_err(|err| {
        crate::commands::CommandError::new("hide_failed", format!("could not hide window: {err}"))
    })
}

/// Minimize the main application window to the taskbar.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn minimize_window(window: tauri::WebviewWindow) -> Result<(), crate::commands::CommandError> {
    window.minimize().map_err(|err| {
        crate::commands::CommandError::new(
            "minimize_failed",
            format!("could not minimize window: {err}"),
        )
    })
}

/// Show and focus the main application window.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn show_window(window: tauri::WebviewWindow) -> Result<(), crate::commands::CommandError> {
    let _ = window.show();
    let _ = window.unminimize();
    window.set_focus().map_err(|err| {
        crate::commands::CommandError::new("focus_failed", format!("could not focus window: {err}"))
    })
}

/// Toggle window visibility between hidden and shown.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn toggle_window(window: tauri::WebviewWindow) -> Result<bool, crate::commands::CommandError> {
    let is_visible = window.is_visible().unwrap_or(true);
    let is_minimized = window.is_minimized().unwrap_or(false);
    if is_visible && !is_minimized {
        window.hide().map_err(|err| {
            crate::commands::CommandError::new(
                "hide_failed",
                format!("could not hide window: {err}"),
            )
        })?;
        Ok(false)
    } else {
        let _ = window.show();
        let _ = window.unminimize();
        window.set_focus().map_err(|err| {
            crate::commands::CommandError::new(
                "focus_failed",
                format!("could not focus window: {err}"),
            )
        })?;
        Ok(true)
    }
}

/// Cleanly exit the application.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::external_url_allowed;

    #[test]
    fn https_and_http_are_allowed() {
        assert!(external_url_allowed("https://vndb.org/v17"));
        assert!(external_url_allowed("http://example.invalid/x"));
        assert!(external_url_allowed("  https://vndb.org/v17  "));
        assert!(external_url_allowed("HTTPS://vndb.org/v17"));
        assert!(external_url_allowed("https://t.vndb.org/sf/90/190.jpg"));
        assert!(external_url_allowed("mailto:studio@example.invalid"));
    }

    #[test]
    fn scripts_and_local_paths_are_refused() {
        // This is the whole point of the allowlist: these arrive from a response body
        // or a cached row, so they must not reach the OS.
        assert!(!external_url_allowed("javascript:alert(1)"));
        assert!(!external_url_allowed("JavaScript:alert(1)"));
        assert!(!external_url_allowed(
            "file:///C:/Windows/System32/calc.exe"
        ));
        assert!(!external_url_allowed("data:text/html,<script>x</script>"));
        assert!(!external_url_allowed("vbscript:msgbox(1)"));
        assert!(!external_url_allowed("steam://run/12345"));
        assert!(!external_url_allowed("ms-settings:privacy"));
    }

    #[test]
    fn garbage_is_refused_rather_than_guessed_at() {
        assert!(!external_url_allowed(""));
        assert!(!external_url_allowed("   "));
        assert!(!external_url_allowed("vndb.org/v17"));
        assert!(!external_url_allowed("https://"));
        assert!(!external_url_allowed("mailto:"));
        assert!(!external_url_allowed(&format!(
            "https://{}",
            "a".repeat(2100)
        )));
    }

    #[test]
    fn a_control_character_cannot_smuggle_a_second_url() {
        // Windows' shell and some browsers split on the first newline, so a payload
        // like "https://ok\njavascript:..." must not pass because it starts correctly.
        assert!(!external_url_allowed(
            "https://vndb.org/v17\njavascript:alert(1)"
        ));
    }
}
