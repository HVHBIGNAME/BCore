//! Isolate interpolated: should return the inner constant.
use bcore_worldgen::density::{self, EvalContext};

fn main() {
    let ctx = EvalContext::default();
    // interpolated(constant 5) should be 5 everywhere.
    let f = density::parse_json(
        r#"{"type":"minecraft:interpolated","argument":{"type":"minecraft:constant","value":5}}"#,
    )
    .unwrap();
    for (x, y, z) in [(8.0, 64.0, 8.0), (9.0, 65.0, 9.0), (0.0, 0.0, 0.0)] {
        println!("interpolated(5) at ({x},{y},{z}) = {:.4}", density::evaluate(&f, x, y, z, &ctx));
    }
}
