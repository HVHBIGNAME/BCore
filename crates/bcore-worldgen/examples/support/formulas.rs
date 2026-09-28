//! Edit candidates here, then run `math_lab experiment`. Production stays separate.
use super::numeric::{Precision, Spec};

pub struct Candidate {
    pub name: &'static str,
    pub sample: fn([f64; 3]) -> f64,
}

pub fn candidates(spec: &Spec) -> Vec<Candidate> {
    match spec {
        Spec::Smoothstep => vec![
            Candidate {
                name: "fade/expanded",
                sample: |[t, _, _]| {
                    6.0 * t * t * t * t * t - 15.0 * t * t * t * t + 10.0 * t * t * t
                },
            },
            Candidate {
                name: "fade/fma",
                sample: |[t, _, _]| t * t * t * t.mul_add(t.mul_add(6.0, -15.0), 10.0),
            },
        ],
        Spec::Lerp {
            precision: Precision::F64,
        } => vec![
            Candidate {
                name: "lerp/weighted",
                sample: |[t, a, b]| (1.0 - t) * a + t * b,
            },
            Candidate {
                name: "lerp/fma",
                sample: |[t, a, b]| t.mul_add(b - a, a),
            },
        ],
        Spec::Lerp {
            precision: Precision::F32,
        } => vec![
            Candidate {
                name: "lerp_f32/weighted",
                sample: |p| {
                    let [t, a, b] = p.map(|v| v as f32);
                    ((1.0 - t) * a + t * b) as f64
                },
            },
            Candidate {
                name: "lerp_f32/fma",
                sample: |p| {
                    let [t, a, b] = p.map(|v| v as f32);
                    t.mul_add(b - a, a) as f64
                },
            },
        ],
        _ => Vec::new(),
    }
}
