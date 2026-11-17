//! Run with: cargo nextest run --features lifecycle
#![cfg(feature = "lifecycle")]

mod common;

use std::{
    future::{Future, poll_fn},
    pin::pin,
    sync::Arc,
    task::Poll,
    time::{Duration, Instant},
};

use common::{K, SLOW_FN, SLOW_SECS, admitted_then, define_slow, shared_handle_closed};
use julia_bridge::{
    lifecycle::{Lifecycle, LifecycleError, Strategy},
    shared,
};
use tokio::sync::mpsc;

#[tokio::test(flavor = "multi_thread")]
async fn drain_returns_every_admitted_call() {
    define_slow().await;
    let lifecycle = Arc::new(Lifecycle::new(Strategy::Drain));
    let other = Lifecycle::new(Strategy::Drain);
    assert_eq!(lifecycle.strategy(), Strategy::Drain);
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
        admitted_rx.recv().await.unwrap();
    }

    let first = result_rx.recv().await.unwrap();
    let before_shutdown = Instant::now();
    let report = lifecycle.shutdown(drain_timeout).await.unwrap();
    let around_shutdown = before_shutdown.elapsed();

    let mut results = vec![first];
    while let Some(result) = result_rx.recv().await {
        results.push(result);
    }
    let ok_exact = results
        .iter()
        .filter(|r| matches!(r, Ok(x) if *x == SLOW_SECS))
        .count();

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

    let late = lifecycle.call_base1("sqrt", 4.0).await;
    assert!(
        matches!(late, Err(LifecycleError::Cancelled)),
        "call after shutdown: got {late:?}, expected Err(Cancelled)"
    );
    let late = lifecycle.eval_f64("1.0 + 1.0").await;
    assert!(
        matches!(late, Err(LifecycleError::Cancelled)),
        "eval after shutdown: got {late:?}, expected Err(Cancelled)"
    );

    assert_eq!(
        shared_handle_closed(),
        Some(false),
        "handle closed after a Drain shutdown"
    );
    assert!(
        report.close.is_ok(),
        "close step under Drain: got {:?}, expected Ok",
        report.close
    );
    let other_call = other.call_base1("sqrt", 4.0).await;
    assert!(
        matches!(other_call, Ok(x) if x == 2.0),
        "call through another lifecycle after a Drain shutdown: got {other_call:?}, expected Ok(2.0)"
    );
    let direct = shared::call_base1("sqrt", 4.0).await;
    assert!(
        matches!(direct, Ok(x) if x == 2.0),
        "direct call after a Drain shutdown: got {direct:?}, expected Ok(2.0)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn drain_timeout_reports_the_running_call() {
    define_slow().await;
    let lifecycle = Lifecycle::new(Strategy::Drain);
    let drain_timeout = Duration::from_millis(50);

    let mut call = pin!(lifecycle.call_base1(SLOW_FN, 2.0));
    let first = poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx))).await;
    assert!(first.is_pending(), "first poll of slow(2.0): got {first:?}");

    let before_shutdown = Instant::now();
    let report = lifecycle.shutdown(drain_timeout).await.unwrap();
    let around_shutdown = before_shutdown.elapsed();

    assert!(!report.drained, "drained: got true, expected false");
    assert_eq!(report.remaining, 1, "remaining");
    assert!(
        report.elapsed >= drain_timeout,
        "elapsed: got {:?}, expected at least {drain_timeout:?}",
        report.elapsed
    );
    assert!(
        report.elapsed <= around_shutdown,
        "elapsed: got {:?}, expected at most {around_shutdown:?}, the time the test measured around shutdown",
        report.elapsed
    );

    let second = lifecycle.shutdown(Duration::from_secs(30)).await.unwrap();
    assert!(second.drained, "second drained: got false, expected true");
    assert_eq!(second.remaining, 0, "second remaining");
}

#[tokio::test(flavor = "multi_thread")]
async fn drain_counts_a_call_whose_caller_dropped_it() {
    define_slow().await;
    let lifecycle = Lifecycle::new(Strategy::Drain);

    {
        let mut call = pin!(lifecycle.call_base1(SLOW_FN, 0.5));
        let first = poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx))).await;
        assert!(first.is_pending(), "first poll of slow(0.5): got {first:?}");
    }

    let report = lifecycle.shutdown(Duration::from_millis(50)).await.unwrap();

    assert!(!report.drained, "drained: got true, expected false");
    assert_eq!(report.remaining, 1, "remaining");

    let second = lifecycle.shutdown(Duration::from_secs(30)).await.unwrap();
    assert!(second.drained, "second drained: got false, expected true");
    assert_eq!(second.remaining, 0, "second remaining");
}

#[tokio::test(flavor = "multi_thread")]
async fn external_cancel_rejects_the_next_call() {
    for strategy in [Strategy::Drain, Strategy::Close] {
        let lifecycle = Lifecycle::new(strategy);
        lifecycle.cancellation_token().cancel();

        let call = lifecycle.call_base1("sqrt", 4.0).await;
        assert!(
            matches!(call, Err(LifecycleError::Cancelled)),
            "{strategy:?} call after cancel: got {call:?}, expected Err(Cancelled)"
        );
        let eval = lifecycle.eval_f64("1.0 + 1.0").await;
        assert!(
            matches!(eval, Err(LifecycleError::Cancelled)),
            "{strategy:?} eval after cancel: got {eval:?}, expected Err(Cancelled)"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn julia_error_displays_the_jlrs_error() {
    let lifecycle = Lifecycle::new(Strategy::Drain);

    let failed = lifecycle.call_base1("no_such_function", 1.0).await;
    let Err(error) = failed else {
        panic!("call of no_such_function: got {failed:?}, expected Err(Julia(_))");
    };
    let text = error.to_string();
    let LifecycleError::Julia(inner) = error else {
        panic!("call of no_such_function: got {error:?}, expected Julia(_)");
    };

    assert_eq!(text, inner.to_string(), "Display of LifecycleError::Julia");
    assert!(
        text.contains("no_such_function"),
        "Display of LifecycleError::Julia: got {text:?}, expected it to name no_such_function"
    );
}
