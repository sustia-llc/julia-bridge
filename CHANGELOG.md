# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
