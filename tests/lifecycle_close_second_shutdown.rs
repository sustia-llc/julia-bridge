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

use common::{SLOW_FN, UNTIL_CLOSED_FN, define_slow, define_waits};
use julia_bridge::{
    lifecycle::{Lifecycle, Strategy},
    shared,
};

const SLOW_SECS: f64 = 1.0;
/// The time after which the waiting Julia function gives up, in seconds.
const CAP_SECS: f64 = 10.0;

#[tokio::test(flavor = "multi_thread")]
async fn second_shutdown_counts_calls_only() {
    define_slow().await;
    define_waits().await;
    let lifecycle = Lifecycle::new(Strategy::Close);

    let mut call = pin!(lifecycle.call_base1(SLOW_FN, SLOW_SECS));
    let first = poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx))).await;
    assert!(first.is_pending(), "first poll of slow(1.0): got {first:?}");
    let mut closed = pin!(shared::call_base1(UNTIL_CLOSED_FN, CAP_SECS));
    let first = poll_fn(|cx| Poll::Ready(closed.as_mut().poll(cx))).await;
    assert!(
        first.is_pending(),
        "first poll of until_closed: got {first:?}"
    );

    {
        let mut shutdown = pin!(lifecycle.shutdown(Duration::from_millis(50)));
        let first = poll_fn(|cx| Poll::Ready(shutdown.as_mut().poll(cx))).await;
        assert!(first.is_pending(), "first poll of shutdown: got {first:?}");
    }
    let closed = tokio::time::timeout(Duration::from_secs(30), closed).await;
    assert!(
        matches!(closed, Ok(Ok(x)) if x == 1.0),
        "call that waits for the close step of the first shutdown: got {closed:?}, expected Ok(Ok(1.0))"
    );

    let second = tokio::time::timeout(
        Duration::from_secs(30),
        lifecycle.shutdown(Duration::from_millis(50)),
    )
    .await
    .expect("second shutdown did not return within 30 s")
    .unwrap();
    let running = tokio::time::timeout(Duration::from_secs(30), call).await;

    assert!(!second.drained, "drained: got true, expected false");
    assert_eq!(second.remaining, 1, "remaining of the second shutdown");
    assert!(
        second.close.is_ok(),
        "close step: got {:?}, expected Ok",
        second.close
    );
    assert!(
        matches!(running, Ok(Ok(x)) if x == SLOW_SECS),
        "tracked call: got {running:?}, expected Ok(Ok(1.0))"
    );
}
