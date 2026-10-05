//! ProtoChunk light-property changes, native decrease/pull/increase queues.
use super::*;

fn update_flags(state: u32) -> FeatureResult<u8> {
    static FLAGS: OnceLock<Vec<u8>> = OnceLock::new();
    FLAGS
        .get_or_init(|| {
            #[derive(Deserialize)]
            struct Capture {
                state_count: usize,
                update_state_ranges: Vec<[usize; 3]>,
            }
            let data: Capture =
                serde_json::from_str(include_str!("../../data/lighting_updates_26_1.json"))
                    .expect("native light update flags");
            let mut flags = Vec::with_capacity(data.state_count);
            for [start, end, value] in data.update_state_ranges {
                assert_eq!(start, flags.len());
                assert!(end > start && end <= data.state_count && value <= 3);
                flags.resize(end, value as u8);
            }
            assert_eq!(flags.len(), data.state_count);
            flags
        })
        .get(state as usize)
        .copied()
        .ok_or_else(|| FeatureError::MissingData(format!("native light update flags for {state}")))
}

pub fn has_different_light_properties(before: u32, after: u32) -> FeatureResult<bool> {
    let a = state_light(before)?;
    let b = state_light(after)?;
    Ok(before != after
        && (a.dampening != b.dampening
            || a.emission != b.emission
            || update_flags(before)? & 1 != 0
            || update_flags(after)? & 1 != 0))
}

#[derive(Clone, Copy)]
struct Decrease {
    pos: Pos,
    level: u8,
    directions: u8,
}

impl GenerationLight {
    /// Apply ordered ProtoChunk writes and publish the resulting native light
    /// work. Invalid input is rejected before any block or light storage changes.
    /// Registration is required; uninitialized columns update blocks only.
    pub fn apply_block_updates(&mut self, writes: &[(Pos, u32)]) -> FeatureResult<()> {
        for &(pos, state) in writes {
            state_light(state)?;
            update_flags(state)?;
            if !self.columns.contains_key(&(pos.0 >> 4, pos.2 >> 4)) {
                return Err(FeatureError::MissingData(format!(
                    "light update column at {pos:?}"
                )));
            }
        }
        let mut checks = Vec::new();
        for &(pos, state) in writes {
            if !(self.min_y..self.min_y + self.height).contains(&pos.1) {
                continue;
            }
            let dy = pos.1 - self.min_y;
            let index = dy as usize * 256 + (pos.2 & 15) as usize * 16 + (pos.0 & 15) as usize;
            let column = self.columns.get_mut(&(pos.0 >> 4, pos.2 >> 4)).unwrap();
            let old = column.states[index];
            if old == state {
                continue;
            }
            let count = &mut column.section_counts[dy as usize / 16];
            let was_empty = *count == 0;
            *count = (*count as i32 + i32::from(!crate::is_air(state))
                - i32::from(!crate::is_air(old))) as u16;
            let now_empty = *count == 0;
            column.states[index] = state;
            if !column.initialized {
                continue;
            }
            if was_empty != now_empty {
                self.set_section_nonempty(Self::key(pos), !now_empty);
            }
            if has_different_light_properties(old, state)? {
                self.update_sky_source(pos)?;
                checks.push(pos);
            }
        }
        let checks = check_order(checks);
        for sky in [false, true] {
            if sky && !self.has_sky {
                continue;
            }
            let mut decrease = VecDeque::new();
            let mut increase = VecDeque::new();
            for &pos in &checks {
                if sky {
                    self.check_sky_node(pos, &mut decrease, &mut increase);
                } else {
                    self.check_block_node(pos, &mut decrease, &mut increase)?;
                }
            }
            self.propagate_decreases(&mut decrease, &mut increase, sky)?;
            self.propagate(&mut increase, sky)?;
        }
        for key in std::mem::take(&mut self.removed_sections) {
            self.sections.remove(&key);
            self.changed_columns.insert((key.0, key.2));
        }
        Ok(())
    }

