//! Disables `WebView2`'s built-in browser context menu via `ICoreWebView2Settings`.
//! Tauri v2 does not forward `wry::with_default_context_menus(false)`; this only
//! configures the app's own webview at startup without touching external processes.

#![cfg(windows)]
// Opt in to `unsafe` solely for the single documented WebView2 COM call below.
#![allow(unsafe_code)]

/// Hides the built-in browser context menu for `"main"` (non-fatal on failure).
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
