//! Run with: cargo nextest run --features lifecycle
//!
//! This file holds one test function, which shuts a `Strategy::Close` lifecycle down.
#![cfg(feature = "lifecycle")]

mod common;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use common::{K, SLOW_FN, SLOW_SECS, admitted_then, define_slow, shared_handle_closed, within};
use jlrs::error::{JlrsError, RuntimeError};
use julia_bridge::{
    lifecycle::{Lifecycle, LifecycleError, Strategy},
    shared,
};
use tokio::sync::mpsc;

#[tokio::test(flavor = "multi_thread")]
async fn close_drains_every_admitted_call_then_closes_the_handle() {
    define_slow().await;
    let lifecycle = Arc::new(Lifecycle::new(Strategy::Close));
    let created_before = Lifecycle::new(Strategy::Drain);
    let drain_timeout = Duration::from_secs(30);

    let (admitted_tx, mut admitted_rx) = mpsc::unbounded_channel();
    let (result_tx, mut result_rx) = mpsc::unbounded_channel();
    for _ in 0..K {
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
    for _ in 0..K {
        within("the first poll of a call", admitted_rx.recv())
            .await
            .unwrap();
    }

    let first = within("the first result", result_rx.recv()).await.unwrap();
    assert_eq!(
        shared_handle_closed(),
        Some(false),
        "handle closed before shutdown"
    );
    let before_shutdown = Instant::now();
    let report = within("shutdown", lifecycle.shutdown(drain_timeout))
        .await
        .unwrap();
    let around_shutdown = before_shutdown.elapsed();

    let closed_after = shared_handle_closed();

    let mut results = vec![first];
    while results.len() < K {
        let next = tokio::time::timeout(Duration::from_secs(5), result_rx.recv()).await;
        let Ok(Some(result)) = next else {
            break;
        };
        results.push(result);
    }
    let ok_exact = results
        .iter()
        .filter(|r| matches!(r, Ok(x) if *x == SLOW_SECS))
        .count();
    let errored = results.iter().filter(|r| r.is_err()).count();
    eprintln!(
        "close split: ok={ok_exact} err={errored} returned={} remaining={} drained={} elapsed={:?}",
        results.len(),
        report.remaining,
        report.drained,
        report.elapsed
    );

    assert!(report.drained, "drained: got false, expected true");
    assert_eq!(report.remaining, 0, "remaining");
    assert!(
        report.elapsed < drain_timeout,
        "elapsed: got {:?}, expected less than {drain_timeout:?}",
        report.elapsed
    );
    assert!(
        report.elapsed <= around_shutdown,
        "elapsed: got {:?}, expected at most {around_shutdown:?}, the time the test measured around shutdown",
        report.elapsed
    );
    assert_eq!(ok_exact, K, "calls that returned Ok(0.02): {results:?}");
    assert_eq!(errored, 0, "calls that returned Err: {results:?}");
    assert_eq!(closed_after, Some(true), "handle closed after shutdown");
    assert!(
        report.close.is_ok(),
        "close step: got {:?}, expected Ok",
        report.close
    );

    let late = within("the call after shutdown", lifecycle.call_base1("sqrt", 4.0)).await;
    assert!(
        matches!(late, Err(LifecycleError::Cancelled)),
        "call after shutdown: got {late:?}, expected Err(Cancelled)"
    );
    let before = within(
        "the call through a lifecycle created before the shutdown",
        created_before.call_base1("sqrt", 4.0),
    )
    .await;
    assert!(
        matches!(before, Err(LifecycleError::Cancelled)),
        "call through a lifecycle created before the Close shutdown: got {before:?}, expected Err(Cancelled)"
    );
    let created_after = Lifecycle::new(Strategy::Drain);
    let after = within(
        "the call through a lifecycle created after the shutdown",
        created_after.call_base1("sqrt", 4.0),
    )
    .await;
    assert!(
        matches!(after, Err(LifecycleError::Cancelled)),
        "call through a lifecycle created after the Close shutdown: got {after:?}, expected Err(Cancelled)"
    );
    let direct = within(
        "the direct call after shutdown",
        shared::call_base1("sqrt", 4.0),
    )
    .await;
    eprintln!("close: direct call after shutdown resolved to {direct:?}");
    assert!(
        matches!(
            &direct,
            Err(error) if matches!(**error, JlrsError::RuntimeError(RuntimeError::ChannelClosed))
        ),
        "direct call after a Close shutdown: got {direct:?}, expected Err(ChannelClosed)"
    );
}
