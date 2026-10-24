"""Read protocol API descriptors from class files without executing the target."""
from __future__ import annotations

import argparse
import io
import json
import struct
import zipfile

import native

CLASSES = (
    "net.minecraft.network.ProtocolInfo",
    "net.minecraft.network.ProtocolInfo$Unbound",
    "net.minecraft.network.protocol.login.LoginProtocols",
    "net.minecraft.network.protocol.configuration.ConfigurationProtocols",
    "net.minecraft.network.protocol.game.GameProtocols",
    "net.minecraft.network.RegistryFriendlyByteBuf",
    "net.minecraft.resources.RegistryDataLoader",
    "net.minecraft.core.RegistryAccess$ImmutableRegistryAccess",
    "net.minecraft.core.RegistryAccess$RegistryEntry",
    "net.minecraft.network.protocol.configuration.ClientboundRegistryDataPacket",
)


def descriptors(data: bytes) -> dict:
    stream = io.BytesIO(data)

    def take(size: int) -> bytes:
        value = stream.read(size)
        if len(value) != size:
            raise ValueError("truncated class file")
        return value

    def number(fmt: str) -> int:
        return struct.unpack(fmt, take(struct.calcsize(fmt)))[0]

    if take(4) != b"\xca\xfe\xba\xbe":
        raise ValueError("not a class file")
    minor, major = number(">H"), number(">H")
    pool = [None] * number(">H")
    index = 1
    widths = {3: 4, 4: 4, 5: 8, 6: 8, 7: 2, 8: 2, 9: 4, 10: 4,
              11: 4, 12: 4, 15: 3, 16: 2, 17: 4, 18: 4, 19: 2, 20: 2}
    while index < len(pool):
        tag = number(">B")
        if tag == 1:
            # Member names and descriptors use the ASCII subset of modified UTF-8.
            pool[index] = take(number(">H"))
        elif tag in widths:
            take(widths[tag])
        else:
            raise ValueError(f"unsupported constant-pool tag {tag}")
        index += 2 if tag in (5, 6) else 1
    take(6)
    take(2 * number(">H"))

    def members() -> list[dict]:
        result = []
        for _ in range(number(">H")):
            flags, name, descriptor = number(">H"), number(">H"), number(">H")
            for _ in range(number(">H")):
                take(2)
                take(number(">I"))
            result.append({"name": pool[name].decode("ascii"),
                           "descriptor": pool[descriptor].decode("ascii"), "flags": flags})
        return result

    return {"class_version": [major, minor], "fields": members(), "methods": members()}


def inspect(jar, classes=CLASSES) -> dict:
    with zipfile.ZipFile(jar) as archive:
        result = {}
        for name in classes:
            member = name.replace(".", "/") + ".class"
            if member not in archive.namelist():
                prefix = member.split("$", 1)[0].removesuffix(".class")
                result[name] = {"missing": True, "candidates": [entry for entry in archive.namelist()
                                 if entry.startswith(prefix) and entry.endswith(".class")]}
            else:
                result[name] = descriptors(archive.read(member))
        return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("classes", nargs="*", default=CLASSES)
    args = parser.parse_args()
    if native.sha(native.JAR) != native.JAR_SHA256:
        raise ValueError("official 26.2 JAR digest differs")
    print(json.dumps({"jar_sha256": native.JAR_SHA256,
                      "classes": inspect(native.JAR, args.classes)}, indent=2))


if __name__ == "__main__":
    main()
