use bcore_worldgen::mth;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Barrier, OnceLock};

const TABLE_SHA256: &str = "cdfaec6870788e193dff3f1ddfa3ca7d3f1a897042d78365a1fe3a433be3627d";

#[derive(Debug, Deserialize)]
struct Sample {
    precision: String,
    #[serde(rename = "case")]
    label: String,
    input_bits: String,
    sin_bits: String,
    cos_bits: String,
}

#[derive(Deserialize)]
struct Fixture {
    minecraft: String,
    jar_sha256: String,
    methods: Vec<String>,
    float_overloads: BTreeMap<String, bool>,
    sin_scale_bits: String,
    sin_mask: i64,
    cos_offset: i64,
    sin_table_length: usize,
    sin_table_sha256_le: String,
    sample_counts: BTreeMap<String, usize>,
    samples: Vec<Sample>,
    sin_table_bits: Vec<String>,
}

fn bits32(text: &str) -> u32 {
    assert_eq!(text.len(), 8);
    u32::from_str_radix(text, 16).unwrap()
}

fn bits64(text: &str) -> u64 {
    assert_eq!(text.len(), 16);
    u64::from_str_radix(text, 16).unwrap()
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let fixture: Fixture = serde_json::from_str(include_str!("../data/mth_26_1.json")).unwrap();
        assert_eq!(fixture.minecraft, "26.1");
        assert_eq!(
            fixture.jar_sha256,
            "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
        );
        assert_eq!(
            fixture.methods,
            [
                "public static float net.minecraft.util.Mth.sin(double)",
                "public static float net.minecraft.util.Mth.cos(double)",
            ]
        );
        assert_eq!(
            fixture.float_overloads,
            BTreeMap::from([("sin".into(), false), ("cos".into(), false)])
        );
        assert_eq!(
            bits64(&fixture.sin_scale_bits),
            10430.378350470453_f64.to_bits()
        );
        assert_eq!((fixture.sin_mask, fixture.cos_offset), (65535, 16384));
        assert_eq!(fixture.sin_table_length, 65_536);
        assert_eq!(fixture.sin_table_bits.len(), 65_536);
        assert_eq!(fixture.sin_table_sha256_le, TABLE_SHA256);
        assert_eq!(fixture.samples.len(), 1903);
        let mut counts = BTreeMap::new();
        let mut unique = BTreeSet::new();
        for sample in &fixture.samples {
            *counts.entry(sample.precision.clone()).or_insert(0) += 1;
            assert!(
                unique.insert((&sample.precision, &sample.input_bits)),
                "{sample:?}"
            );
        }
        assert_eq!(
            counts,
            BTreeMap::from([("f32".into(), 898), ("f64".into(), 1005)])
        );
        assert_eq!(fixture.sample_counts, counts);
        fixture
    })
}

fn check(sample: &Sample) {
    let (sin, cos) = match sample.precision.as_str() {
        "f32" => {
            let input = f32::from_bits(bits32(&sample.input_bits));
            (mth::sin_f32(input), mth::cos_f32(input))
        }
        "f64" => {
            let input = f64::from_bits(bits64(&sample.input_bits));
            (mth::sin(input), mth::cos(input))
        }
        other => panic!("unknown input precision: {other}"),
    };
    assert_eq!(
        sin.to_bits(),
        bits32(&sample.sin_bits),
        "sin: {} {sample:?}",
        sample.label
    );
    assert_eq!(
        cos.to_bits(),
        bits32(&sample.cos_bits),
        "cos: {} {sample:?}",
        sample.label
    );
}

#[test]
fn every_native_sine_table_entry_matches_bit_for_bit() {
    let fixture = fixture();
    let table = mth::sin_table();
    assert_eq!(mth::SIN_TABLE_LEN, 65_536);
    let mut digest = Sha256::new();
    for (index, (&actual, expected)) in table.iter().zip(&fixture.sin_table_bits).enumerate() {
        assert_eq!(actual.to_bits(), bits32(expected), "native SIN[{index}]");
        digest.update(actual.to_bits().to_le_bytes());
    }
    assert_eq!(format!("{:x}", digest.finalize()), TABLE_SHA256);
    println!("65,536 native SIN entries matched; SHA-256 (little-endian bits): {TABLE_SHA256}");
}

#[test]
fn float_promoted_and_double_arguments_match_native_samples() {
    let fixture = fixture();
    for sample in &fixture.samples {
        check(sample);
    }
    println!("1,903 native angle samples matched: 898 f32-promoted + 1,005 f64; 3,806 outputs");
}

#[test]
fn concurrent_lookups_share_one_table_and_preserve_native_bits() {
    let fixture = fixture();
    let barrier = Barrier::new(8);
    let tables = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    let table = mth::sin_table();
                    for sample in fixture.samples.iter().rev() {
                        check(sample);
                    }
                    table
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(tables.iter().all(|table| std::ptr::eq(tables[0], *table)));
}
