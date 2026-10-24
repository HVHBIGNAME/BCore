"""Restore verified native dependencies without running Java or Cargo.

Each attempt needs a new --output directory under target/. Downloads, upstream
metadata, bundle manifests and failures remain there. Existing installed files
are reused only when their bytes match. Run --verify-runtime through the shared
scripts/run_generation_check.py queue; that mode actually starts Java/javac.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import datetime as dt
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
from urllib.parse import urlsplit, urlunsplit
from urllib.request import Request, urlopen
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "target"
MINECRAFT = "26.1"
PROTOCOL = 775
SERVER_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
ASM_SHA256 = "876eab6a83daecad5ca67eb9fcabb063c97b5aeb8cf1fca7a989ecde17522051"
PREFERRED_JAVA = "jdk-25.0.4.1+1"
VERSION_MANIFEST = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
ADOPTIUM_RELEASES = (
    "https://api.adoptium.net/v3/assets/feature_releases/25/ga"
    "?architecture=x64&image_type=jre&jvm_impl=hotspot&os=windows&vendor=eclipse&page_size=20"
)
ASM_URL = "https://repo.maven.apache.org/maven2/org/ow2/asm/asm/9.8/asm-9.8.jar"


def utc():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def write_json(path, value, *, exclusive=True):
    with path.open("x" if exclusive else "w", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def digest(path, algorithm="sha256"):
    result = hashlib.new(algorithm)
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def download(url, destination, *, expected=None, size=None):
    """Retain transfer evidence, including partial bytes after any failure."""
    if urlsplit(url).scheme != "https":
        raise ValueError(f"download is not HTTPS: {url}")
    record = {"url": url, "started_utc": utc(), "expected": expected or {},
              "expected_size": size, "state": "downloading"}
    receipt = destination.with_name(destination.name + ".download.json")
    partial = destination.with_name(destination.name + ".part")
    write_json(receipt, record)
    print(f"Downloading {url}", flush=True)
    try:
        request = Request(url, headers={"User-Agent": "BCore-native-restoration/1"})
        with urlopen(request, timeout=120) as response, partial.open("xb") as stream:
            final = urlsplit(response.url)
            if final.scheme != "https":
                raise ValueError("download redirected away from HTTPS")
            # Signed CDN query strings are transient; the stable upstream URL is above.
            record["response_url"] = urlunsplit((final.scheme, final.netloc, final.path, "", ""))
            record["http_status"] = response.status
            record["headers"] = {k: response.headers[k] for k in
                                 ("Content-Length", "Content-Type", "ETag", "Last-Modified")
                                 if k in response.headers}
            shutil.copyfileobj(response, stream, length=1024 * 1024)
        record["size"] = partial.stat().st_size
        record["sha256"] = digest(partial)
        for algorithm, wanted in (expected or {}).items():
            actual = digest(partial, algorithm)
            record[algorithm] = actual
            if actual != wanted:
                raise ValueError(f"{destination.name}: {algorithm} differs ({actual} != {wanted})")
        if size is not None and record["size"] != size:
            raise ValueError(f"{destination.name}: size differs")
        if destination.exists():
            raise FileExistsError(destination)
        partial.rename(destination)
        record["state"] = "verified" if expected else "downloaded"
        return record
    except Exception as error:
        record.update(state="failed", error_type=type(error).__name__, error=str(error))
        raise
    finally:
        record["finished_utc"] = utc()
        write_json(receipt, record, exclusive=False)


def member_path(name):
    path = PurePosixPath(name)
    if not path.parts or path.is_absolute() or ".." in path.parts or "\\" in name or ":" in name:
        raise ValueError(f"unsafe archive path: {name}")
    return Path(*path.parts)


def install_member(archive, member, destination, expected=None):
    info = archive.getinfo(member)
    if stat.S_ISLNK(info.external_attr >> 16):
        raise ValueError(f"archive symlink: {member}")
    data = archive.read(info)
    actual = hashlib.sha256(data).hexdigest()
    if expected is not None and actual != expected:
        raise ValueError(f"bundle manifest hash differs: {member}")
    install_bytes(data, destination, actual)
    return actual


def install_bytes(data, destination, expected):
    if destination.exists():
        if digest(destination) != expected:
            raise ValueError(f"existing installed artifact differs: {destination}")
        return
    if not destination.resolve().is_relative_to(TARGET.resolve()):
        raise ValueError(f"installation escapes target: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("xb") as stream:
        stream.write(data)
    if digest(destination) != expected:
        raise ValueError(f"installed artifact hash differs: {destination}")


def manifest_rows(data):
    result = []
    seen = set()
    for line in data.decode("utf-8").splitlines():
        fields = line.split("\t")
        if len(fields) != 3 or not re.fullmatch("[0-9a-f]{64}", fields[0]):
            raise ValueError(f"invalid bundle manifest row: {line}")
        checksum, identity, name = fields
        member_path(name)
        if name in seen or not name.endswith(".jar"):
            raise ValueError(f"duplicate or non-JAR bundle entry: {name}")
        seen.add(name)
        result.append((checksum, identity, name))
    if not result:
        raise ValueError("empty bundle manifest")
    return result


def restore_minecraft(output):
    directory = output / "minecraft"
    directory.mkdir()
    manifest = directory / "version-manifest.json"
    download(VERSION_MANIFEST, manifest)
    entry, = [v for v in json.loads(manifest.read_bytes())["versions"] if v["id"] == MINECRAFT]
    metadata = directory / "version-26.1.json"
    download(entry["url"], metadata, expected={"sha1": entry["sha1"]})
    version = json.loads(metadata.read_bytes())
    if version["id"] != MINECRAFT or version["type"] != "release" or version["javaVersion"]["majorVersion"] != 25:
        raise ValueError("official Minecraft version identity differs")
    server = version["downloads"]["server"]
    bundle = directory / "server-26.1-bundle.jar"
    transfer = download(server["url"], bundle, expected={"sha1": server["sha1"]}, size=server["size"])
    vanilla = TARGET / "vanilla-775"
    files = {}
    manifests = {}
    with zipfile.ZipFile(bundle) as archive:
        for name in ("META-INF/MANIFEST.MF", "META-INF/versions.list", "META-INF/libraries.list", "META-INF/main-class"):
            data = archive.read(name)
            (directory / PurePosixPath(name).name).write_bytes(data)
            manifests[name] = hashlib.sha256(data).hexdigest()
        versions = manifest_rows(archive.read("META-INF/versions.list"))
        libraries = manifest_rows(archive.read("META-INF/libraries.list"))
        checksum, identity, name = versions[0]
        if len(versions) != 1 or identity != MINECRAFT or checksum != SERVER_SHA256:
            raise ValueError("bundle inner-server identity differs from repository pin")
        if name != "26.1/server-26.1.jar":
            raise ValueError(f"unexpected inner-server path: {name}")
        for kind, rows in (("versions", versions), ("libraries", libraries)):
            for checksum, identity, name in rows:
                destination = vanilla / kind / member_path(name)
                files[destination.relative_to(ROOT).as_posix()] = install_member(
                    archive, f"META-INF/{kind}/{name}", destination, checksum)
    inner = vanilla / "versions/26.1/server-26.1.jar"
    with zipfile.ZipFile(inner) as archive:
        data = archive.read("version.json")
        (directory / "inner-version.json").write_bytes(data)
        identity = json.loads(data)
    if identity["name"] != MINECRAFT or identity["protocol_version"] != PROTOCOL or identity["java_version"] != 25:
        raise ValueError("inner server version/protocol/Java requirement differs")
    actual_libraries = {p.relative_to(ROOT).as_posix() for p in (vanilla / "libraries").rglob("*.jar")}
    expected_libraries = {p for p in files if p.startswith("target/vanilla-775/libraries/")}
    if actual_libraries != expected_libraries:
        raise ValueError("native library directory has entries outside the official bundle manifest")
    return {"version_manifest_entry": entry, "version_metadata_sha256": digest(metadata),
            "bundle": transfer, "bundle_manifests_sha256": manifests, "inner_version": identity,
            "inner_server_sha256": digest(inner), "library_count": len(libraries), "files_sha256": files}


def restore_java(output):
    directory = output / "java"
    directory.mkdir()
    metadata = directory / "adoptium-releases.json"
    download(ADOPTIUM_RELEASES, metadata)
    releases = json.loads(metadata.read_bytes())
    eligible = [r for r in releases if r["version_data"]["major"] == 25 and r["release_type"] == "ga"
                and r["vendor"] == "eclipse"]
    if not eligible:
        raise ValueError("Adoptium lists no compatible Java 25 GA JRE")
    release = next((r for r in eligible if r["release_name"] == PREFERRED_JAVA), eligible[0])
    binary, = [b for b in release["binaries"] if b["architecture"] == "x64" and b["os"] == "windows"
               and b["image_type"] == "jre" and b["jvm_impl"] == "hotspot"]
    package = binary["package"]
    checksum_file = directory / (package["name"] + ".sha256.txt")
    download(package["checksum_link"], checksum_file)
    if checksum_file.read_text(encoding="utf-8").split()[0] != package["checksum"]:
        raise ValueError("Adoptium API and release SHA-256 disagree")
    download(package["metadata_link"], directory / (package["name"] + ".json"))
    archive_path = directory / package["name"]
    transfer = download(package["link"], archive_path, expected={"sha256": package["checksum"]}, size=package["size"])
    java_root = TARGET / "jre25"
    top = release["release_name"] + "-jre"
    files = {}
    with zipfile.ZipFile(archive_path) as archive:
        for info in archive.infolist():
            path = member_path(info.filename)
            if path.parts[0] != top:
                raise ValueError(f"JRE archive root differs from release identity: {path}")
            if info.is_dir():
                continue
            destination = java_root / path
            files[destination.relative_to(ROOT).as_posix()] = install_member(archive, info.filename, destination)
    home = java_root / top
    properties = dict(line.split("=", 1) for line in (home / "release").read_text(encoding="utf-8").splitlines() if "=" in line)
    properties = {k: v.strip('"') for k, v in properties.items()}
    if properties["JAVA_RUNTIME_VERSION"] != release["version_data"]["openjdk_version"]:
        raise ValueError("installed JRE release file differs from official release metadata")
    java = home / "bin/java.exe"
    if not java.is_file():
        raise ValueError("installed JRE has no java.exe")
    return {"preferred_release": PREFERRED_JAVA, "release_name": release["release_name"],
            "preferred_release_restored": release["release_name"] == PREFERRED_JAVA,
            "selection": "preferred release when listed, otherwise latest compatible official Java 25 GA",
            "release_link": release["release_link"], "version": release["version_data"],
            "release_properties": properties, "archive": transfer, "java_path": str(java),
            "files_sha256": files}


def restore_asm(output):
    directory = output / "asm"
    directory.mkdir()
    archive = directory / "asm-9.8.jar"
    transfer = download(ASM_URL, archive, expected={"sha256": ASM_SHA256})
    installed = TARGET / "full-lifecycle-deps/asm-9.8.jar"
    install_bytes(archive.read_bytes(), installed, ASM_SHA256)
    return {"version": "9.8", "download": transfer,
            "files_sha256": {installed.relative_to(ROOT).as_posix(): digest(installed)}}


def restore(output):
    output.mkdir(parents=True)
    script = Path(__file__).resolve()
    shutil.copyfile(script, output / script.name)
    record = {"schema": 1, "state": "restoring", "started_utc": utc(), "root": str(ROOT),
              "command": [sys.executable, *sys.argv], "script_sha256": digest(script),
              "components": {}, "errors": {},
              "historical_comparability": "No old timing/capture artifacts reconstructed; fresh baseline required."}
    write_json(output / "restoration.json", record)
    tasks = {"minecraft": restore_minecraft, "java": restore_java, "asm": restore_asm}
    with ThreadPoolExecutor(max_workers=3) as executor:
        futures = {executor.submit(action, output): name for name, action in tasks.items()}
        for future in as_completed(futures):
            name = futures[future]
            try:
                result = future.result()
                record["components"][name] = result
                write_json(output / f"{name}-provenance.json", result)
                print(f"Verified and restored {name}", flush=True)
            except Exception as error:
                record["errors"][name] = {"error_type": type(error).__name__, "error": str(error)}
                print(f"Restoration failed for {name}: {error}", flush=True)
            write_json(output / "restoration.json", record, exclusive=False)
    record.update(state="failed" if record["errors"] else "verified-files", finished_utc=utc())
    write_json(output / "restoration.json", record, exclusive=False)
    print(json.dumps({"output": str(output), "state": record["state"], "errors": record["errors"],
                      "java_path": record["components"].get("java", {}).get("java_path")}, indent=2))
    return 1 if record["errors"] else 0


def verify_runtime(path):
    """Native process execution: caller must use the shared heavy-work queue."""
    source = json.loads(path.read_bytes())
    if source["state"] != "verified-files" or source["root"] != str(ROOT):
        raise ValueError("restoration is incomplete or belongs to another workspace")
    output = path.parent / "runtime-verification.json"
    record = {"schema": 1, "started_utc": utc(), "restoration_sha256": digest(path), "state": "verifying"}
    write_json(output, record)
    try:
        count = 0
        for component in source["components"].values():
            for name, expected in component["files_sha256"].items():
                if digest(ROOT / name) != expected:
                    raise ValueError(f"restored artifact changed: {name}")
                count += 1
        java = source["components"]["java"]
        javac = shutil.which("javac")
        if javac is None:
            raise ValueError("javac is missing from PATH")
        versions = {}
        for name, executable in (("java", java["java_path"]), ("javac", javac)):
            command = [executable, "-version"]
            result = subprocess.run(command, capture_output=True, text=True, timeout=60)
            text = result.stdout + result.stderr
            log = path.parent / f"runtime-verification-{name}.log"
            with log.open("x", encoding="utf-8") as stream:
                stream.write(text)
            versions[name] = {"command": command, "exit_code": result.returncode, "output": text.strip(),
                              "executable_sha256": digest(Path(executable)), "log_sha256": digest(log)}
            record["versions"] = versions
            result.check_returncode()
        expected_version = java["release_properties"]["JAVA_VERSION"]
        if f'version "{expected_version}"' not in versions["java"]["output"]:
            raise ValueError("running Java version differs from the verified release file")
        record.update(state="verified", verified_file_count=count)
    except Exception as error:
        record.update(state="failed", error_type=type(error).__name__, error=str(error))
        raise
    finally:
        record["finished_utc"] = utc()
        write_json(output, record, exclusive=False)
        print(json.dumps(record, indent=2))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--output", type=Path, help="new restoration provenance directory inside target/")
    mode.add_argument("--verify-runtime", type=Path, metavar="RESTORATION_JSON", help="run ONLY through the shared queue")
    args = parser.parse_args()
    if args.verify_runtime:
        return verify_runtime(args.verify_runtime.resolve())
    output = args.output.resolve()
    if not output.is_relative_to(TARGET.resolve()) or output == TARGET.resolve() or output.exists():
        parser.error("output must be a new directory inside this workspace target/")
    return restore(output)


if __name__ == "__main__":
    raise SystemExit(main())
