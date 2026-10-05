"""Capture extra trees or the existing four native collision-bin traces.

The collision probe is separate so ExtraTreeReference.java retains the exact
source fingerprint of the immutable configured-tree and catalog fixtures.
"""
import hashlib
import json
from pathlib import Path

import capture_tree_reference as capture


TREE_SOURCES = (
    "OreReference.java", "NativeWorldgenRegistries.java", "VegetationReference.java",
    "ExtraTreeReference.java",
)

POSITION_PROBE = """import java.util.*;
import java.security.MessageDigest;

public class ExtraTreePositionsReference extends ExtraTreeReference {
    static void record(MessageDigest trace, int[] operation, Set<Object> positions) throws Exception {
        digest(trace, operation);
        digest(trace, new int[]{positions.size()});
        for (Object element : positions) digest(trace, xyz(position(element)));
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        List<Object> samples = new ArrayList<>();
        for (String kind : List.of("ascending", "descending", "split", "signed")) {
            Set<Object> positions = new HashSet<>();
            MessageDigest trace = MessageDigest.getInstance("MD5");
            int steps = 0;
            for (int i = 0; i < 144; i++) {
                int value = switch (kind) {
                    case "descending" -> 300 - i;
                    case "signed" -> i - 80;
                    default -> i;
                };
                int spread = value * (kind.equals("split") ? 64 : 1024);
                int x = spread ^ (spread >>> 16);
                positions.add(make("core.BlockPos", x, 0, 0));
                record(trace, new int[]{0, x, 0, 0}, positions);
                steps++;
                if (i > 12 && i % 3 == 0) {
                    Iterator<Object> iterator = positions.iterator();
                    Pos removed = position(iterator.next());
                    iterator.remove();
                    record(trace, new int[]{1, removed.x(), removed.y(), removed.z()}, positions);
                    steps++;
                }
            }
            while (!positions.isEmpty()) {
                Iterator<Object> iterator = positions.iterator();
                Pos removed = position(iterator.next());
                iterator.remove();
                record(trace, new int[]{1, removed.x(), removed.y(), removed.z()}, positions);
                steps++;
            }
            samples.add(Map.of("name", kind, "steps", steps,
                "trace_md5", HexFormat.of().formatHex(trace.digest())));
        }
        System.out.println("EXTRA_TREE_POSITIONS_REFERENCE=" + call(gson, "toJson", Map.of(
            "samples", samples, "encoding", "For each operation: four int32 LE op values, int32 LE size, then every native iterator xyz as int32 LE.")));
    }
}
"""


def main():
    capture.PROBES["extra_tree"] = TREE_SOURCES
    capture.PROBES["extra_tree_positions"] = (*TREE_SOURCES, "ExtraTreePositionsReference.java")
    args = capture.parse_args()
    if args.probe == "extra_tree_positions":
        build = (args.build_dir or capture.ROOT / "target/extra-tree-positions-reference").resolve()
        build.mkdir(parents=True, exist_ok=True)
        source = build / "ExtraTreePositionsReference.java"
        source.write_text(POSITION_PROBE, encoding="utf-8", newline="\n")
        capture.PROBES[args.probe] = (*TREE_SOURCES, str(source))
        args.build_dir = build
    reference = capture.capture(args)
    if args.probe == "extra_tree_positions":
        sources = (capture.ROOT / "scripts" / name for name in ("TreeReference.java", *TREE_SOURCES))
        reference["tree_probe_sha256"] = hashlib.sha256(b"".join(p.read_bytes() for p in sources)).hexdigest()
        reference["capture_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    if args.verify and reference != json.loads(args.verify.read_text(encoding="utf-8")):
        raise ValueError(f"JAR output differs from {args.verify}")
    args.output.write_text(json.dumps(reference, indent=2) + "\n", encoding="utf-8")
    print(f"{'Verified' if args.verify else 'Captured'} {capture.summarize(args.probe, reference)} to {args.output}")


if __name__ == "__main__":
    main()
