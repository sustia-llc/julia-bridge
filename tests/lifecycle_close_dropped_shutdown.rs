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

use common::{SLOW_FN, define_slow, shared_handle_closed};
use julia_bridge::{
    lifecycle::{Lifecycle, Strategy},
    shared,
};

const SLOW_SECS: f64 = 0.5;

#[tokio::test(flavor = "multi_thread")]
async fn close_runs_after_the_shutdown_future_is_dropped() {
    define_slow().await;
    let lifecycle = Lifecycle::new(Strategy::Close);

    let mut call = pin!(lifecycle.call_base1(SLOW_FN, SLOW_SECS));
    let first = poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx))).await;
    assert!(first.is_pending(), "first poll of slow(0.5): got {first:?}");

    {
        let mut shutdown = pin!(lifecycle.shutdown(Duration::from_secs(30)));
        let first = poll_fn(|cx| Poll::Ready(shutdown.as_mut().poll(cx))).await;
        assert!(first.is_pending(), "first poll of shutdown: got {first:?}");
    }

    let running = tokio::time::timeout(Duration::from_secs(30), call).await;
    assert!(
        matches!(running, Ok(Ok(x)) if x == SLOW_SECS),
        "tracked call: got {running:?}, expected Ok(Ok(0.5))"
    );

    let refused = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Err(error) = shared::call_base1("sqrt", 4.0).await {
                break error;
            }
        }
    })
    .await;
    assert!(
        refused.is_ok(),
        "direct calls after the dropped shutdown: none returned Err within 30 s, expected the handle to close"
    );
    assert_eq!(
        shared_handle_closed(),
        Some(true),
        "handle closed after the dropped shutdown"
    );
}
