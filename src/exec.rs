//! Caller-owned rayon thread pool whose jobs are awaited from async code (feature `exec`).
//!
//! Use it for CPU-heavy pure-Rust work, so that work blocks neither tokio's worker
//! threads nor the Julia runtime thread. Pool threads are not Julia threads: a job must
//! not call into Julia directly. Compute here, then pass the plain Rust result to the
//! functions in [`crate::shared`].

use tokio_rayon::AsyncThreadPool;
pub use tokio_rayon::rayon;

/// A rayon thread pool with threads named `julia-bridge-exec-{i}`, running each
/// submitted closure on the pool and resolving to its value.
#[derive(Debug)]
pub struct Executor {
    pool: rayon::ThreadPool,
}

impl Executor {
    /// Builds an executor with rayon's default thread count, or an error when the pool
    /// cannot be built.
    pub fn new() -> Result<Self, rayon::ThreadPoolBuildError> {
        Self::build(rayon::ThreadPoolBuilder::new())
    }

    /// Builds an executor with `num_threads` threads (rayon's default count when `0`),
    /// or an error when the pool cannot be built.
    pub fn with_num_threads(num_threads: usize) -> Result<Self, rayon::ThreadPoolBuildError> {
        Self::build(rayon::ThreadPoolBuilder::new().num_threads(num_threads))
    }

    fn build(builder: rayon::ThreadPoolBuilder) -> Result<Self, rayon::ThreadPoolBuildError> {
        let pool = builder
            .thread_name(|i| format!("julia-bridge-exec-{i}"))
            .build()?;
        Ok(Self { pool })
    }

    /// Runs `f` on the pool and resolves to its return value; a panic in `f` resumes on
    /// the awaiting task. Rayon's parallel iterators used inside `f` run on this pool,
    /// not the global one.
    ///
    /// A future dropped before its first poll does not run `f`. A future dropped after
    /// its first poll discards the return value of `f`, and a panic of `f` then reaches
    /// only the panic hook.
    pub async fn run<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        self.pool.spawn_async(f).await
    }
}
