// Read the PE import directory without loading executable code or relying on
// dumpbin being present on the consumer's machine.
export function peImports(bytes) {
  const b = Buffer.from(bytes);
  if (b.toString('ascii', 0, 2) !== 'MZ') throw Error('Expected a PE executable');
  const pe = b.readUInt32LE(0x3c);
  if (b.readUInt32LE(pe) !== 0x4550) throw Error('Invalid PE signature');
  const sections = b.readUInt16LE(pe + 6), optionalSize = b.readUInt16LE(pe + 20);
  const optional = pe + 24, magic = b.readUInt16LE(optional);
  if (![0x10b, 0x20b].includes(magic)) throw Error('Unsupported PE optional header');
  const directories = optional + (magic === 0x20b ? 112 : 96);
  const table = optional + optionalSize;
  function offset(rva) {
    for (let i = 0; i < sections; i++) {
      const s = table + i * 40, start = b.readUInt32LE(s + 12);
      const size = Math.max(b.readUInt32LE(s + 8), b.readUInt32LE(s + 16));
      if (rva >= start && rva < start + size) return b.readUInt32LE(s + 20) + rva - start;
    }
    throw Error(`PE RVA is outside sections: ${rva}`);
  }
  function name(rva) {
    const start = offset(rva), end = b.indexOf(0, start);
    if (end < start || end - start > 255) throw Error('Invalid PE import name');
    return b.toString('ascii', start, end).toLowerCase();
  }
  const imports = new Set();
  const regular = b.readUInt32LE(directories + 8);
  if (regular) {
    for (let p = offset(regular); b.readUInt32LE(p + 12); p += 20) imports.add(name(b.readUInt32LE(p + 12)));
  }
  const delayed = b.readUInt32LE(directories + 13 * 8);
  if (delayed) {
    for (let p = offset(delayed); b.readUInt32LE(p + 4); p += 32) {
      if (!(b.readUInt32LE(p) & 1)) throw Error('Unsupported VA-based delayed PE imports');
      imports.add(name(b.readUInt32LE(p + 4)));
    }
  }
  return [...imports].sort();
}

export const isVisualCppRuntime = name => /^(?:vcruntime|msvcp|concrt|vccorlib|mfc|mfcm)\d[^/\\]*\.dll$/i.test(name);
