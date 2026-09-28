//! Native JAR captures; compare IEEE-754 bits, not an epsilon or decimal rendering.
use bcore_worldgen::{noise_perlin, random, simplex};

fn digest(draws: u64, mut next: impl FnMut() -> f64) -> String {
    let mut digest = md5::Context::new();
    for _ in 0..draws {
        digest.consume(next().to_le_bytes());
    }
    format!("{:x}", digest.compute())
}

#[test]
fn double_streams_match_vanilla_26_1() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../data/random_26_1.json")).unwrap();
    let draws = fixture["draws"].as_u64().unwrap();
    for sample in fixture["samples"].as_array().unwrap() {
        let seed = sample["seed"].as_i64().unwrap();
        let expected = sample["doubles_md5"].as_str().unwrap();
        let next = sample["next_i64"].as_i64().unwrap();
        match sample["kind"].as_str().unwrap() {
            "legacy" => {
                let mut rng = simplex::JavaRandom::new(seed);
                assert_eq!(
                    digest(draws, || rng.next_double()),
                    expected,
                    "legacy seed {seed}"
                );
                assert_eq!(rng.next_long(), next);
            }
            "xoroshiro" => {
                let mut rng = noise_perlin::Xoroshiro::new(seed);
                assert_eq!(
                    digest(draws, || rng.next_double()),
                    expected,
                    "noise seed {seed}"
                );
                assert_eq!(rng.next_long() as i64, next);
                let mut rng = random::Xoroshiro::from_seed(seed as u64);
                assert_eq!(
                    digest(draws, || rng.next_f64()),
                    expected,
                    "feature seed {seed}"
                );
                assert_eq!(rng.next_i64(), next);
            }
            "worldgen_xoroshiro" => {
                let mut rng = simplex::WorldgenRandom::new(seed);
                assert_eq!(
                    digest(draws, || rng.next_double()),
                    expected,
                    "ore seed {seed}"
                );
                assert_eq!(rng.next_long(), next);
                let mut rng = random::WorldgenRandom::from_seed(seed as u64);
                assert_eq!(
                    digest(draws, || rng.next_f64()),
                    expected,
                    "tree seed {seed}"
                );
                assert_eq!(rng.next_i64(), next);
            }
            other => panic!("unknown native RNG: {other}"),
        }
    }
}
