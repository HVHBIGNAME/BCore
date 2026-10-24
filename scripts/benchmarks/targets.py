"""Explicit native versions and recorded Java source derivations for NOISE timing."""
from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import sys
import zipfile

from provenance import JAVA_SOURCES, sha, write_json


@dataclass(frozen=True)
class Target:
    minecraft: str
    protocol: int
    jar_sha256: str
    directory: str

    @property
    def engine(self) -> str:
        return "Vanilla " + self.minecraft


TARGETS = {
    "26.1": Target("26.1", 775, "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52", "vanilla-775"),
    "26.2": Target("26.2", 776, "183c0499c5f855570ee487dd38e141a53f0121f83a0b07a3bac2d8b6698823e8", "vanilla-776"),
}


def authenticate(root: Path, target: Target) -> tuple[Path, list[Path], dict]:
    jar = root / f"target/{target.directory}/versions/{target.minecraft}/server-{target.minecraft}.jar"
    if sha(jar) != target.jar_sha256:
        raise ValueError(f"native JAR digest differs from pinned {target.minecraft}")
    if target.minecraft == "26.2":
        sys.path.insert(0, str(root / "scripts/migration-26.2"))
        import native
        version, libraries = native.environment()
        if len(libraries) != 39 or jar != native.JAR:
            raise ValueError("26.2 requires the prepared JAR and all 39 authenticated libraries")
    else:
        libraries = sorted((root / f"target/{target.directory}/libraries").rglob("*.jar"))
        if not libraries:
            raise ValueError("missing native libraries")
        with zipfile.ZipFile(jar) as archive:
            version = json.loads(archive.read("version.json"))
    if (version["name"], version["protocol_version"]) != (target.minecraft, target.protocol):
        raise ValueError("native version metadata differs from the selected target")
    return jar, libraries, version


def adapt_benchmark(raw: bytes) -> bytes:
    text = raw.decode("utf-8", "strict")
    setup = '        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();'
    label = 'result.put("engine", "Vanilla 26.1")'
    output = '        result.put("seed", Long.toString(seed));'
    if any(text.count(seam) != 1 for seam in (setup, label, output)):
        raise ValueError("preserved NOISE benchmark derivation seam differs")
    proof = '''
        @SuppressWarnings("unchecked") Map<String, Object> nativeVersion;
        try (java.io.InputStream stream = type("SharedConstants").getResourceAsStream("/version.json")) {
            if (stream == null) throw new IllegalStateException("native version resource missing");
            nativeVersion = (Map<String, Object>) call(gson, "fromJson",
                new String(stream.readAllBytes(), java.nio.charset.StandardCharsets.UTF_8), Map.class);
        }
        if (!"26.2".equals(nativeVersion.get("name"))
                || ((Number) nativeVersion.get("protocol_version")).intValue() != 776
                || ((Number) nativeVersion.get("world_version")).intValue() != 4903)
            throw new IllegalStateException("native benchmark is not running 26.2 / 776 / 4903");'''
    identity = '''        result.put("minecraft", nativeVersion.get("name"));
        result.put("protocol", ((Number) nativeVersion.get("protocol_version")).intValue());
        result.put("data_version", ((Number) nativeVersion.get("world_version")).intValue());
'''
    text = text.replace(setup, setup + proof).replace(label, 'result.put("engine", "Vanilla 26.2")')
    text = text.replace(output, identity + output)
    return text.encode("utf-8")


def sources(source: Path, output: Path, target: Target) -> tuple[list[Path], dict]:
    if target.minecraft == "26.1":
        return [source / name for name in JAVA_SOURCES], {}
    derived = output / "native-source"
    derived.mkdir()
    paths, receipt = [], {}
    for name in JAVA_SOURCES:
        original = source / name
        raw = original.read_bytes()
        value = adapt_benchmark(raw) if original.name == "NoiseFillBenchmark.java" else raw
        path = derived / original.name
        with path.open("xb") as stream:
            stream.write(value)
        paths.append(path)
        receipt[path.relative_to(output).as_posix()] = {"sha256": sha(path), "original_path": name,
                                                     "original_sha256": sha(original)}
    write_json(output / "native-source-derivation.json", receipt)
    return paths, receipt
