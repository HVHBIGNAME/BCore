//! Dump ore/blob placement positions for a chunk, for diffing against vanilla.
use bcore_worldgen::features;
use std::cell::RefCell;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed: i64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1234);
    let cx: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(-34);
    let cz: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);

    let placed = RefCell::new(Vec::<(i32, i32, i32, u32)>::new());
    let ocean_floor = |_wx: i32, _wz: i32| 100; // diagnostic: everything below 100 fits
    features::place_ore_veins(seed, cx, cz, &ocean_floor, &mut |x, y, z, state| {
        placed.borrow_mut().push((x, y, z, state));
    });
    let mut v = placed.into_inner();
    v.sort();
    for (x, y, z, state) in v {
        println!("{x},{y},{z},{state}");
    }
}
