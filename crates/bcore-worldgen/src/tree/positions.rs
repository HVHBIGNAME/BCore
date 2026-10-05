//! Native HashSet traversal, including tree bins and iterator removal. Leaf
//! propagation interleaves removal and insertion, so rebuilding a bin after
//! removal would change the next root-to-front move.
use super::Pos;

pub(super) struct Positions {
    bins: Vec<Bin>,
    pub(super) len: usize,
}

fn hash((x, y, z): Pos) -> usize {
    let h = y
        .wrapping_add(z.wrapping_mul(31))
        .wrapping_mul(31)
        .wrapping_add(x) as u32;
    (h ^ (h >> 16)) as usize
}

impl Positions {
    pub(super) fn new() -> Self {
        Self {
            bins: (0..16).map(|_| Bin::default()).collect(),
            len: 0,
        }
    }
    pub(super) fn contains(&self, pos: Pos) -> bool {
        self.bins[hash(pos) & (self.bins.len() - 1)]
            .entries
            .contains(&pos)
    }
    pub(super) fn insert(&mut self, pos: Pos) {
        let capacity = self.bins.len();
        let index = hash(pos) & (capacity - 1);
        if self.bins[index].entries.contains(&pos) {
            return;
        }
        self.bins[index].insert(pos, capacity >= 64);
        self.len += 1;
        if self.len > capacity * 3 / 4 || (capacity < 64 && self.bins[index].entries.len() > 8) {
            let old = std::mem::replace(
                &mut self.bins,
                (0..capacity * 2).map(|_| Bin::default()).collect(),
            );
            for (index, bin) in old.into_iter().enumerate() {
                let (lo, hi) = bin.split(capacity);
                self.bins[index] = lo;
                self.bins[index + capacity] = hi;
            }
        }
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = Pos> + '_ {
        self.bins.iter().flat_map(|bin| bin.entries.iter().copied())
    }
    pub(super) fn pop(&mut self) -> Option<Pos> {
        let bin = self.bins.iter_mut().find(|bin| !bin.entries.is_empty())?;
        let pos = bin.entries.remove(0);
        if bin.entries.is_empty() {
            bin.tree = None;
        } else if let Some(tree) = &mut bin.tree {
            tree.remove(pos);
        }
        self.len -= 1;
        Some(pos)
    }
    pub(super) fn sorted_y(&self) -> Vec<Pos> {
        let mut result: Vec<_> = self.iter().collect();
        result.sort_by_key(|p| p.1);
        result
    }
}

#[derive(Default)]
struct Bin {
    entries: Vec<Pos>,
    tree: Option<Tree>,
}
impl Bin {
    fn insert(&mut self, pos: Pos, treeify: bool) {
        if let Some(tree) = &mut self.tree {
            let parent = tree.insert(pos).expect("nonempty position tree");
            let index = self
                .entries
                .iter()
                .position(|p| *p == parent)
                .expect("tree parent in bin");
            self.entries.insert(index + 1, pos);
        } else {
            self.entries.push(pos);
            if treeify && self.entries.len() > 8 {
                self.tree = Some(Tree::build(&self.entries));
            }
        }
        self.move_root_first();
    }
    fn move_root_first(&mut self) {
        if let Some(tree) = &self.tree {
            let root = tree.nodes[tree.root].pos;
            let index = self
                .entries
                .iter()
                .position(|p| *p == root)
                .expect("tree root in bin");
            if index != 0 {
                self.entries.remove(index);
                self.entries.insert(0, root);
            }
        }
    }
    fn split(mut self, bit: usize) -> (Self, Self) {
        let lo = self.entries.iter().filter(|p| hash(**p) & bit == 0).count();
        let hi = self.entries.len() - lo;
        if lo == 0 || hi == 0 {
            if self.entries.len() <= 6 {
                self.tree = None;
            }
            return if hi == 0 {
                (self, Self::default())
            } else {
                (Self::default(), self)
            };
        }
        let was_tree = self.tree.is_some();
        let (lo, hi) = self
            .entries
            .into_iter()
            .partition::<Vec<_>, _>(|p| hash(*p) & bit == 0);
        let rebuild = |entries: Vec<Pos>| {
            let tree = (was_tree && entries.len() > 6).then(|| Tree::build(&entries));
            let mut bin = Self { entries, tree };
            bin.move_root_first();
            bin
        };
        (rebuild(lo), rebuild(hi))
    }
}

