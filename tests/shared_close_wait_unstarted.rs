//! Run with: cargo nextest run --features tokio
//!
//! This file holds one test function, which closes the shared Julia handle.
#![cfg(feature = "tokio")]

use julia_bridge::shared;

#[tokio::test]
async fn close_and_wait_without_julia_returns_ok_and_does_not_start_julia() {
    let waited = shared::close_and_wait().await;

    assert!(
        waited.is_ok(),
        "close_and_wait with Julia never started: got {waited:?}, expected Ok"
    );
    assert!(
        shared::julia_if_started().is_none(),
        "Julia after close_and_wait: got started, expected not started"
    );
}
