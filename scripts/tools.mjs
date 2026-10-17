import { chmodSync, existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve, relative } from 'node:path';
import { unzipSync } from 'fflate';
import { root, ensure, hash, write } from './common.mjs';

const specs = JSON.parse(readFileSync(join(root, 'packaging/tools.json'), 'utf8'));
export async function tool(name) {
  const spec = specs[name];
  if (!spec) throw Error(`Unknown packaging tool ${name}`);
  const cache = ensure(join(root, '.cache/packaging'));
  const path = join(cache, spec.file);
  if (!existsSync(path) || hash(readFileSync(path)) !== spec.sha256) {
    const response = await fetch(spec.url, { signal: AbortSignal.timeout(120_000) });
    if (!response.ok) throw Error(`Cannot download ${name}: ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    if (hash(bytes) !== spec.sha256) throw Error(`${name} SHA-256 mismatch`);
    writeFileSync(path, bytes);
  }
  if (name !== 'nsis') { chmodSync(path, 0o755); return path; }
  const dest = ensure(join(cache, `nsis-${spec.version}`));
  for (const [key, bytes] of Object.entries(unzipSync(readFileSync(path)))) {
    if (key.endsWith('/')) continue;
    const trimmed = key.replace(`nsis-${spec.version}/`, '');
    const output = resolve(dest, trimmed), rel = relative(dest, output);
    if (rel.startsWith('..') || key.includes('\\') || key.startsWith('/')) throw Error(`Unsafe NSIS archive path: ${key}`);
    write(output, bytes);
  }
  return join(dest, 'makensis.exe');
}
