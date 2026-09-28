//! Branching oak geometry. Float rounding follows the 26.1 placer operations.
use super::{place_below_trunk_block, FoliageAttachment, ShapeSink, TreeConfig, TreeRandom};

type Pos = (i32, i32, i32);

fn layer_radius(height: i32, layer: i32) -> Option<f32> {
    if (layer as f32) < height as f32 * 0.3 {
        return None;
    }
    let half = height as f32 / 2.0;
    let delta = half - layer as f32;
    let radius = if delta == 0.0 {
        half
    } else if delta.abs() >= half {
        0.0
    } else {
        ((half * half - delta * delta) as f64).sqrt() as f32
    };
    Some(radius * 0.5)
}

fn limb<P: ShapeSink + ?Sized>(
    chunk: &mut P,
    config: &TreeConfig,
    from: Pos,
    to: Pos,
    write: bool,
) -> bool {
    let delta = (to.0 - from.0, to.1 - from.1, to.2 - from.2);
    let steps = delta.0.abs().max(delta.1.abs()).max(delta.2.abs());
    if steps == 0 && !write {
        return true;
    }
    let divisor = steps.max(1) as f32;
    let increment = (
        delta.0 as f32 / divisor,
        delta.1 as f32 / divisor,
        delta.2 as f32 / divisor,
    );
    for step in 0..=steps {
        let dx = (0.5 + step as f32 * increment.0).floor() as i32;
        let dy = (0.5 + step as f32 * increment.1).floor() as i32;
        let dz = (0.5 + step as f32 * increment.2).floor() as i32;
        let (wx, y, wz) = (from.0 + dx, from.1 + dy, from.2 + dz);
        let pos = (wx, y, wz);
        let Some(state) = chunk.state(pos) else {
            continue;
        };
        if write {
            if chunk.valid_state(state) {
                let log = if dx == 0 && dz == 0 {
                    config.log
                } else if dx.abs() >= dz.abs() {
                    config.log - 1 // axis=x
                } else {
                    config.log + 1 // axis=z
                };
                chunk.log(pos, log);
            }
        } else if !chunk.free_state(state) {
            return false;
        }
    }
    true
}

pub(super) fn place_trunk<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    chunk: &mut P,
    random: &mut R,
    config: &TreeConfig,
    origin: Pos,
    height: i32,
    attachments: &mut Vec<FoliageAttachment>,
) {
    let (ox, oy, oz) = origin;
    let total_height = height + 2;
    let trunk_top = oy + (total_height as f64 * 0.618).floor() as i32;
    place_below_trunk_block(chunk, config, (ox, oy - 1, oz));
    let mut clusters = vec![((ox, oy + total_height - 5, oz), trunk_top)];
    // Vanilla's min(1, floor(1.382 + (height / 13)^2)) is one for tree heights.
    for layer in (0..=total_height - 5).rev() {
        let Some(radius) = layer_radius(total_height, layer) else {
            continue;
        };
        let reach = radius as f64 * (random.next_f32() as f64 + 0.328);
        let angle = (random.next_f32() * 2.0) as f64 * std::f64::consts::PI;
        let x = ox + (reach * angle.sin() + 0.5).floor() as i32;
        let z = oz + (reach * angle.cos() + 0.5).floor() as i32;
        let crown = (x, oy + layer - 1, z);
        if !limb(chunk, config, crown, (x, crown.1 + 5, z), false) {
            continue;
        }
        let dx = ox - x;
        let dz = oz - z;
        let slope_y = crown.1 as f64 - ((dx * dx + dz * dz) as f64).sqrt() * 0.381;
        let joint_y = if slope_y > trunk_top as f64 {
            trunk_top
        } else {
            slope_y as i32
        };
        if limb(chunk, config, (ox, joint_y, oz), crown, false) {
            clusters.push((crown, joint_y));
        }
    }
    limb(chunk, config, origin, (ox, trunk_top, oz), true);
    for &(crown, joint_y) in &clusters {
        if (joint_y - oy) as f64 >= total_height as f64 * 0.2 {
            let joint = (ox, joint_y, oz);
            if joint != crown {
                limb(chunk, config, joint, crown, true);
            }
            attachments.push(FoliageAttachment {
                x: crown.0,
                y: crown.1,
                z: crown.2,
                radius_offset: 0,
                double_trunk: false,
            });
        }
    }
}
