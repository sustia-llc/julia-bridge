//! Julia can only be initialized once per process, and cargo runs the tests of one file
//! in one process. So everything that needs the runtime lives in this single test.

use julia_bridge::{JlrsResult, call_base2, eval_f64, par_map_f64, run};

#[test]
fn multithreaded_runtime() {
    run(2, |mut julia| -> JlrsResult<()> {
        assert_eq!(eval_f64(&mut julia, "1.5 + 2.5")?, 4.0);
        assert_eq!(call_base2(&mut julia, "hypot", 3.0, 4.0)?, 5.0);

        let inputs = [1.0, 4.0, 9.0, 16.0, 25.0];
        assert_eq!(
            par_map_f64(&julia, "sqrt", &inputs, 3)?,
            [1.0, 2.0, 3.0, 4.0, 5.0]
        );

        // Julia exceptions come back as errors, not panics.
        assert!(eval_f64(&mut julia, "error(\"boom\")").is_err());
        // So do type mismatches when unboxing.
        assert!(eval_f64(&mut julia, "1 + 1").is_err());
        Ok(())
    })
    .expect("failed to start Julia")
    .expect("Julia call failed");
}
