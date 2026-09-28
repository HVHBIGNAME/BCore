//! Block-position biome lookup; noise-biome storage remains quart-resolution.
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
pub struct BiomeZoom {
    seed: i64,
}

impl BiomeZoom {
    pub fn new(world_seed: i64) -> Self {
        let digest = Sha256::digest(world_seed.to_le_bytes());
        Self {
            seed: i64::from_le_bytes(digest[..8].try_into().unwrap()),
        }
    }

    /// Return the selected noise-biome coordinates, before the world's Y clamp.
    pub fn quart_at(self, (x, y, z): (i32, i32, i32)) -> (i32, i32, i32) {
        let shifted = [x.wrapping_sub(2), y.wrapping_sub(2), z.wrapping_sub(2)];
        let base = shifted.map(|v| v >> 2);
        let fraction = shifted.map(|v| f64::from(v & 3) / 4.0);
        let mut best_distance = f64::INFINITY;
        let mut selected = base;
        for corner in 0..8 {
            let offsets = [(corner >> 2) & 1, (corner >> 1) & 1, corner & 1];
            let q = std::array::from_fn::<_, 3, _>(|i| base[i] + offsets[i]);
            let mut state = self.seed;
            for coordinate in [q[0], q[1], q[2], q[0], q[1], q[2]] {
                state = next(state, i64::from(coordinate));
            }
            let dx = fraction[0] - f64::from(offsets[0]) + fiddle(state);
            state = next(state, self.seed);
            let dy = fraction[1] - f64::from(offsets[1]) + fiddle(state);
            state = next(state, self.seed);
            let dz = fraction[2] - f64::from(offsets[2]) + fiddle(state);
            // Native summation order matters at equal-distance boundaries.
            let distance = dz * dz + dy * dy + dx * dx;
            if distance < best_distance {
                best_distance = distance;
                selected = q;
            }
        }
        (selected[0], selected[1], selected[2])
    }
}

fn next(state: i64, salt: i64) -> i64 {
    state
        .wrapping_mul(
            state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407),
        )
        .wrapping_add(salt)
}

fn fiddle(state: i64) -> f64 {
    (f64::from(((state >> 24) & 1023) as i32) / 1024.0 - 0.5) * 0.9
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_to_quart_selection_matches_native_manager() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../data/biome_zoom_26_1.json")).unwrap();
        for sample in fixture["samples"].as_array().unwrap() {
            let zoom = BiomeZoom::new(sample["seed"].as_i64().unwrap());
            assert_eq!(zoom.seed, sample["zoom_seed"].as_i64().unwrap());
            let center: Vec<i32> = sample["center"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as i32)
                .collect();
            let r = sample["radius"].as_i64().unwrap() as i32;
            let mut digest = md5::Context::new();
            for x in center[0] - r..=center[0] + r {
                for y in center[1] - r..=center[1] + r {
                    for z in center[2] - r..=center[2] + r {
                        let (qx, qy, qz) = zoom.quart_at((x, y, z));
                        digest.consume(qx.to_le_bytes());
                        digest.consume(qy.to_le_bytes());
                        digest.consume(qz.to_le_bytes());
                    }
                }
            }
            assert_eq!(
                format!("{:x}", digest.compute()),
                sample["quart_coordinates_md5"].as_str().unwrap(),
                "{sample}"
            );
        }
    }
}
