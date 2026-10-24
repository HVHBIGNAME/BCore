//! Validated saved light inputs and boundary transfer into freshly lit columns.
use super::*;

pub(crate) struct SavedLightColumn {
    pos: ChunkPos,
    column: Column,
    light: Option<ChunkLight>,
}

impl GenerationLight {
    pub(crate) fn prepare_saved_chunk(
        &self,
        pos: ChunkPos,
        states: &[u32],
        light: Option<&ChunkLight>,
    ) -> FeatureResult<SavedLightColumn> {
        self.validate_chunk_inputs(pos, states)?;
        if light.is_some_and(|light| !light.validate(self.min_y, self.height, self.has_sky)) {
            return Err(FeatureError::InvalidConfig("saved light geometry".into()));
        }
        Ok(SavedLightColumn {
            pos,
            column: Column {
                states: states.to_vec(),
                section_counts: section_counts(states),
                sources: if light.is_some() {
                    sky_light_sources(self.min_y, states)?
                } else {
                    SkyLightSources {
                        lowest_source_y: vec![i32::MIN; 256],
                    }
                },
                initialized: light.is_some(),
            },
            light: light.cloned(),
        })
    }

    pub(crate) fn admit_saved_chunk(&mut self, saved: SavedLightColumn) {
        let SavedLightColumn { pos, column, light } = saved;
        let counts = column.section_counts.clone();
        self.columns.insert((pos.x, pos.z), column);
        let Some(light) = light else { return };
        self.enabled.insert((pos.x, pos.z));
        for (index, count) in counts.into_iter().enumerate() {
            if count != 0 {
                self.set_section_nonempty((pos.x, self.min_y / 16 + index as i32, pos.z), true);
            }
        }
        for (index, section) in light.sections.into_iter().enumerate() {
            let key = (pos.x, light.min_section_y + index as i32, pos.z);
            if section.sky.is_none() && section.block.is_none() {
                self.sections.remove(&key);
                continue;
            }
            let unpack = |values: Option<Vec<u8>>| {
                values.map_or_else(
                    || vec![0; 4096],
                    |bytes| {
                        bytes
                            .into_iter()
                            .flat_map(|byte| [byte & 15, byte >> 4])
                            .collect()
                    },
                )
            };
            self.sections.insert(
                key,
                StoredSection {
                    sky_present: section.sky.is_some(),
                    block_present: section.block.is_some(),
                    sky: unpack(section.sky),
                    block: unpack(section.block),
                    sky_empty: section.sky_empty,
                    block_empty: section.block_empty,
                },
            );
        }
        self.changed_columns.insert((pos.x, pos.z));
    }

