use bcore_worldgen::density::{self, DensityFunction, EvalContext, EvaluationMode};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn fixture() -> Value {
    let bytes = include_bytes!("../data/interval_select_26_2.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "1d716a9ba300c0f2c89604770bf79019c218ec3973791672fbb4a1fb3625605b"
    );
    let fixture: Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["minecraft"], bcore_registry::MC_VERSION);
    assert_eq!(fixture["protocol"], bcore_registry::PROTOCOL_VERSION);
    assert_eq!(fixture["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
    fixture
}

#[test]
fn native_interval_codec_cardinality_order_and_numeric_bounds_match() {
    let fixture = fixture();
    let inputs = fixture["inputs"]["codecs"].as_array().unwrap();
    let results = fixture["codecs"].as_array().unwrap();
    assert_eq!(inputs.len(), 18);
    assert_eq!(results.len(), inputs.len());
    let mut accepted = 0;
    for (input, expected) in inputs.iter().zip(results) {
        assert_eq!(input["id"], expected["id"]);
        let parsed = density::parse_json(&input["function"].to_string());
        assert_eq!(
            parsed.is_ok(),
            expected["accepted"].as_bool().unwrap(),
            "{}: {parsed:?}",
            input["id"]
        );
        if let Ok(function) = parsed {
            let actual = function.evaluate(0.0, 0.0, 0.0, &EvalContext::default());
            assert_eq!(format!("{:016x}", actual.to_bits()), expected["value_bits"]);
            accepted += 1;
        }
    }
    assert_eq!(accepted, 4);
}

#[test]
fn native_interval_signed_zero_infinity_nan_and_lazy_branch_selection_match() {
    let fixture = fixture();
    let thresholds: Vec<f64> =
        serde_json::from_value(fixture["inputs"]["runtime"]["thresholds"].clone()).unwrap();
    let values: Vec<f64> =
        serde_json::from_value(fixture["inputs"]["runtime"]["functions"].clone()).unwrap();
    let results = fixture["runtime"].as_array().unwrap();
    assert_eq!(results.len(), 9);
    for sample in results {
        let input = f64::from_bits(
            u64::from_str_radix(sample["input_bits"].as_str().unwrap(), 16).unwrap(),
        );
        let selected = sample["calls"][1]
            .as_str()
            .unwrap()
            .strip_prefix("function/")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(sample["calls"][0], "input");
        assert_eq!(sample["calls"].as_array().unwrap().len(), 2);
        let function = DensityFunction::IntervalSelect {
            input: Box::new(DensityFunction::Constant(input)),
            thresholds: thresholds.clone(),
            functions: values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    if index == selected {
                        Box::new(DensityFunction::Constant(*value))
                    } else {
                        Box::new(DensityFunction::IntervalSelect {
                            input: Box::new(DensityFunction::Constant(0.0)),
                            thresholds: Vec::new(),
                            functions: Vec::new(),
                        })
                    }
                })
                .collect(),
        };
        for mode in [
            EvaluationMode::Raw,
            EvaluationMode::NoiseChunk,
            EvaluationMode::NoiseChunkMaterial,
        ] {
            let actual = function.evaluate(
                0.0,
                0.0,
                0.0,
                &EvalContext {
                    mode,
                    ..Default::default()
                },
            );
            assert_eq!(
                format!("{:016x}", actual.to_bits()),
                sample["output_bits"],
                "input {input:?}"
            );
        }
    }
}

#[test]
fn malformed_interval_documents_cannot_become_silent_zero_densities() {
    for document in [
        r#"{"type":"minecraft:interval_select","input":false,"thresholds":[0],"functions":[1,2]}"#,
        r#"{"type":"minecraft:interval_select","input":0,"thresholds":[0],"functions":[1,{"type":"minecraft:unknown"}]}"#,
        r#"{"type":"minecraft:interval_select","input":0,"thresholds":[0],"functions":[1,2]} trailing"#,
    ] {
        assert!(density::parse_json(document).is_err(), "{document}");
    }
}
