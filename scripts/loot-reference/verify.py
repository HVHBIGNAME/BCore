"""Check native loot evidence, RNG transitions, slot placement, and exact repeats."""

import argparse
from collections import Counter
import json
from pathlib import Path
import re
import struct
import itertools

from inventory import digest, write_json

MULTIPLIER = 25214903917
MASK = (1 << 48) - 1


def signed(value, width):
    value &= (1 << width) - 1
    return value - (1 << width) if value >= 1 << (width - 1) else value


class Rng:
    def __init__(self, state):
        self.state = state

    def bits(self, bits):
        self.state = (self.state * MULTIPLIER + 11) & MASK
        return signed(self.state >> (48 - bits), 32)

    def integer(self, bound=None):
        if bound is None:
            return self.bits(32)
        if bound & (bound - 1) == 0:
            return (bound * self.bits(31)) >> 31
        while True:
            bits = self.bits(31)
            value = bits % bound
            if signed(bits - value + bound - 1, 32) >= 0:
                return value

    def long(self):
        return signed((self.bits(32) << 32) + self.bits(32), 64)

    def float_bits(self):
        return struct.unpack(">I", struct.pack(">f", self.bits(24) / (1 << 24)))[0]

    def double_bits(self):
        value = ((self.bits(26) << 27) + self.bits(27)) / (1 << 53)
        return struct.unpack(">Q", struct.pack(">d", value))[0]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def check_rng(enter, exit_row):
    before, after = enter.get("rng"), exit_row.get("rng")
    if not before or "seed48" not in before:
        return False
    rng = Rng(int(before["seed48"]))
    name = enter["method"].split(".")[-1]
    args, actual = enter["arguments"], exit_row["result"]
    if name == "setSeed(J)V":
        rng.state = (int(args[0]["i64"]) ^ MULTIPLIER) & MASK
        expected = None
    elif name == "next(I)I":
        expected = rng.bits(args[0])
    elif name == "nextInt()I":
        expected = rng.integer()
    elif name == "nextInt(I)I":
        expected = rng.integer(args[0])
    elif name == "nextLong()J":
        expected, actual = rng.long(), int(actual["i64"])
    elif name == "nextBoolean()Z":
        expected = bool(rng.bits(1))
    elif name == "nextFloat()F":
        expected, actual = rng.float_bits(), int(actual["bits"])
    elif name == "nextDouble()D":
        expected, actual = rng.double_bits(), int(actual["bits"])
    else:
        raise ValueError(f"RNG operation needs an explicit verifier: {name}")
    require(expected == actual and rng.state == int(after["seed48"]),
            f"RNG mismatch in {enter['case']}/{enter['route']} call {enter['call']}: {name}")
    return True


def check_continuation(route):
    continuation = route["continuation"]
    require(continuation["status"] == "captured", "missing exact RNG continuation")
    rng = Rng(int(route["rng_after"]["seed48"]))
    expected = [rng.integer(1000), rng.long(), bool(rng.bits(1)), rng.float_bits(), rng.double_bits(), rng.state]
    actual = [continuation["next_int_1000"], int(continuation["next_long"]["i64"]), continuation["next_boolean"],
              int(continuation["next_float"]["bits"]), int(continuation["next_double"]["bits"]), int(continuation["after"]["seed48"])]
    require(expected == actual, "native continuation differs from 48-bit state")


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def normalize_trace(row):
    """Only remove JVM hidden-lambda address suffixes from class metadata."""
    if isinstance(row, dict):
        return {key: re.sub(r"(?<=\$\$Lambda)/0x[0-9a-fA-F]+$", "/<runtime-address>", value)
                if key == "class" and isinstance(value, str) else normalize_trace(value)
                for key, value in row.items()}
    if isinstance(row, list):
        return [normalize_trace(item) for item in row]
    return row


def stacks_total(values):
    result = Counter()
    for value in values:
        if value:
            item = {k: v for k, v in value.items() if k != "count"}
            result[canonical(item)] += value.get("count", 1)
    return result


def check_route(route):
    before, after = route["before"], route["after"]
    require(len(before) == len(after), "container size changed")
    for slot, item in enumerate(before):
        if item:
            require(item == after[slot], f"preexisting slot {slot} changed or merged")
    shuffled_slots = next(stage["values"] for stage in route["stages"] if ".getAvailableSlots(" in stage["method"])
    require(sorted(shuffled_slots) == [i for i, item in enumerate(before) if item is None], "empty-slot inventory differs")
    split = next(stage["values"] for stage in route["stages"] if ".shuffleAndSplitItems(" in stage["method"])
    generated = next(stage["values"] for stage in route["stages"] if ".getRandomItems(" in stage["method"])
    require(stacks_total(generated) == stacks_total(split), "shuffle/split changed item count or components")
    expected = [{"slot": slot, "stack": item} for slot, item in zip(reversed(shuffled_slots), split)]
    require(expected == route["placements"], "native placement order differs from shuffled-slot pop order")
    for item in route["placements"]:
        final = after[item["slot"]]
        require((final["stack"] if final else None) == item["stack"], "stored item differs from placement")
    for item in after:
        if item:
            require(not item["effective_components"]["unserialized_transient_types"], "uncaptured transient component")
            require(item["stack"].get("count", 1) <= item["max_stack_size"], "item over maximum stack size")
    check_continuation(route)


