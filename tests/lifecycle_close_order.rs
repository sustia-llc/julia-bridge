//! Run with: cargo nextest run --features lifecycle
//!
//! This file holds one test function, which shuts a `Strategy::Close` lifecycle down.
#![cfg(feature = "lifecycle")]

mod common;

use std::{
    future::{Future, poll_fn},
    pin::pin,
    task::Poll,
    time::Duration,
};

use common::{SLOW_FN, closed_at_end, define_slow, shared_handle_closed, within};
use julia_bridge::lifecycle::{Lifecycle, Strategy};

const SLOW_SECS: f64 = 0.5;

#[tokio::test(flavor = "multi_thread")]
async fn close_keeps_the_handle_open_until_the_tracked_call_ends() {
    define_slow().await;
    let lifecycle = Lifecycle::new(Strategy::Close);

    let mut call = pin!(lifecycle.call_base1(SLOW_FN, SLOW_SECS));
    let first = poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx))).await;
    assert!(first.is_pending(), "first poll of slow(0.5): got {first:?}");

    let report = within("shutdown", lifecycle.shutdown(Duration::from_secs(30)))
        .await
        .unwrap();
    let closed_after = shared_handle_closed();
    let returned = tokio::time::timeout(Duration::from_secs(30), call).await;

    assert!(report.drained, "drained: got false, expected true");
    assert!(
        matches!(returned, Ok(Ok(x)) if x == SLOW_SECS),
        "tracked call: got {returned:?}, expected Ok(Ok(0.5))"
    );
    assert_eq!(
        closed_at_end(),
        [false],
        "handle closed at the end of each slow call, during the shutdown"
    );
    assert_eq!(closed_after, Some(true), "handle closed after shutdown");
    assert!(
        report.close.is_ok(),
        "close step: got {:?}, expected Ok",
        report.close
    );
}
