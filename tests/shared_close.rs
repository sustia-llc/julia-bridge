//! Run with: cargo nextest run --features tokio
//!
//! This file holds one test function, which closes the shared Julia handle.
#![cfg(feature = "tokio")]

use std::time::Duration;

use jlrs::error::{JlrsError, RuntimeError};
use julia_bridge::shared;

#[tokio::test(flavor = "multi_thread")]
async fn close_refuses_calls_and_closes_a_started_handle() {
    shared::close();

    let refused = shared::call_base1("sqrt", 4.0).await;
    assert!(
        matches!(
            &refused,
            Err(error) if matches!(**error, JlrsError::RuntimeError(RuntimeError::ChannelClosed))
        ),
        "call after close: got {refused:?}, expected Err(ChannelClosed)"
    );
    assert!(
        shared::julia_if_started().is_none(),
        "Julia after close and a refused call: got started, expected not started"
    );

    let started_after_close = shared::julia().is_closed();
    shared::close();
    let closed = shared::julia_if_started().map(|handle| handle.is_closed());

    assert!(
        !started_after_close,
        "handle that julia() started after a close: got closed, expected open"
    );
    assert_eq!(closed, Some(true), "handle closed after close");

    let waited = tokio::time::timeout(Duration::from_secs(30), shared::close_and_wait())
        .await
        .expect("close_and_wait did not return within 30 s");
    assert!(
        waited.is_ok(),
        "close_and_wait: got {waited:?}, expected Ok"
    );

    let eval = tokio::time::timeout(Duration::from_secs(30), shared::eval_f64("1.0"))
        .await
        .expect("eval_f64 did not return within 30 s");
    assert!(
        matches!(
            &eval,
            Err(error) if matches!(**error, JlrsError::RuntimeError(RuntimeError::ChannelClosed))
        ),
        "eval after the close of a started handle: got {eval:?}, expected Err(ChannelClosed)"
    );
}
