//! Turning off `WebView2`'s built-in browser context menu.
//!
//! Right-clicking anywhere in the window offered Back, Refresh, Save as, Print,
//! More tools and Inspect. That is Chromium's own menu leaking into a desktop
//! application: none of it belongs here, and "Inspect" in particular is a
//! developer tool the user should not be handed by accident.
//!
//! Tauri v2 exposes no setting for this. `wry` has
//! `with_default_context_menus(false)`, but Tauri never forwards it, so the
//! only supported route is `PlatformWebview::controller()` and the raw
//! `ICoreWebView2Settings` COM interface. The app then draws its own menu.
//!
//! Nothing here injects into another process or touches another application's
//! memory: it is this app's own webview, configured at startup.

#![cfg(windows)]
// The workspace denies `unsafe` everywhere. Configuring WebView2 means calling a
// COM interface, so this file opts in like the probe does: one documented call,
// with its SAFETY note. Nothing here reads or writes another process.
#![allow(unsafe_code)]

/// Hide the built-in context menu for `window`.
///
/// Failure is reported rather than fatal: the app is still usable with the
/// browser menu present, and refusing to start over a right-click menu would be
/// a worse outcome than leaving it.
pub fn disable_default_context_menu(app: &tauri::AppHandle) {
    use tauri::Manager;

    let Some(window) = app.get_webview_window("main") else {
        // The label is set in tauri.conf.json; if it ever changes this becomes a
        // silent no-op, so say so instead of pretending it worked.
        eprintln!("context menu: no window labelled \"main\"");
        return;
    };

    let result = window.with_webview(|webview| {
        let controller = webview.controller();
        // SAFETY: COM interfaces from WebView2 are apartment-bound. This closure
        // runs on the main thread from Tauri's own callback, which is where the
        // controller was created, so the call is on the right thread.
        unsafe {
            // `Settings()` lives on ICoreWebView2, which the controller exposes.
            match controller.CoreWebView2() {
                Ok(core) => match core.Settings() {
                    Ok(settings) => {
                        let _ = settings.SetAreDefaultContextMenusEnabled(false);
                    }
                    Err(error) => eprintln!("context menu: no settings: {error}"),
                },
                Err(error) => eprintln!("context menu: no core handle: {error}"),
            }
        }
    });

    if let Err(error) = result {
        eprintln!("context menu: could not reach the webview: {error}");
    }
}