    pub(super) fn set_section_nonempty(&mut self, key: SectionKey, nonempty: bool) {
        if nonempty {
            if !self.nonempty_sections.insert(key) {
                return;
            }
            self.removed_sections.remove(&key);
            self.initialize_section(key);
        } else if !self.nonempty_sections.remove(&key) {
            return;
        }
        let (x, y, z) = key;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let p = (x + dx, y + dy, z + dz);
                    if nonempty {
                        self.removed_sections.remove(&p);
                        self.initialize_section(p);
                    } else if !(-1..=1).any(|a| {
                        (-1..=1).any(|b| {
                            (-1..=1).any(|c| {
                                self.nonempty_sections
                                    .contains(&(p.0 + a, p.1 + b, p.2 + c))
                            })
                        })
                    }) {
                        self.removed_sections.insert(p);
                    }
                }
            }
        }
    }

    pub(super) fn set_stored_level(&mut self, pos: Pos, sky: bool, value: u8) {
        self.changed_columns.insert((pos.0 >> 4, pos.2 >> 4));
        let section = self
            .sections
            .get_mut(&Self::key(pos))
            .expect("stored light section");
        if sky {
            section.sky[Self::index(pos)] = value;
            section.sky_empty = false;
        } else {
            section.block[Self::index(pos)] = value;
            section.block_empty = false;
        }
    }

    fn update_sky_source(&mut self, pos: Pos) -> FeatureResult<()> {
        let original = self.lowest_source(pos.0, pos.2).max(self.min_y - 1);
        if pos.1 + 1 < original {
            return Ok(());
        }
        for upper_y in [pos.1 + 1, pos.1] {
            let above = self.state((pos.0, upper_y, pos.2));
            let below = self.state((pos.0, upper_y - 1, pos.2));
            let blocked =
                state_light(below)?.dampening != 0 || face_occludes(above, below, Direction::Down)?;
            let next = if blocked && upper_y > original {
                Some(upper_y)
            } else if !blocked && upper_y == original {
                let mut from = below;
                let mut lowest = i32::MIN;
                for y in (self.min_y - 1..upper_y - 1).rev() {
                    let to = self.state((pos.0, y, pos.2));
                    if state_light(to)?.dampening != 0 || face_occludes(from, to, Direction::Down)?
                    {
                        lowest = y + 1;
                        break;
                    }
                    from = to;
                }
                Some(lowest)
            } else {
                None
            };
            if let Some(next) = next {
                self.columns
                    .get_mut(&(pos.0 >> 4, pos.2 >> 4))
                    .unwrap()
                    .sources
                    .lowest_source_y[((pos.2 & 15) * 16 + (pos.0 & 15)) as usize] = next;
                break;
            }
        }
        Ok(())
    }

    fn emission(&self, pos: Pos) -> FeatureResult<u8> {
        if self.enabled.contains(&(pos.0 >> 4, pos.2 >> 4)) {
            Ok(state_light(self.state(pos))?.emission)
        } else {
            Ok(0)
        }
    }

    fn emission_entry(&self, pos: Pos, level: u8) -> FeatureResult<Increase> {
        Ok(Increase {
            pos,
            level,
            directions: 63,
            from_empty: update_flags(self.state(pos))? & 2 != 0,
            from_emission: true,
        })
    }

    fn check_block_node(
        &mut self,
        pos: Pos,
        decrease: &mut VecDeque<Decrease>,
        increase: &mut VecDeque<Increase>,
    ) -> FeatureResult<()> {
        if !self.sections.contains_key(&Self::key(pos)) {
            return Ok(());
        }
        let emission = self.emission(pos)?;
        let old = self.block_brightness(pos);
        if emission < old {
            self.set_stored_level(pos, false, 0);
            decrease.push_back(Decrease {
                pos,
                level: old,
                directions: 63,
            });
        } else {
            decrease.push_back(Decrease {
                pos,
                level: 1,
                directions: 63,
            });
        }
        if emission > 0 {
            increase.push_back(self.emission_entry(pos, emission)?);
        }
        Ok(())
    }

    fn check_sky_node(
        &mut self,
        pos: Pos,
        decrease: &mut VecDeque<Decrease>,
        increase: &mut VecDeque<Increase>,
    ) {
        let lowest = if self.enabled.contains(&(pos.0 >> 4, pos.2 >> 4)) {
            self.lowest_source(pos.0, pos.2)
        } else {
            i32::MAX
        };
        if lowest != i32::MAX {
            self.update_sources_in_column(pos.0, pos.2, lowest, decrease, increase);
        }
        if !self.sections.contains_key(&Self::key(pos)) {
            return;
        }
        if pos.1 >= lowest {
            decrease.push_back(Decrease {
                pos,
                level: 15,
                directions: 61,
            });
            increase.push_back(Increase {
                pos,
                level: 15,
                directions: 61,
                from_empty: false,
                from_emission: false,
            });
        } else {
            let old = self.sky_brightness(pos);
            if old > 0 {
                self.set_stored_level(pos, true, 0);
                decrease.push_back(Decrease {
                    pos,
                    level: old,
                    directions: 63,
                });
            } else {
                decrease.push_back(Decrease {
                    pos,
                    level: 1,
                    directions: 63,
                });
            }
        }
    }

    fn update_sources_in_column(
        &mut self,
        x: i32,
        z: i32,
        lowest: i32,
        decrease: &mut VecDeque<Decrease>,
        increase: &mut VecDeque<Increase>,
    ) {
        let Some(bottom) = self.sections.keys().map(|p| p.1 * 16).min() else {
            return;
        };
        if lowest > bottom {
            for y in (bottom..lowest).rev() {
                let pos = (x, y, z);
                if !self.sections.contains_key(&Self::key(pos)) {
                    continue;
                }
                if self.sky_brightness(pos) != 15 {
                    break;
                }
                self.set_stored_level(pos, true, 0);
                decrease.push_back(Decrease {
                    pos,
                    level: 15,
                    directions: if y == lowest - 1 { 63 } else { 61 },
                });
            }
        }
        let neighbour_lowest = [
            self.lowest_source(x - 1, z),
            self.lowest_source(x + 1, z),
            self.lowest_source(x, z - 1),
            self.lowest_source(x, z + 1),
        ]
        .into_iter()
        .max()
        .unwrap();
        let Some(top) = self
            .sections
            .keys()
            .filter(|p| p.0 == x >> 4 && p.2 == z >> 4)
            .map(|p| p.1 * 16 + 15)
            .max()
        else {
            return;
        };
        for y in lowest.max(bottom)..=top {
            let pos = (x, y, z);
            if !self.sections.contains_key(&Self::key(pos)) {
                continue;
            }
            if self.sky_brightness(pos) == 15 {
                break;
            }
            self.set_stored_level(pos, true, 15);
            if y < neighbour_lowest || y == lowest {
                increase.push_back(Increase {
                    pos,
                    level: 15,
                    directions: 61,
                    from_empty: false,
                    from_emission: false,
                });
            }
        }
    }

    fn propagate_decreases(
        &mut self,
        decrease: &mut VecDeque<Decrease>,
        increase: &mut VecDeque<Increase>,
        sky: bool,
    ) -> FeatureResult<()> {
        while let Some(Decrease {
            pos,
            level,
            directions,
        }) = decrease.pop_front()
        {
            let skipped = if sky {
                self.empty_sections_below(pos)
            } else {
                0
            };
            for direction in Direction::ALL {
                if directions & (1 << direction as u8) == 0 {
                    continue;
                }
                let next = direction.step(pos);
                if !self.sections.contains_key(&Self::key(next)) {
                    continue;
                }
                let old = if sky {
                    self.sky_brightness(next)
                } else {
                    self.block_brightness(next)
                };
                if old == 0 {
                    continue;
                }
                let back = 1 << (direction as u8 ^ 1);
                if old < level {
                    let emission = if sky { 0 } else { self.emission(next)? };
                    self.set_stored_level(next, sky, 0);
                    if sky || emission < old {
                        decrease.push_back(Decrease {
                            pos: next,
                            level: old,
                            directions: 63 ^ back,
                        });
                    }
                    if emission > 0 {
                        increase.push_back(self.emission_entry(next, emission)?);
                    }
                    if sky {
                        self.decrease_empty_sections(next, direction, old, skipped, decrease);
                    }
                } else {
                    increase.push_back(Increase {
                        pos: next,
                        level: old,
                        directions: back,
                        from_empty: false,
                        from_emission: false,
                    });
                }
            }
        }
        Ok(())
    }

    fn decrease_empty_sections(
        &mut self,
        pos: Pos,
        direction: Direction,
        level: u8,
        count: i32,
        queue: &mut VecDeque<Decrease>,
    ) {
        let crossed = match direction {
            Direction::North => pos.2 & 15 == 15,
            Direction::South => pos.2 & 15 == 0,
            Direction::West => pos.0 & 15 == 15,
            Direction::East => pos.0 & 15 == 0,
            _ => false,
        };
        if !crossed {
            return;
        }
        let (cx, sy, cz) = Self::key(pos);
        for section in (sy - count..sy).rev() {
            if !self.sections.contains_key(&(cx, section, cz)) {
                continue;
            }
            for y in (section * 16..section * 16 + 16).rev() {
                let p = (pos.0, y, pos.2);
                self.set_stored_level(p, true, 0);
                queue.push_back(Decrease {
                    pos: p,
                    level,
                    directions: 63 ^ (1 << (direction as u8 ^ 1)),
                });
            }
        }
    }
}

