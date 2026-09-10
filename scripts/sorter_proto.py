"""Prototype of vanilla's FeatureSorter: compute the global placed-feature
index used by setFeatureSeed(decorationSeed, index, step).

Mirrors SteelMC's `steel-core/src/worldgen/feature/sorter.rs`:
  1. walk every possible biome's features in declaration order, assigning a
     first-seen `order` per placed feature;
  2. add an edge feature[i] -> feature[i+1] for consecutive features within a
     biome;
  3. DFS topological sort over vertices ordered by (step, order, feature_id);
  4. the final index is the position in that sorted list.
"""
import json
import os

BASE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(BASE, "..", "target", "vanilla-data")

biome_features = json.load(open(os.path.join(DATA, "biome_features.json")))
placed_features = json.load(open(os.path.join(DATA, "placed_features.json")))

# Registry id = position in the registry's registration order. The vanilla
# registry fills in datapack file order, i.e. alphabetical by resource path.
feature_id = {name: i for i, name in enumerate(placed_features)}
short = lambda k: k.split(":")[-1]


def build(possible_biomes):
    feature_order = {}
    next_order = 0
    edges = {}
    for biome in possible_biomes:
        steps = biome_features.get(short(biome))
        if steps is None:
            continue
        biome_vertices = []
        for step, stage in enumerate(steps):
            for key in stage:
                fid = feature_id.get(short(key))
                if fid is None:
                    continue
                order = feature_order.setdefault(fid, next_order)
                if order == next_order:
                    next_order += 1
                v = (step, order, fid)
                edges.setdefault(v, [])
                biome_vertices.append(v)
        for a, b in zip(biome_vertices, biome_vertices[1:]):
            edges.setdefault(a, []).append(b)

    # DFS topological sort (equivalent to SteelMC's visit + reverse).
    sorted_v = []
    discovered = set()
    visiting = set()

    def visit(v):
        if v in discovered:
            return
        if v in visiting:
            raise RuntimeError("cycle in feature graph")
        visiting.add(v)
        for w in edges.get(v, []):
            visit(w)
        visiting.discard(v)
        discovered.add(v)
        sorted_v.append(v)

    for v in sorted(edges, key=lambda t: (t[0], t[1], t[2])):
        visit(v)
    sorted_v.reverse()
    return {v[2]: i for i, v in enumerate(sorted_v)}


# possible_biomes = every registered biome, in registry (alphabetical) order
ALL = sorted(biome_features)
idx = build(ALL)

TREES = [
    "trees_plains", "trees_birch_and_oak_leaf_litter", "trees_birch",
    "trees_taiga", "trees_snowy", "trees_savanna", "trees_jungle",
    "trees_swamp", "trees_windswept_hills", "dark_forest_vegetation",
    "trees_sparse_jungle", "trees_badlands", "trees_flower_forest",
    "trees_mangrove", "trees_grove", "trees_old_growth_pine_taiga",
]
print("registered placed features:", len(placed_features))
for t in TREES:
    fid = feature_id.get(t)
    pos = idx.get(fid) if fid is not None else None
    print(f"  {t:38s} registry_id={fid}  sorter_index={pos}")
print("total sorted vertices:", len(idx))
