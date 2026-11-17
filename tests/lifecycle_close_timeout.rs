//! Run with: cargo nextest run --features lifecycle
//!
//! This file holds one test function, which shuts a `Strategy::Close` lifecycle down.
#![cfg(feature = "lifecycle")]

mod common;

use std::{
    future::{Future, poll_fn},
    pin::pin,
    task::Poll,
    time::{Duration, Instant},
};

use common::{SLOW_FN, define_slow, end_instants, ends, shared_handle_closed, within};
use julia_bridge::lifecycle::{Lifecycle, Strategy};

const SLOW_SECS: f64 = 0.5;

#[tokio::test(flavor = "multi_thread")]
async fn close_after_a_timed_out_drain_returns_after_the_running_call_ended() {
    define_slow().await;
    let lifecycle = Lifecycle::new(Strategy::Close);

    let mut call = pin!(lifecycle.call_base1(SLOW_FN, SLOW_SECS));
    let first = poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx))).await;
    assert!(first.is_pending(), "first poll of slow(0.5): got {first:?}");
    assert_eq!(
        shared_handle_closed(),
        Some(false),
        "handle closed before shutdown"
    );

    let mut shutdown = pin!(lifecycle.shutdown(Duration::from_millis(50)));
    let first = poll_fn(|cx| Poll::Ready(shutdown.as_mut().poll(cx))).await;
    let entered_by = Instant::now();
    let report = match first {
        Poll::Ready(out) => out,
        Poll::Pending => within("shutdown", shutdown).await,
    }
    .unwrap();
    let ends_at_return = ends();
    let end_instants_at_return = end_instants();
    let closed_after = shared_handle_closed();

    assert!(!report.drained, "drained: got true, expected false");
    assert_eq!(report.remaining, 1, "remaining");
    assert_eq!(closed_after, Some(true), "handle closed after shutdown");
    assert!(
        report.close.is_ok(),
        "close step: got {:?}, expected Ok",
        report.close
    );
    assert_eq!(
        ends_at_return,
        [(true, 1)],
        "(handle closed, slow calls started) at the end of each slow call that had ended when shutdown returned"
    );
    let ended_at = *end_instants_at_return
        .first()
        .expect("no end instant of the slow call when shutdown returned");
    let floor = ended_at.saturating_duration_since(entered_by);
    assert!(
        report.elapsed >= floor,
        "elapsed: got {:?}, expected at least {floor:?}, the time from the return of the first poll of shutdown to the end of the slow call",
        report.elapsed
    );

    let running = tokio::time::timeout(Duration::from_secs(30), call).await;
    eprintln!(
        "close timeout: running call resolved to {running:?}; elapsed {:?}; floor {floor:?}",
        report.elapsed
    );
    assert!(
        matches!(running, Ok(Ok(x)) if x == SLOW_SECS),
        "running call: got {running:?}, expected Ok(Ok(0.5))"
    );
}
