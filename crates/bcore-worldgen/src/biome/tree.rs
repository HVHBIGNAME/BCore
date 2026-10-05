// SPDX-License-Identifier: MIT
//! Independent implementation of the 26.1 Climate.RTree behavior measured by
//! BiomeTreeReference. Construction order is part of the result on equal distances.
use super::{BiomeId, BiomeParameters, ClimateRange};
use std::cmp::Ordering;

const DIMENSIONS: usize = 7;
const FANOUT: usize = 6;
type Space = [ClimateRange; DIMENSIONS];

#[derive(Clone, Copy)]
pub(super) struct Node {
    pub(super) space: Space,
    /// Leaf: original row index. Branch: first child in the arena.
    pub(super) first: u32,
    pub(super) count: u32,
}

impl Node {
    #[inline]
    pub(super) fn distance(&self, target: &[i64; DIMENSIONS]) -> i64 {
        self.space
            .iter()
            .zip(target)
            .fold(0_i64, |sum, (&range, &value)| {
                let distance = range.distance(value);
                sum.wrapping_add(distance.wrapping_mul(distance))
            })
    }
}

pub(super) struct Tree {
    pub(super) nodes: Box<[Node]>,
}

impl Tree {
    pub(super) fn new(rows: &[(BiomeId, BiomeParameters)]) -> Self {
        assert!(!rows.is_empty(), "nonempty biome parameter list");
        assert!(rows.len() <= i32::MAX as usize, "native list size limit");
        let spaces: Vec<_> = rows.iter().map(|(_, p)| p.space()).collect();
        let mut order: Vec<_> = (0..rows.len()).collect();
        let mut nodes = Vec::with_capacity(rows.len() + rows.len() / 3 + 1);
        nodes.push(Node {
            space: spaces[0],
            first: 0,
            count: 0,
        });
        build(&spaces, &mut order, &mut nodes, 0);
        Self {
            nodes: nodes.into_boxed_slice(),
        }
    }

    pub(super) fn search(&self, target: [i64; 6], last: Option<usize>) -> usize {
        if self.nodes[0].count == 0 {
            return 0;
        }
        let target = [
            target[0], target[1], target[2], target[3], target[4], target[5], 0,
        ];
        let mut best = last;
        let mut distance = last.map_or(i64::MAX, |index| self.nodes[index].distance(&target));
        self.visit(0, &target, &mut best, &mut distance);
        best.expect("native climate search returned no leaf")
    }

    fn visit(
        &self,
        branch: usize,
        target: &[i64; DIMENSIONS],
        best: &mut Option<usize>,
        best_distance: &mut i64,
    ) {
        let node = self.nodes[branch];
        let start = node.first as usize;
        for (offset, child) in self.nodes[start..start + node.count as usize]
            .iter()
            .enumerate()
        {
            let distance = child.distance(target);
            if distance < *best_distance {
                let index = start + offset;
                if child.count == 0 {
                    *best = Some(index);
                    *best_distance = distance;
                } else {
                    // Passing the incumbent through recursion preserves native strict
                    // replacement, including overflow and zero-distance candidates.
                    self.visit(index, target, best, best_distance);
                }
            }
        }
    }

    pub(super) fn row(&self, leaf: usize) -> usize {
        self.nodes[leaf].first as usize
    }
}

fn center(range: ClimateRange) -> i64 {
    // Division truncates toward zero, including odd negative endpoint sums.
    range.min.wrapping_add(range.max) / 2
}

fn compare(a: &Space, b: &Space, axis: usize, absolute: bool) -> Ordering {
    for offset in 0..DIMENSIONS {
        let dimension = (axis + offset) % DIMENSIONS;
        let mut left = center(a[dimension]);
        let mut right = center(b[dimension]);
        if absolute {
            left = left.wrapping_abs();
            right = right.wrapping_abs();
        }
        let ordering = left.cmp(&right);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

fn bounds(spaces: &[Space], rows: &[usize]) -> Space {
    let mut result = spaces[rows[0]];
    for &row in &rows[1..] {
        for (range, next) in result.iter_mut().zip(spaces[row]) {
            range.min = range.min.min(next.min);
            range.max = range.max.max(next.max);
        }
    }
    result
}

fn cost(space: &Space) -> i64 {
    space.iter().fold(0_i64, |sum, range| {
        sum.wrapping_add(range.max.wrapping_sub(range.min).wrapping_abs())
    })
}

fn bucket_size(count: usize) -> usize {
    // For Java's positive int list lengths this is exactly
    // (int) pow(6, floor(log(count - 0.01) / log(6))).
    let mut size = 1;
    while size <= (count - 1) / FANOUT {
        size *= FANOUT;
    }
    size
}

fn reserve_children(nodes: &mut Vec<Node>, parent: usize, space: Space, count: usize) -> usize {
    let start = nodes.len();
    let empty = Node {
        space,
        first: 0,
        count: 0,
    };
    nodes.resize(start + count, empty);
    nodes[parent] = Node {
        space,
        first: start as u32,
        count: count as u32,
    };
    start
}

fn build(spaces: &[Space], rows: &mut [usize], nodes: &mut Vec<Node>, parent: usize) {
    if rows.len() == 1 {
        nodes[parent] = Node {
            space: spaces[rows[0]],
            first: rows[0] as u32,
            count: 0,
        };
        return;
    }
    if rows.len() <= FANOUT {
        rows.sort_by_key(|&row| {
            spaces[row].iter().fold(0_i64, |sum, &range| {
                sum.wrapping_add(center(range).wrapping_abs())
            })
        });
        let start = reserve_children(nodes, parent, bounds(spaces, rows), rows.len());
        for (index, &row) in rows.iter().enumerate() {
            nodes[start + index] = Node {
                space: spaces[row],
                first: row as u32,
                count: 0,
            };
        }
        return;
    }

    let size = bucket_size(rows.len());
    let mut lowest_cost = i64::MAX;
    let mut split_axis = 0;
    let mut chosen = Vec::new();
    let mut buckets = Vec::new();
    for axis in 0..DIMENSIONS {
        rows.sort_by(|&a, &b| compare(&spaces[a], &spaces[b], axis, false));
        let candidate: Vec<_> = rows
            .chunks(size)
            .map(|chunk| bounds(spaces, chunk))
            .collect();
        let total = candidate
            .iter()
            .fold(0_i64, |sum, space| sum.wrapping_add(cost(space)));
        if total < lowest_cost {
            lowest_cost = total;
            split_axis = axis;
            // Keep this snapshot. Later stable sorts must not alter the winner.
            chosen = rows.to_vec();
            buckets = candidate;
        }
    }
    assert!(
        !chosen.is_empty(),
        "native climate construction found no split"
    );
    let mut bucket_order: Vec<_> = (0..buckets.len()).collect();
    bucket_order.sort_by(|&a, &b| compare(&buckets[a], &buckets[b], split_axis, true));
    let start = reserve_children(nodes, parent, bounds(spaces, &chosen), buckets.len());
    for (index, bucket) in bucket_order.into_iter().enumerate() {
        let end = ((bucket + 1) * size).min(chosen.len());
        build(
            spaces,
            &mut chosen[bucket * size..end],
            nodes,
            start + index,
        );
    }
}
