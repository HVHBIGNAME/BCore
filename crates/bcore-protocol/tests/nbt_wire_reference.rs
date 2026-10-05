use bcore_protocol::nbt::{encode_text, encode_typed_nbt};
use bcore_worldgen::structure::template::Nbt;
use sha2::{Digest, Sha256};

#[test]
fn all_typed_payloads_match_the_native_anonymous_nbt_writer() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../data/nbt_wire_26_1.json")).unwrap();
    assert_eq!(fixture["minecraft"], "26.1");
    assert_eq!(
        fixture["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    assert_eq!(
        fixture["probe_sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../../../scripts/NbtWireReference.java"))
        )
    );
    assert_eq!(
        fixture["capture_sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../../../scripts/capture_nbt_reference.py"))
        )
    );
    for row in fixture["samples"].as_array().unwrap() {
        let name = row["name"].as_str().unwrap();
        let value: Nbt = match name {
            "float_nan_payload" => Nbt::Float(f32::from_bits(
                u32::from_str_radix(row["input_bits"].as_str().unwrap(), 16).unwrap(),
            )),
            "double_nan_payload" => Nbt::Double(f64::from_bits(
                u64::from_str_radix(row["input_bits"].as_str().unwrap(), 16).unwrap(),
            )),
            _ => serde_json::from_value(row["nbt"].clone()).unwrap(),
        };
        let actual = encode_typed_nbt(&value).unwrap();
        let expected = row["hex"].as_str().unwrap();
        let expected: Vec<_> = (0..expected.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&expected[at..at + 2], 16).unwrap())
            .collect();
        assert_eq!(actual, expected, "native NbtIo case {name}");
        if let Nbt::String(text) = value {
            assert_eq!(
                encode_text(&text),
                expected,
                "chat must use the same native string encoding"
            );
        }
    }
}
