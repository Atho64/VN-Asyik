//! Regression: editing an ending must keep its route link.
//!
//! Marking an ending achieved once rebuilt the row through `Ending::new` and
//! dropped `route_id`, moving the ending into "no route". The route link belongs
//! to `set_ending_route` and route merges, so the name/achieved editor preserves
//! it. These tests pin that rule at the pool-based command seam.

use vn_core::model::{Ending, Game, GameStatus, Route};
use vn_core::{DeviceId, Timestamp};
use vn_tracker_lib::commands::library;
use vn_tracker_lib::db::{Db, queries};

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millis(ms)
}

async fn linked_ending() -> (Db, DeviceId, Game, Route, Ending) {
    let db = Db::open_memory().await.unwrap();
    let device = DeviceId::parse("ending-device").unwrap();
    let game = Game::new(&device, at(1), "A long story", GameStatus::Playing).unwrap();
    queries::upsert_game(db.pool(), &game).await.unwrap();

    let route = Route::new(&device, at(2), game.meta.id, "Mio").unwrap();
    queries::upsert_route(db.pool(), &route).await.unwrap();

    // The create flow links by route_id (set_ending_route), not the free-text route.
    let mut ending = Ending::new(&device, at(3), game.meta.id, "True End").unwrap();
    ending.route_id = Some(route.meta.id);
    queries::upsert_ending(db.pool(), &ending).await.unwrap();
    (db, device, game, route, ending)
}

#[tokio::test]
async fn marking_an_ending_achieved_keeps_its_route_link() {
    let (db, device, _game, route, ending) = linked_ending().await;

    // The mark-achieved editor sends the free-text route (null here), not route_id.
    let updated = library::edit_ending(
        db.pool(),
        &device,
        ending.meta.id.to_string(),
        "True End".to_owned(),
        None,
        true,
    )
    .await
    .unwrap();

    assert!(updated.achieved);
    assert!(updated.achieved_at.is_some());
    assert_eq!(
        updated.route_id,
        Some(route.meta.id),
        "the route link must survive marking achieved",
    );

    // Persisted, not just returned.
    let stored = queries::ending_by_id(db.pool(), ending.meta.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.route_id, Some(route.meta.id));
    assert!(stored.achieved);
}

#[tokio::test]
async fn unmarking_and_renaming_also_keep_the_route_link() {
    let (db, device, _game, route, ending) = linked_ending().await;

    library::edit_ending(
        db.pool(),
        &device,
        ending.meta.id.to_string(),
        "True End".to_owned(),
        None,
        true,
    )
    .await
    .unwrap();

    // Rename and unmark: still the same route, and the date clears with achieved.
    let renamed = library::edit_ending(
        db.pool(),
        &device,
        ending.meta.id.to_string(),
        "True End (revised)".to_owned(),
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(renamed.route_id, Some(route.meta.id));
    assert_eq!(renamed.name, "True End (revised)");
    assert!(!renamed.achieved);
    assert!(renamed.achieved_at.is_none());

    // Re-marking keeps the link and records a date again.
    let remarked = library::edit_ending(
        db.pool(),
        &device,
        ending.meta.id.to_string(),
        "True End (revised)".to_owned(),
        None,
        true,
    )
    .await
    .unwrap();
    assert_eq!(remarked.route_id, Some(route.meta.id));
    assert!(remarked.achieved_at.is_some());
}
