use super::*;
use crate::density_materials_tests::{bits, chunk_pos, fixture, pos, selected, starts};

#[test]
fn native_float_kernel_and_exact_bury_beard_contributions() {
    let fixture = fixture(include_str!("../data/beardifier_26_1.json"), "beard");
    assert_eq!(fixture["kernel"].as_array().unwrap().len(), KERNEL_LENGTH);
    for (index, value) in fixture["kernel"].as_array().unwrap().iter().enumerate() {
        assert_eq!(
            kernel()[index].to_bits(),
            bits(value) as u32,
            "kernel index {index}"
        );
    }
    for case in fixture["contributions"].as_array().unwrap() {
        let (x, y, z) = pos(&case["input"]);
        let ground = case["input"][3].as_i64().unwrap() as i32;
        assert_eq!(
            beard(x, y, z, ground).to_bits(),
            bits(&case["bits"]),
            "{case}"
        );
    }
    for case in fixture["buries"].as_array().unwrap() {
        let values: Vec<_> = case["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| f64::from_bits(bits(v)))
            .collect();
        assert_eq!(
            bury(values[0], values[1], values[2]).to_bits(),
            bits(&case["bits"]),
            "{case}"
        );
    }
}

#[test]
fn native_start_selection_projection_reference_margins_and_density_bits() {
    let fixture = fixture(include_str!("../data/beardifier_26_1.json"), "beard");
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 20);
    let mut sample_count = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let from_native_parts = selected(&case["selected"]);
        let ordinary_piece = case["starts"].as_array().unwrap().iter().any(|s| {
            s["pieces"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["projection"] == "non_pool")
        });
        let beard = if ordinary_piece {
            // The typed selected-piece API serves non-pool structures; the
            // for_chunk adapter is specifically for the real JigsawStart type.
            from_native_parts.clone()
        } else {
            let starts = starts(&case["starts"]);
            let beard = Beardifier::for_chunk(chunk_pos(&case["chunk"]), &starts).unwrap();
            assert_eq!(
                beard.pieces(),
                from_native_parts.pieces(),
                "{id}: piece selection"
            );
            assert_eq!(
                beard.junctions(),
                from_native_parts.junctions(),
                "{id}: junction selection/order"
            );
            assert_eq!(
                beard.affected_bounds(),
                from_native_parts.affected_bounds(),
                "{id}: affected guard"
            );
            beard
        };
        for (point, value) in case["points"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["bits"].as_array().unwrap())
        {
            let (x, y, z) = pos(point);
            assert_eq!(
                beard.compute(x, y, z).to_bits(),
                bits(value),
                "{id} at {x},{y},{z}"
            );
            sample_count += 1;
        }
        // No mutable iterator cursor or history is retained by a density query.
        for (point, value) in case["points"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["bits"].as_array().unwrap())
            .rev()
            .step_by(97)
        {
            let (x, y, z) = pos(point);
            assert_eq!(
                beard.compute(x, y, z).to_bits(),
                bits(value),
                "{id}: repeated reverse query"
            );
        }
    }
    assert_eq!(sample_count, 112_640);
}

#[test]
fn unknown_adjustments_are_rejected_and_empty_beards_are_exact_zero() {
    let start = JigsawStart {
        generation_point: (0, 0, 0),
        pieces: Vec::new(),
        terrain_adaptation: "future_adjustment".into(),
        decoration_step: "surface_structures".into(),
        references: 0,
    };
    assert!(matches!(
        Beardifier::for_chunk(ChunkPos::new(0, 0), [&start]),
        Err(FeatureError::Unsupported(_))
    ));
    for p in [(0, 0, 0), (-1, -64, -1), (i32::MIN, i32::MAX, i32::MIN)] {
        assert_eq!(Beardifier::default().compute(p.0, p.1, p.2).to_bits(), 0);
    }
}
