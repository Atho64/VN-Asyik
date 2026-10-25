//! Live check that the window-title chapter path works against a real game.
//!
//! Ignored by default: it needs the game running and focused.
//!
//! ```text
//! SIGLUS_GAME="Summer Pockets REFLECTION BLUE" cargo test --test title_live -- --ignored --nocapture
//! ```

use vn_core::window_title::{TitleChapter, parse_siglus_title};
use vn_tracker_lib::platform::probe;

#[test]
#[ignore = "needs a running, focused game window"]
fn the_focused_window_title_yields_a_chapter() {
    let game = std::env::var("SIGLUS_GAME").expect("set SIGLUS_GAME");

    let probe = probe::probe().expect("a probe exists on this platform");
    let window = probe
        .focused_window()
        .expect("the focused window is readable")
        .expect("something has focus");

    println!("focused pid:   {}", window.pid);
    println!("focused title: {:?}", window.title);

    let parsed = parse_siglus_title(&window.title, &game);
    println!("parsed:        {parsed:?}");

    assert!(
        !matches!(parsed, TitleChapter::Foreign),
        "the focused window is not the game; focus it and retry"
    );
}
