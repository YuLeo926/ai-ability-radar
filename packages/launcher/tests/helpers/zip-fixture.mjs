import { createHash } from "node:crypto";

import { validateReleaseManifest } from "../../lib/manifest.mjs";

function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

export function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

export function createStoredZip(entries, { diskNumber = 0, comment = Buffer.alloc(0) } = {}) {
  const localParts = [];
  const centralParts = [];
  let localOffset = 0;
  for (const source of entries) {
    const nameBytes = source.nameBytes ?? Buffer.from(source.name, "utf8");
    const data = Buffer.from(source.data ?? Buffer.alloc(0));
    const flags = source.flags ?? 0;
    const method = source.method ?? 0;
    const versionNeeded = source.versionNeeded ?? 20;
    const checksum = source.crc32 ?? crc32(data);
    const compressedSize = source.compressedSize ?? data.length;
    const uncompressedSize = source.uncompressedSize ?? data.length;
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(versionNeeded, 4);
    local.writeUInt16LE(flags, 6);
    local.writeUInt16LE(method, 8);
    local.writeUInt32LE(checksum, 14);
    local.writeUInt32LE(compressedSize, 18);
    local.writeUInt32LE(uncompressedSize, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    localParts.push(local, nameBytes, data);

    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(source.versionMadeBy ?? 20, 4);
    central.writeUInt16LE(versionNeeded, 6);
    central.writeUInt16LE(flags, 8);
    central.writeUInt16LE(method, 10);
    central.writeUInt32LE(checksum, 16);
    central.writeUInt32LE(compressedSize, 20);
    central.writeUInt32LE(uncompressedSize, 24);
    central.writeUInt16LE(nameBytes.length, 28);
    central.writeUInt32LE((source.externalAttributes ?? (source.name?.endsWith("/") ? 0x10 : 0x20)) >>> 0, 38);
    central.writeUInt32LE(localOffset, 42);
    centralParts.push(central, nameBytes);
    localOffset += local.length + nameBytes.length + data.length;
  }
  const centralOffset = localOffset;
  const centralSize = centralParts.reduce((total, part) => total + part.length, 0);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(diskNumber, 4);
  end.writeUInt16LE(diskNumber, 6);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(centralSize, 12);
  end.writeUInt32LE(centralOffset, 16);
  end.writeUInt16LE(comment.length, 20);
  return Buffer.concat([...localParts, ...centralParts, end, comment]);
}

function byteSort(left, right) {
  return Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8"));
}

export function createPortableFixture({
  checksumText,
  readmeText = "portable fixture\n",
  executableText = "MZ fake executable bytes",
} = {}) {
  const readme = Buffer.from(readmeText);
  const executable = Buffer.from(executableText);
  const payloads = new Map([
    ["ability-radar-portable/README.txt", readme],
    ["ability-radar-portable/ability-radar.exe", executable],
  ]);
  const generatedChecksums = [...payloads]
    .map(([path, bytes]) => `${sha256(bytes)}  ${path.slice("ability-radar-portable/".length)}`)
    .sort()
    .join("\n") + "\n";
  payloads.set(
    "ability-radar-portable/SHA256SUMS.txt",
    Buffer.from(checksumText ?? generatedChecksums),
  );
  const orderedFiles = [...payloads].sort(([left], [right]) => byteSort(left, right));
  const archive = createStoredZip([
    { name: "ability-radar-portable/" },
    ...orderedFiles.map(([name, data]) => ({ name, data })),
  ]);
  const manifest = validateReleaseManifest(
    {
      schema_version: "launcher-release-manifest-v1",
      repository: "YuLeo926/ai-ability-radar",
      launcher_version: "0.2.2",
      desktop_version: "0.2.2",
      tag: "v0.2.2",
      assets: {
        portable: {
          file_name: "ability-radar_0.2.2_windows-x64-portable.zip",
          size: archive.length,
          sha256: sha256(archive),
          root_directory: "ability-radar-portable",
          executable: "ability-radar-portable/ability-radar.exe",
          files: orderedFiles.map(([path, bytes]) => ({
            path,
            size: bytes.length,
            sha256: sha256(bytes),
          })),
        },
        checksums: { file_name: "SHA256SUMS.txt" },
      },
    },
    { packageVersion: "0.2.2" },
  );
  return { archive, manifest, payloads };
}
