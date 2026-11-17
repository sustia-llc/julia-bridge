//! Run with: cargo nextest run --features lifecycle
//!
//! This file holds one test function, which shuts a `Strategy::Close` lifecycle down.
#![cfg(feature = "lifecycle")]

use std::time::Duration;

use jlrs::error::{JlrsError, RuntimeError};
use julia_bridge::{
    lifecycle::{Lifecycle, Strategy},
    shared,
};

#[tokio::test(flavor = "multi_thread")]
async fn close_without_a_call_does_not_start_julia() {
    let lifecycle = Lifecycle::new(Strategy::Close);
    assert!(
        shared::julia_if_started().is_none(),
        "Julia started before shutdown"
    );

    let report = tokio::time::timeout(
        Duration::from_secs(30),
        lifecycle.shutdown(Duration::from_secs(1)),
    )
    .await
    .expect("shutdown did not return within 30 s")
    .unwrap();

    assert!(
        shared::julia_if_started().is_none(),
        "Julia after shutdown: got started, expected not started"
    );
    assert!(report.drained, "drained: got false, expected true");
    assert_eq!(report.remaining, 0, "remaining");
    assert!(
        report.close.is_ok(),
        "close step: got {:?}, expected Ok",
        report.close
    );

    let direct = shared::call_base1("sqrt", 4.0).await;
    assert!(
        matches!(
            &direct,
            Err(error) if matches!(**error, JlrsError::RuntimeError(RuntimeError::ChannelClosed))
        ),
        "direct call after a Close shutdown: got {direct:?}, expected Err(ChannelClosed)"
    );
    let eval = shared::eval_f64("1.0").await;
    assert!(
        matches!(
            &eval,
            Err(error) if matches!(**error, JlrsError::RuntimeError(RuntimeError::ChannelClosed))
        ),
        "direct eval after a Close shutdown: got {eval:?}, expected Err(ChannelClosed)"
    );
    assert!(
        shared::julia_if_started().is_none(),
        "Julia after the refused calls: got started, expected not started"
    );
}
