//! Run with: cargo test --features exec
#![cfg(feature = "exec")]

use julia_bridge::exec::{
    Executor,
    rayon::{self, prelude::*},
};

#[tokio::test]
async fn runs_on_named_pool_thread() {
    let exec = Executor::with_num_threads(2).unwrap();
    let name = exec
        .run(|| std::thread::current().name().map(str::to_owned))
        .await;
    assert!(name.unwrap().starts_with("julia-bridge-exec-"));
}

#[tokio::test]
async fn parallel_iterators_use_this_pool() {
    let exec = Executor::with_num_threads(3).unwrap();
    let (sum, threads) = exec
        .run(|| {
            let sum: u64 = (1..=1_000u64).into_par_iter().map(|x| x * x).sum();
            (sum, rayon::current_num_threads())
        })
        .await;
    assert_eq!(sum, 333_833_500);
    assert_eq!(threads, 3);
}

#[tokio::test]
#[should_panic(expected = "job failed")]
async fn panics_resume_on_the_awaiting_task() {
    let exec = Executor::new().unwrap();
    exec.run(|| panic!("job failed")).await
}
