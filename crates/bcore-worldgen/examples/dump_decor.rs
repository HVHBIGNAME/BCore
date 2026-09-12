//! Dump the decoration seed + feature seed + count + in_square positions for a
//! chunk's forest tree feature WITHOUT intervening tree/decorator draws.
//! Only the first attempt predicts a real placement; this is not a tree-origin oracle.
use bcore_worldgen::feature_sorter::sorter;
use bcore_worldgen::random::WorldgenRandom;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let seed: i64 = args[1].parse().expect("seed");
    let cx: i32 = args[2].parse().expect("chunk x");
    let cz: i32 = args[3].parse().expect("chunk z");
    let base_x = cx * 16;
    let base_z = cz * 16;

    let mut random = WorldgenRandom::from_seed(0);
    let decoration_seed = random.set_decoration_seed(seed, base_x, base_z);
    println!("decoration_seed = {}", decoration_seed);

    let s = sorter();
    let (step, index) = s
        .within_step_index("trees_birch_and_oak_leaf_litter")
        .expect("forest feature missing");
    println!("forest feature: step={step} index={index}");

    random.set_feature_seed(decoration_seed, index as i32, step as i32);
    // count: weighted 10 (w9) / 11 (w1)
    let count = if random.next_i32_bounded(10) < 9 {
        10
    } else {
        11
    };
    println!("count = {count}");
    print!("in_square xz: ");
    for _ in 0..count {
        let x = base_x + random.next_i32_bounded(16);
        let z = base_z + random.next_i32_bounded(16);
        print!("({x},{z}) ");
    }
    println!();
}
