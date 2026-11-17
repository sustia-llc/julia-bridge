//! Rayon for the Rust-side number crunching, Julia for the rest.
//!
//! Run with: cargo run --features tokio,exec --example pipeline

use julia_bridge::{
    exec::{Executor, rayon::prelude::*},
    shared::{call_base1, julia},
};

#[tokio::main]
async fn main() {
    let exec = Executor::new().expect("failed to build thread pool");
    tokio::task::spawn_blocking(julia).await.unwrap();

    // Step 1: CPU-bound Rust work on the rayon pool. The tokio worker is free meanwhile.
    let sum_of_squares: u64 = exec
        .run(|| (1..=1_000_000u64).into_par_iter().map(|x| x * x).sum())
        .await;

    // Step 2: hand the plain Rust result to Julia.
    let root = call_base1("sqrt", sum_of_squares as f64).await.unwrap();

    println!("rayon  sum of squares 1..=1e6 = {sum_of_squares}");
    println!("julia  sqrt of that           = {root}");
}