struct Node {
    pos: Pos,
    hash: i32,
    parent: Option<usize>,
    children: [Option<usize>; 2],
    red: bool,
    active: bool,
}
struct Tree {
    nodes: Vec<Node>,
    root: usize,
}
impl Tree {
    fn build(positions: &[Pos]) -> Self {
        let mut tree = Self {
            nodes: Vec::new(),
            root: 0,
        };
        for &pos in positions {
            tree.insert(pos);
        }
        tree
    }
    fn insert(&mut self, pos: Pos) -> Option<Pos> {
        let hash = hash(pos) as i32;
        let mut parent = None;
        let mut side = 0;
        let mut cursor = (!self.nodes.is_empty()).then_some(self.root);
        while let Some(index) = cursor {
            assert_ne!(
                hash, self.nodes[index].hash,
                "unsupported native BlockPos identity-hash tie: {pos:?} and {:?}",
                self.nodes[index].pos
            );
            parent = Some(index);
            side = usize::from(hash > self.nodes[index].hash);
            cursor = self.nodes[index].children[side];
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            pos,
            hash,
            parent,
            children: [None, None],
            red: true,
            active: true,
        });
        if let Some(parent) = parent {
            self.nodes[parent].children[side] = Some(index);
        } else {
            self.root = index;
        }
        self.balance_insert(index);
        parent.map(|p| self.nodes[p].pos)
    }
    fn is_red(&self, index: Option<usize>) -> bool {
        index.is_some_and(|i| self.nodes[i].red)
    }
    fn rotate(&mut self, index: usize, side: usize) {
        let pivot = self.nodes[index].children[side].expect("rotation child");
        let middle = self.nodes[pivot].children[side ^ 1];
        self.nodes[index].children[side] = middle;
        if let Some(middle) = middle {
            self.nodes[middle].parent = Some(index);
        }
        let parent = self.nodes[index].parent;
        self.nodes[pivot].parent = parent;
        if let Some(parent) = parent {
            let slot = usize::from(self.nodes[parent].children[1] == Some(index));
            self.nodes[parent].children[slot] = Some(pivot);
        } else {
            self.root = pivot;
        }
        self.nodes[pivot].children[side ^ 1] = Some(index);
        self.nodes[index].parent = Some(pivot);
    }
    fn balance_insert(&mut self, mut index: usize) {
        while let Some(mut parent) = self.nodes[index].parent {
            if !self.nodes[parent].red {
                break;
            }
            let Some(mut grandparent) = self.nodes[parent].parent else {
                break;
            };
            let side = usize::from(self.nodes[grandparent].children[1] == Some(parent));
            let uncle = self.nodes[grandparent].children[side ^ 1];
            if let Some(uncle) = uncle.filter(|i| self.nodes[*i].red) {
                self.nodes[parent].red = false;
                self.nodes[uncle].red = false;
                self.nodes[grandparent].red = true;
                index = grandparent;
                continue;
            }
            if self.nodes[parent].children[side ^ 1] == Some(index) {
                index = parent;
                self.rotate(index, side ^ 1);
                parent = self.nodes[index].parent.expect("rotated parent");
                grandparent = self.nodes[parent].parent.expect("rotated grandparent");
            }
            self.nodes[parent].red = false;
            self.nodes[grandparent].red = true;
            self.rotate(grandparent, side);
            break;
        }
        self.nodes[self.root].red = false;
    }
    fn remove(&mut self, pos: Pos) {
        let mut index = self
            .nodes
            .iter()
            .position(|n| n.active && n.pos == pos)
            .expect("position in tree");
        if self.nodes[index].children.iter().all(Option::is_some) {
            let mut next = self.nodes[index].children[1].unwrap();
            while let Some(left) = self.nodes[next].children[0] {
                next = left;
            }
            let replacement = self.nodes[next].pos;
            self.nodes[next].pos = pos;
            self.nodes[next].hash = self.nodes[index].hash;
            self.nodes[index].pos = replacement;
            self.nodes[index].hash = hash(replacement) as i32;
            index = next;
        }
        let replacement = self.nodes[index].children[0]
            .or(self.nodes[index].children[1])
            .unwrap_or(index);
        if replacement != index {
            let parent = self.nodes[index].parent;
            self.nodes[replacement].parent = parent;
            if let Some(parent) = parent {
                let side = usize::from(self.nodes[parent].children[1] == Some(index));
                self.nodes[parent].children[side] = Some(replacement);
            } else {
                self.root = replacement;
            }
            self.nodes[index].parent = None;
            self.nodes[index].children = [None, None];
        }
        if !self.nodes[index].red {
            self.balance_remove(replacement);
        }
        if replacement == index {
            if let Some(parent) = self.nodes[index].parent {
                for side in 0..2 {
                    if self.nodes[parent].children[side] == Some(index) {
                        self.nodes[parent].children[side] = None;
                    }
                }
            }
            self.nodes[index].parent = None;
        }
        self.nodes[index].active = false;
    }
    fn balance_remove(&mut self, mut index: usize) {
        loop {
            if index == self.root {
                return;
            }
            let Some(parent) = self.nodes[index].parent else {
                self.nodes[index].red = false;
                return;
            };
            if self.nodes[index].red {
                self.nodes[index].red = false;
                return;
            }
            let side = usize::from(self.nodes[parent].children[1] == Some(index));
            let mut sibling = self.nodes[parent].children[side ^ 1];
            if let Some(s) = sibling.filter(|i| self.nodes[*i].red) {
                self.nodes[s].red = false;
                self.nodes[parent].red = true;
                self.rotate(parent, side ^ 1);
                sibling = self.nodes[parent].children[side ^ 1];
            }
            let Some(mut sibling) = sibling else {
                index = parent;
                continue;
            };
            if self.nodes[sibling]
                .children
                .iter()
                .all(|&child| !self.is_red(child))
            {
                self.nodes[sibling].red = true;
                index = parent;
                continue;
            }
            if !self.is_red(self.nodes[sibling].children[side ^ 1]) {
                if let Some(near) = self.nodes[sibling].children[side] {
                    self.nodes[near].red = false;
                }
                self.nodes[sibling].red = true;
                self.rotate(sibling, side);
                sibling = self.nodes[parent].children[side ^ 1].expect("rotated sibling");
            }
            self.nodes[sibling].red = self.nodes[parent].red;
            if let Some(far) = self.nodes[sibling].children[side ^ 1] {
                self.nodes[far].red = false;
            }
            self.nodes[parent].red = false;
            self.rotate(parent, side ^ 1);
            index = self.root;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Positions;
    use crate::tree::extra_tests::probe_sha256;
    use sha2::{Digest, Sha256};

    fn record(trace: &mut md5::Context, operation: [i32; 4], positions: &Positions) {
        for value in operation.into_iter().chain([positions.len as i32]) {
            trace.consume(value.to_le_bytes());
        }
        for (x, y, z) in positions.iter() {
            for value in [x, y, z] {
                trace.consume(value.to_le_bytes());
            }
        }
    }

    #[test]
    fn native_collision_bins_with_iterator_removal_and_resize() {
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("../../data/extra_tree_positions_26_1.json"))
                .unwrap();
        assert_eq!(reference["minecraft"], "26.1");
        assert_eq!(
            reference["jar_sha256"],
            "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
        );
        assert_eq!(reference["tree_probe_sha256"], probe_sha256(&[]));
        let helper = include_str!("../../../../scripts/capture_extra_tree_reference.py");
        assert_eq!(
            reference["capture_sha256"],
            format!("{:x}", Sha256::digest(helper.as_bytes()))
        );
        // The helper writes this literal with LF endings as the sixth Java source.
        let source = helper
            .split_once("POSITION_PROBE = \"\"\"")
            .unwrap()
            .1
            .split_once("\"\"\"")
            .unwrap()
            .0;
        assert_eq!(reference["probe_sha256"], probe_sha256(source.as_bytes()));

        let samples = reference["samples"]
            .as_array()
            .expect("native position-set traces");
        assert_eq!(samples.len(), 4);
        for (sample, name) in samples
            .iter()
            .zip(["ascending", "descending", "split", "signed"])
        {
            assert_eq!(sample["name"], name);
            let mut positions = Positions::new();
            let mut trace = md5::Context::new();
            let mut steps = 0;
            for i in 0..144_i32 {
                let value = match name {
                    "descending" => 300 - i,
                    "signed" => i - 80,
                    _ => i,
                };
                let spread = value * if name == "split" { 64 } else { 1024 };
                let x = (spread as u32 ^ ((spread as u32) >> 16)) as i32;
                positions.insert((x, 0, 0));
                record(&mut trace, [0, x, 0, 0], &positions);
                steps += 1;
                if i > 12 && i % 3 == 0 {
                    let (x, y, z) = positions.pop().expect("nonempty native trace");
                    record(&mut trace, [1, x, y, z], &positions);
                    steps += 1;
                }
            }
            while let Some((x, y, z)) = positions.pop() {
                record(&mut trace, [1, x, y, z], &positions);
                steps += 1;
            }
            assert_eq!(positions.len, 0);
            assert_eq!(steps, 288);
            assert_eq!(sample["steps"], steps);
            assert_eq!(
                sample["trace_md5"],
                format!("{:x}", trace.compute()),
                "{name}"
            );
        }
    }
}
