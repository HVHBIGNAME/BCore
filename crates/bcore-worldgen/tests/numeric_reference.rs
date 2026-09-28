#[path = "../examples/support/numeric.rs"]
mod numeric;

use numeric::{Fixture, Precision};
use rayon::prelude::*;

fn fixture() -> Fixture {
    let fixture: Fixture = serde_json::from_str(include_str!("../data/numeric_26_1.json")).unwrap();
    fixture.validate(true).unwrap();
    assert_eq!(fixture.cases.len(), 895);
    assert_eq!(
        fixture.samples.iter().map(|s| s.bits.len()).sum::<usize>(),
        62649
    );
    fixture
}

#[test]
fn numeric_values_match_native_26_1_bit_for_bit() {
    let fixture = fixture();
    let failures: Vec<_> = fixture
        .cases
        .iter()
        .map(|case| fixture.check(case).unwrap())
        .filter(|report| report.mismatches != 0)
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn reversed_inputs_and_four_workers_preserve_native_values() {
    let fixture = fixture();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let failures: Vec<_> = pool.install(|| {
        fixture
            .cases
            .par_iter()
            .rev()
            .map(|case| {
                let points = &fixture.points[&case.points];
                let kernel = case.prepare(points).unwrap();
                let mut values = vec![0.0; points.len()];
                for (i, p) in points.iter().enumerate().rev() {
                    values[i] = kernel.sample(*p);
                }
                let mut values = values.into_iter();
                numeric::compare(case, points, fixture.reference(case).unwrap(), |_| {
                    values.next().unwrap()
                })
                .unwrap()
            })
            .filter(|report| report.mismatches != 0)
            .collect()
    });
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn comparison_rejects_a_single_bit_and_a_zero_sign_change() {
    let mut fixture = fixture();
    for (id, wrong) in [
        ("scalar/wrap", "0000000000000001"),
        ("scalar/lerp/f32", "80000000"),
    ] {
        fixture
            .samples
            .iter_mut()
            .find(|s| s.id == id)
            .unwrap()
            .bits[0] = wrong.into();
        let case = fixture.cases.iter().find(|c| c.id == id).unwrap();
        let report = fixture.check(case).unwrap();
        assert_eq!(report.mismatches, 1, "{report:#?}");
        assert_eq!(report.max_ulps, 1, "{report:#?}");
    }
}

#[test]
fn ulp_distance_is_monotonic_across_negative_values_and_zero() {
    for (a, b) in [
        (-1.0f64, (-1.0f64).next_up()),
        (-0.0, 0.0),
        (0.0, 0.0f64.next_up()),
    ] {
        assert_eq!(Precision::F64.ulps(a.to_bits(), b.to_bits()), 1);
    }
    for (a, b) in [
        (-1.0f32, (-1.0f32).next_up()),
        (-0.0, 0.0),
        (0.0, 0.0f32.next_up()),
    ] {
        assert_eq!(
            Precision::F32.ulps(a.to_bits() as u64, b.to_bits() as u64),
            1
        );
    }
}
