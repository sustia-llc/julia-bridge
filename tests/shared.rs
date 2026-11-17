//! Run with: cargo test --features tokio
//!
//! Unlike `tests/runtime.rs`, this file has several tests. They all run in one process
//! and all use the same lazily started runtime, so the once-per-process rule is no
//! longer something each test has to work around.
#![cfg(feature = "tokio")]

use julia_bridge::shared::{call_base1, eval_f64};

#[tokio::test(flavor = "multi_thread")]
async fn eval() {
    assert_eq!(eval_f64("1.5 + 2.5").await.unwrap(), 4.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn call() {
    assert_eq!(call_base1("sqrt", 81.0).await.unwrap(), 9.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn errors_are_results() {
    assert!(eval_f64("error(\"boom\")").await.is_err());
    assert!(call_base1("no_such_function", 1.0).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn many_tasks_at_once() {
    let tasks: Vec<_> = (1..=100)
        .map(|i| tokio::spawn(call_base1("sqrt", f64::from(i * i))))
        .collect();

    for (i, task) in (1..=100).zip(tasks) {
        assert_eq!(task.await.unwrap().unwrap(), f64::from(i));
    }
}
