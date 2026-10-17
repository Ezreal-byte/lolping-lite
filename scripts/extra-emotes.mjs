import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { zipSync, strToU8 } from 'fflate';
import { root, hash, write } from './common.mjs';

export function buildExtraEmotes() {
  const upstream = JSON.parse(readFileSync(join(root, 'resources/upstream.json'), 'utf8'));
  const defaults = JSON.parse(readFileSync(join(root, 'resources/default-settings.json'), 'utf8'));
  const core = new Set([...defaults.emoteWheel, defaults.clickEmoteId]);
  const extra = {}, index = {};
  for (const [key, expected] of Object.entries(upstream.files)) {
    if (!key.startsWith('emotes/') || core.has(key.split('/')[1].split('.')[0])) continue;
    if (!/^emotes\/[a-zA-Z0-9_.-]+$/.test(key)) throw Error(`Invalid extension asset path: ${key}`);
    const bytes = readFileSync(join(root, 'resources', key));
    if (bytes.length !== expected.size || hash(bytes) !== expected.sha256) throw Error(`Invalid extension asset: ${key}`);
    extra[key] = bytes; index[key] = expected;
  }
  if (Object.keys(index).length !== 105) throw Error('Expected 35 optional emotes');
  extra['pack.json'] = strToU8(JSON.stringify({ version: 1, id: 'lolping-extra-emotes', upstream: upstream.revision, files: index }, null, 2));
  const path = join(root, 'dist/lolping-extra-emotes-1.zip');
  // ZIP stores a timezone-less DOS timestamp; use the same local clock fields
  // on every builder, including developers whose timezone is not UTC.
  write(path, zipSync(extra, { level: 9, mtime: new Date(2020, 0, 1, 0, 0, 0) }));
  return path;
}
