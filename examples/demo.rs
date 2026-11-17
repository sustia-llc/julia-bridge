//! Run with: cargo run --example demo

use julia_bridge::{call_base2, eval_f64, par_map_f64, run};

fn main() {
    let result = run(4, |mut julia| -> julia_bridge::JlrsResult<()> {
        println!(
            "eval      sqrt(2)     = {}",
            eval_f64(&mut julia, "sqrt(2.0)")?
        );
        println!(
            "call      hypot(3, 4) = {}",
            call_base2(&mut julia, "hypot", 3.0, 4.0)?
        );

        let inputs: Vec<f64> = (1..=8).map(f64::from).collect();
        println!(
            "par_map   sqrt.(1:8)  = {:?}",
            par_map_f64(&julia, "sqrt", &inputs, 4)?
        );
        Ok(())
    });

    // Outer Result: did Julia start? Inner Result: did our Julia calls succeed?
    result
        .expect("failed to start Julia")
        .expect("Julia call failed");
}
