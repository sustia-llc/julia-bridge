//! Run with: cargo nextest run --features lifecycle
//!
//! This file holds one test function, which shuts a `Strategy::Close` lifecycle down.
#![cfg(feature = "lifecycle")]

mod common;

use std::{sync::Arc, time::Duration};

use common::{
    SLOW_FN, admitted_then, define_slow, ends, shared_handle_closed, started_count, wait_started,
};
use julia_bridge::{
    lifecycle::{Lifecycle, Strategy},
    shared,
};
use tokio::sync::mpsc;

const CALLS: usize = 40;
const SLOW_SECS: f64 = 0.5;
const RUNNING_AT_ONCE: usize = shared::MAX_CONCURRENT_TASKS;

#[tokio::test(flavor = "multi_thread")]
async fn close_after_a_timed_out_drain_returns_every_queued_call() {
    define_slow().await;
    let lifecycle = Arc::new(Lifecycle::new(Strategy::Close));

    let (admitted_tx, mut admitted_rx) = mpsc::unbounded_channel();
    let (result_tx, mut result_rx) = mpsc::unbounded_channel();
    for _ in 0..CALLS {
        let lifecycle = Arc::clone(&lifecycle);
        let admitted_tx = admitted_tx.clone();
        let result_tx = result_tx.clone();
        tokio::spawn(async move {
            let call = lifecycle.call_base1(SLOW_FN, SLOW_SECS);
            let result = admitted_then(call, admitted_tx).await;
            result_tx.send(result).unwrap();
        });
    }
    drop(admitted_tx);
    drop(result_tx);
    for _ in 0..CALLS {
        admitted_rx.recv().await.unwrap();
    }
    let saturated =
        tokio::time::timeout(Duration::from_secs(30), wait_started(RUNNING_AT_ONCE)).await;
    assert!(
        saturated.is_ok(),
        "slow calls started within 30 s: got {}, expected at least {RUNNING_AT_ONCE}",
        started_count()
    );

    let report = tokio::time::timeout(
        Duration::from_secs(30),
        lifecycle.shutdown(Duration::from_millis(50)),
    )
    .await
    .expect("shutdown did not return within 30 s")
    .unwrap();
    let ends_at_return = ends();
    let closed_after = shared_handle_closed();

    assert!(!report.drained, "drained: got true, expected false");
    assert_eq!(report.remaining, CALLS, "remaining");
    assert_eq!(closed_after, Some(true), "handle closed after shutdown");
    assert!(
        report.close.is_ok(),
        "close step: got {:?}, expected Ok",
        report.close
    );
    assert_eq!(
        ends_at_return.first(),
        Some(&(true, RUNNING_AT_ONCE)),
        "(handle closed, slow calls started) at the end of the first slow call, of {CALLS} dispatched"
    );
    assert_eq!(
        ends_at_return.len(),
        CALLS,
        "slow calls that had ended when shutdown returned"
    );

    let mut results = Vec::new();
    while results.len() < CALLS {
        let next = tokio::time::timeout(Duration::from_secs(10), result_rx.recv()).await;
        let Ok(Some(result)) = next else {
            break;
        };
        results.push(result);
    }
    let ok_exact = results
        .iter()
        .filter(|r| matches!(r, Ok(x) if *x == SLOW_SECS))
        .count();
    eprintln!(
        "close queued: ok={ok_exact} returned={} remaining_at_shutdown={} first_end={:?} ended_at_return={} elapsed={:?}",
        results.len(),
        report.remaining,
        ends_at_return.first(),
        ends_at_return.len(),
        report.elapsed
    );
    assert_eq!(ok_exact, CALLS, "calls that returned Ok(0.5): {results:?}");
}
