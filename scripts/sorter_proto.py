#!/usr/bin/env python3
"""Reference model of vanilla's FeatureSorter index assignment.

WHY THIS FILE EXISTS
    It is the oracle the Rust port (crates/bcore-worldgen/src/feature_sorter.rs)
    is checked against.  If the two disagree, fix the Rust -- never this file.

INDEX SEMANTICS (from SteelMC steel-core/src/worldgen/feature/sorter.rs)
    FeatureSorter topologically sorts the feature graph, then, PER STEP, gives
    every placed feature an index equal to its position *inside that step's*
    list.  Decoration calls set_feature_seed(decoration_seed, index, step) with
    that WITHIN-STEP index.  It is NOT a global position across all steps.

    This distinction was the bug in the previous revision of this file: it
    returned the global position and reported trees_plains=138 / ore_copper=73,
    which are both wrong and led a Rust port to reproduce them.

    possibleBiomes is the biome *source's* set.  For the overworld that is the
    overworld biomes only -- nether and end biomes must NOT be included, or the
    topological order shifts (trees_plains 50 instead of 48).

VERIFIED ANCHOR
    ore_copper -> step 6, index 25.  Confirmed against an independent reference
    ("ore_copper is 25, not 24") and therefore ASSERTED below: if this script
    stops reproducing it, this script is broken.
"""
import json
import os
import sys

BASE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DATA = os.path.join(BASE, 'target', 'vanilla-data')

with open(os.path.join(DATA, 'biome_features.json')) as fh:
    biome_features = json.load(fh)
with open(os.path.join(DATA, 'placed_features.json')) as fh:
    placed_features = json.load(fh)

feature_id = {name: i for i, name in enumerate(placed_features)}


def short(key):
    return key.split(':')[-1]


# Overworld biome source only -- see module docstring.
NETHER_END = {
    'basalt_deltas', 'crimson_forest', 'warped_forest', 'nether_wastes',
    'soul_sand_valley', 'end_barrens', 'end_highlands', 'end_midlands',
    'small_end_islands', 'the_end',
}
POSSIBLE_BIOMES = [b for b in sorted(biome_features) if b not in NETHER_END]


def sorted_vertices(possible_biomes):
    """Topologically sort the feature graph; mirrors FeatureSorter::build."""
    order, edges, next_order = {}, {}, 0
    for biome in possible_biomes:
        stages = biome_features.get(biome)
        if stages is None:
            continue
        own = []
        for step, stage in enumerate(stages):
            for key in stage:
                fid = feature_id.get(short(key))
                if fid is None:
                    continue
                if fid not in order:
                    order[fid] = next_order
                    next_order += 1
                vertex = (step, order[fid], fid)
                edges.setdefault(vertex, [])
                own.append(vertex)
        for a, b in zip(own, own[1:]):
            edges[a].append(b)

    vertices = sorted(
        {v for v in edges} | {w for lst in edges.values() for w in lst},
        key=lambda t: (t[0], t[1], t[2]),
    )

    done, active, out = set(), set(), []

    def visit(v):
        if v in done:
            return
        if v in active:
            raise RuntimeError('cycle in feature graph at %r' % (v,))
        active.add(v)
        for w in edges.get(v, ()):
            visit(w)
        active.discard(v)
        done.add(v)
        out.append(v)

    for v in vertices:
        visit(v)
    return out[::-1]


def per_step_index(possible_biomes):
    """Map (step, placed_feature_id) -> WITHIN-STEP index."""
    by_step = {}
    for v in sorted_vertices(possible_biomes):
        by_step.setdefault(v[0], []).append(v)
    index = {}
    for step, vs in by_step.items():
        for i, v in enumerate(vs):
            index[(step, v[2])] = i
    return index


# Independently verified; these must never be relaxed to match output.
ANCHORS = {'ore_copper': (6, 25)}


def main():
    index = per_step_index(POSSIBLE_BIOMES)

    print('possible_biomes: %d (nether/end excluded: %d)'
          % (len(POSSIBLE_BIOMES), len(biome_features) - len(POSSIBLE_BIOMES)))
    print()

    bad = 0
    for name, (step, want) in sorted(ANCHORS.items()):
        got = index.get((step, feature_id[name]))
        ok = got == want
        bad += 0 if ok else 1
        print('%s %-22s step=%2d index=%-4s (verified anchor: %d)'
              % ('OK  ' if ok else 'FAIL', name, step, got, want))

    print()
    for name, (step, want) in sorted({
        'ore_copper_large': (6, 24),
        'trees_plains': (9, 48),
        'trees_taiga': (9, 15),
        'trees_birch': (9, 46),
    }.items()):
        got = index.get((step, feature_id[name]))
        flag = '' if got == want else '   <-- MISMATCH'
        print('     %-22s step=%2d index=%-4s (target %d)%s'
              % (name, step, got, want, flag))

    if bad:
        print('\n%d verified anchor(s) FAILED -- this script is wrong' % bad,
              file=sys.stderr)
        return 1
    print('\nverified anchor reproduced; indices are trustworthy')
    return 0


if __name__ == '__main__':
    sys.exit(main())
