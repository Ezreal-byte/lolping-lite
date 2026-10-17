import { createHash } from 'node:crypto';
import { mkdirSync, rmSync, lstatSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve, relative, join, dirname, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const version = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version;
export const target = ({ 'win32-x64': 'windows-x64', 'darwin-arm64': 'macos-arm64', 'linux-x64': 'linux-x64' })[`${process.platform}-${process.arch}`];
export const hash = bytes => createHash('sha256').update(bytes).digest('hex');
export function run(command, args, options = {}) {
  return execFileSync(command, args, { cwd: root, stdio: 'inherit', ...options });
}
export function ensure(dir) { mkdirSync(dir, { recursive: true }); return dir; }
export function write(path, data) { ensure(dirname(path)); writeFileSync(path, data); }
// Only exact, generated paths beneath the repository may be recursively replaced.
export function resetGenerated(path, expected) {
  const full = resolve(root, path), rel = relative(root, full).replaceAll('\\', '/');
  if (rel !== expected || rel.startsWith('../') || isAbsolute(rel)) throw Error(`Unsafe generated path: ${path}`);
  let cursor = root;
  for (const part of rel.split('/')) {
    cursor = join(cursor, part);
    try { if (lstatSync(cursor).isSymbolicLink()) throw Error(`Generated path must not be a symlink: ${cursor}`); }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
  }
  rmSync(full, { recursive: true, force: true });
  return ensure(full);
}
export function filesBelow(dir) {
  const files = [];
  function walk(current) {
    for (const name of readdirSync(current).sort()) {
      const path = join(current, name), stat = lstatSync(path);
      if (stat.isSymbolicLink()) throw Error(`Unexpected symlink: ${path}`);
      if (stat.isDirectory()) walk(path);
      else files.push({ path, key: relative(dir, path).replaceAll('\\', '/'), size: stat.size, mode: stat.mode });
    }
  }
  walk(dir);
  return files;
}
export function executableArchitecture(path) {
  const b = readFileSync(path);
  if (b[0] === 0x4d && b[1] === 0x5a) {
    const offset = b.readUInt32LE(0x3c);
    if (b.readUInt32LE(offset) !== 0x4550) throw Error(`Invalid PE executable: ${path}`);
    return ({ 0x8664: 'x64', 0xaa64: 'arm64' })[b.readUInt16LE(offset + 4)];
  }
  if (b.readUInt32LE(0) === 0xfeedfacf) return ({ 0x1000007: 'x64', 0x100000c: 'arm64' })[b.readUInt32LE(4)];
  if (b.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) return ({ 62: 'x64', 183: 'arm64' })[b.readUInt16LE(18)];
  throw Error(`Unknown executable format: ${path}`);
}
