use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;

/// Surface-height grid probe, output format matching `scripts/bot/dump_terrain.js`
/// so a vanilla comparison can diff the two line-for-line.
///
/// Usage: surface_grid <center_x> <center_z> [region]
/// Prints one line per world Z, one `height:blockid` per world X column.
fn main() {
    let seed = 0x0BC0_0E00_1234_5678u64 as i64;
    let args: Vec<String> = std::env::args().collect();
    let cx: i32 = args[1].parse().expect("center x");
    let cz: i32 = args[2].parse().expect("center z");
    let region: i32 = args.get(3).map(|s| s.parse().expect("region")).unwrap_or(32);
    let generator = WorldGenerator::new(seed);
    let half = region / 2;

    for dz in -half..half {
        let mut row = String::new();
        for dx in -half..half {
            let wx = cx + dx;
            let wz = cz + dz;
            let ccx = wx.div_euclid(16);
            let ccz = wz.div_euclid(16);
            let lx = wx.rem_euclid(16) as usize;
            let lz = wz.rem_euclid(16) as usize;
            let chunk = generator.generate_chunk_vanilla(ChunkPos::new(ccx, ccz));
            let top = chunk.surface_y(lx, lz).unwrap_or(-1);
            let block = if top >= 0 {
                chunk.get(lx, top, lz).unwrap_or(0)
            } else {
                0
            };
            if dx != -half {
                row.push(' ');
            }
            row.push_str(&format!("{top}:{block}"));
        }
        println!("{row}");
    }
}