/// LightEngine uses LongOpenHashSet(512, .5), cleared/trimmed after each drain.
fn check_order(positions: Vec<Pos>) -> Vec<Pos> {
    fn insert(slots: &mut [u64], key: u64) -> bool {
        let h = key.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let h = h ^ (h >> 32);
        let mut i = (h ^ (h >> 16)) as usize & (slots.len() - 1);
        while slots[i] != 0 {
            if slots[i] == key {
                return false;
            }
            i = (i + 1) & (slots.len() - 1);
        }
        slots[i] = key;
        true
    }
    let mut slots = vec![0; 1024];
    let (mut zero, mut size) = (false, 0);
    for (x, y, z) in positions {
        let key =
            ((x as u64 & 0x3ff_ffff) << 38) | ((z as u64 & 0x3ff_ffff) << 12) | (y as u64 & 4095);
        if key == 0 {
            if zero {
                continue;
            }
            zero = true;
        } else if !insert(&mut slots, key) {
            continue;
        }
        size += 1;
        if size > slots.len() / 2 {
            let mut bigger = vec![0; slots.len() * 2];
            for value in slots.into_iter().rev().filter(|&v| v != 0) {
                insert(&mut bigger, value);
            }
            slots = bigger;
        }
    }
    let mut result = Vec::with_capacity(size);
    if zero {
        result.push((0, 0, 0));
    }
    result.extend(slots.into_iter().rev().filter(|&v| v != 0).map(|v| {
        (
            (v as i64 >> 38) as i32,
            ((v << 52) as i64 >> 52) as i32,
            ((v << 26) as i64 >> 38) as i32,
        )
    }));
    result
}