    pub(super) fn queue_neighbour_light(
        &self,
        pos: ChunkPos,
        block: &mut VecDeque<Increase>,
        sky: &mut VecDeque<Increase>,
    ) {
        for (dx, dz, direction) in [
            (-1, 0, Direction::East),
            (1, 0, Direction::West),
            (0, -1, Direction::South),
            (0, 1, Direction::North),
        ] {
            if !self.enabled.contains(&(pos.x + dx, pos.z + dz)) {
                continue;
            }
            for y in self.min_y - 16..self.min_y + self.height + 16 {
                for offset in 0..16 {
                    let source = (
                        pos.x * 16
                            + if dx < 0 {
                                -1
                            } else if dx > 0 {
                                16
                            } else {
                                offset
                            },
                        y,
                        pos.z * 16
                            + if dz < 0 {
                                -1
                            } else if dz > 0 {
                                16
                            } else {
                                offset
                            },
                    );
                    if !self
                        .sections
                        .contains_key(&Self::key(direction.step(source)))
                    {
                        continue;
                    }
                    for (queue, level) in [
                        (&mut *block, self.block_brightness(source)),
                        (
                            &mut *sky,
                            if self.has_sky {
                                self.sky_brightness(source)
                            } else {
                                0
                            },
                        ),
                    ] {
                        if level > 1 {
                            queue.push_back(Increase {
                                pos: source,
                                level,
                                directions: 1 << direction as u8,
                                from_empty: false,
                                from_emission: false,
                            });
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_layers_preserve_presence_empty_flags_and_nibble_order() {
        let pos = ChunkPos::new(-3, 2);
        let mut states = vec![crate::block::AIR; crate::WORLD_HEIGHT as usize * 256];
        states[0] = crate::block::STONE;
        let mut snapshot = ChunkLight {
            min_section_y: -5,
            sections: vec![LightSection::default(); crate::WORLD_HEIGHT as usize / 16 + 2],
        };
        snapshot.sections[1].sky = Some(vec![0; 2048]);
        snapshot.sections[1].sky_empty = true;
        snapshot.sections[2].block = Some(vec![0x7c; 2048]);
        snapshot.sections[3].sky = Some(vec![0xff; 2048]);
        let mut engine = GenerationLight::default();
        let input = engine
            .prepare_saved_chunk(pos, &states, Some(&snapshot))
            .unwrap();
        engine.admit_saved_chunk(input);
        assert!(engine.is_initialized(pos));
        assert!(engine.is_enabled(pos));
        assert_eq!(engine.chunk_light(pos), snapshot);
        assert_eq!(engine.block_brightness((-48, -48, 32)), 12);
        assert_eq!(engine.block_brightness((-47, -48, 32)), 7);
        assert_eq!(engine.sky_brightness((-48, -48, 32)), 15);
        let mut invalid = snapshot.clone();
        invalid.sections.pop();
        assert!(engine
            .prepare_saved_chunk(pos, &states, Some(&invalid))
            .is_err());
        assert_eq!(engine.chunk_light(pos), snapshot);
    }

    #[test]
    fn absent_saved_light_registers_blocks_without_enabling_or_fabricating_layers() {
        let pos = ChunkPos::new(2, -1);
        let states = vec![crate::block::STONE; crate::WORLD_HEIGHT as usize * 256];
        let mut engine = GenerationLight::default();
        let input = engine.prepare_saved_chunk(pos, &states, None).unwrap();
        engine.admit_saved_chunk(input);
        assert_eq!(engine.state((32, 0, -16)), crate::block::STONE);
        assert!(!engine.is_initialized(pos));
        assert!(!engine.is_enabled(pos));
        assert!(engine
            .chunk_light(pos)
            .sections
            .iter()
            .all(|section| { section.sky.is_none() && section.block.is_none() }));
    }

    #[test]
    fn incoming_sky_does_not_replace_an_absent_layer_whose_inherited_light_is_brighter() {
        let target = ChunkPos::new(0, 0);
        let source = ChunkPos::new(1, 0);
        let states = vec![crate::block::AIR; crate::WORLD_HEIGHT as usize * 256];
        let mut snapshot = ChunkLight {
            min_section_y: -5,
            sections: vec![LightSection::default(); crate::WORLD_HEIGHT as usize / 16 + 2],
        };
        snapshot.sections[1].block = Some(vec![0; 2048]);
        snapshot.sections[2].sky = Some(vec![0xff; 2048]);
        let mut source_light = snapshot.clone();
        source_light.sections[1].sky = Some(vec![0xff; 2048]);
        let mut engine = GenerationLight::default();
        let input = engine
            .prepare_saved_chunk(target, &states, Some(&snapshot))
            .unwrap();
        engine.admit_saved_chunk(input);
        let input = engine
            .prepare_saved_chunk(source, &states, Some(&source_light))
            .unwrap();
        engine.admit_saved_chunk(input);
        assert_eq!(engine.sky_brightness((15, MIN_Y, 0)), 15);
        let mut incoming = VecDeque::from([Increase {
            pos: (16, MIN_Y, 0),
            level: 15,
            directions: 1 << Direction::West as u8,
            from_empty: false,
            from_emission: false,
        }]);
        engine.propagate(&mut incoming, true).unwrap();
        assert_eq!(engine.sky_brightness((15, MIN_Y, 0)), 15);
        assert_eq!(engine.chunk_light(target), snapshot);
    }
}