def verify_capture(path, repeat_of=None):
    observations = json.loads((path / "observations.raw.json").read_text(encoding="utf-8"))
    require(observations["gameplay_ticks"] == 0, "unexpected gameplay tick")
    functions, methods, pending, rng_checks, events = Counter(), Counter(), {}, 0, 0
    case_functions = {}
    with (path / "trace.jsonl").open(encoding="utf-8") as trace:
        for line in trace:
            row = json.loads(line)
            events += 1
            require(row["event"] == events, "noncontiguous trace events")
            key = row["case"], row["route"], row["call"]
            if row["boundary"] == "enter":
                require(key not in pending, "duplicate trace call")
                pending[key] = row
                methods[row["method"]] += 1
                if "/loot/functions/" in row["method"]:
                    functions[row["method"].split(".")[0]] += 1
                    case_functions.setdefault(row["case"], set()).add(row["method"].split(".")[0])
            else:
                enter = pending.pop(key)
                require(enter["method"] == row["method"], "trace call method differs")
                rng_checks += check_rng(enter, row)
    require(not pending and events == observations["event_count"], "incomplete native trace")
    routes, paired, missing = 0, 0, []
    for case in observations["cases"]:
        if case["status"] != "captured":
            missing.append(case["input"])
            continue
        check_route(case["direct"])
        routes += 1
        expected_function = case["input"].get("expected_function_class")
        if expected_function:
            require(expected_function in case_functions.get(case["input"]["id"], set()),
                    f"planned native function was not observed: {case['input']['id']}")
        if "unpack" in case:
            unpack = case["unpack"]
            check_route(unpack)
            routes += 1
            for field in ("before", "after", "stages", "placements", "rng_after", "continuation"):
                require(case["direct"][field] == unpack[field], f"direct/unpack differs: {case['input']['id']}/{field}")
            require(unpack["loot_table_after"] is None, "loot key was not cleared")
            require(unpack["loot_seed_after"] == case["input"]["seed"], "unpack unexpectedly changed seed")
            require(unpack["after"] == unpack["second_unpack_slots"] and unpack["second_unpack_rng_unchanged"], "second unpack not a no-op")
            paired += 1
    write_json(path / "fixtures.json", observations)
    repeat_files = ["fixtures.json", "items.json", "registry-types.json", "enchantment.json", "instrument.json", "potions.json", "loaded-tables.json"]
    repeat_results = {}
    if repeat_of:
        for name in repeat_files:
            left = json.loads((repeat_of / name).read_text(encoding="utf-8"))
            right = json.loads((path / name).read_text(encoding="utf-8"))
            require(left == right, f"independent native repeat differs: {name}")
            repeat_results[name] = {"canonical_sha256": digest(canonical(right).encode()), "equal": True}
        with (repeat_of / "trace.jsonl").open(encoding="utf-8") as left, (path / "trace.jsonl").open(encoding="utf-8") as right:
            for index, (a, b) in enumerate(itertools.zip_longest(left, right), 1):
                require(a is not None and b is not None, "independent native trace length differs")
                require(normalize_trace(json.loads(a)) == normalize_trace(json.loads(b)), f"independent native trace differs at event {index}")
        repeat_results["trace.jsonl"] = {"sha256": digest((path / "trace.jsonl").read_bytes()), "equal": True,
            "normalization": "Only $$Lambda/0x<hex> hidden-class runtime address metadata is replaced; all observed values/order are exact."}
    summary = {"status": "pass", "case_count": len(observations["cases"]), "native_fills": routes,
               "direct_unpack_equal": paired, "missing_cases": missing, "events": events,
               "verified_rng_operations": rng_checks, "function_calls": dict(functions),
               "method_calls": dict(methods), "independent_repeat": repeat_results,
               "fixtures_sha256": digest((path / "fixtures.json").read_bytes())}
    write_json(path / "verification.json", summary)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--repeat-of", type=Path)
    args = parser.parse_args()
    print(json.dumps(verify_capture(args.capture, args.repeat_of), indent=2))


if __name__ == "__main__":
    main()
