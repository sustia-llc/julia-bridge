# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-11-16

### Added

- Feature `lifecycle` with the module `lifecycle`: `Lifecycle`, `Strategy`,
  `LifecycleError` and `ShutdownReport`.
- `Lifecycle::call_base1` and `Lifecycle::eval_f64` run a `shared` call in a tracked
  tokio task. They return `LifecycleError::Cancelled` after the token is cancelled.
- `Lifecycle::shutdown` cancels the token, closes the tracker and waits for the tracked
  calls, for the drain timeout at most.
- `Strategy::Drain` leaves the shared Julia handle open. `Strategy::Close` cancels a
  process-wide root token and awaits `shared::close_and_wait`.
- `Lifecycle::cancellation_token`, `Lifecycle::strategy` and `Lifecycle::is_closed`.
- `shared::close` sets a process-wide closed flag and closes the shared handle.
- `shared::close_and_wait` does the same and then waits until the Julia runtime thread
  ends.
- `shared::julia_if_started` returns the shared handle and does not start Julia.
- Benches `per_call` and `shutdown`, with criterion.

### Changed

- `shared::MAX_CONCURRENT_TASKS` is public.
- After a close, `shared::eval_f64` and `shared::call_base1` return the channel-closed
  error and do not start Julia.
- `shared::eval_f64` and `shared::call_base1` return an error if Julia cannot start.
- The feature `tokio` depends on the crate `tokio` with its `sync` feature.
- The examples `tokio_demo` and `pipeline` await `shared::close_and_wait` before `main`
  returns.

## [0.1.0] - 2026-11-16

### Added

- `run` starts Julia with the jlrs multithreaded runtime. `eval_f64`, `call_base2` and
  `par_map_f64` call into Julia.
- Feature `tokio` with the module `shared`: `julia`, `eval_f64` and `call_base1` use one
  process-wide Julia runtime from tokio tasks.
- Feature `exec` with the module `exec`: `Executor` runs closures on a rayon thread
  pool and returns their values to async code.
- `build.rs` adds an rpath of `$JLRS_JULIA_DIR/lib` to the tests, examples and benches.
- Examples `demo`, `tokio_demo` and `pipeline`.
- Environment files `contrib/julia-env.sh` and `contrib/julia.fish`.
- `rust-toolchain.toml` with the stable channel.
