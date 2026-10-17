import { copyFileSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { root, ensure, resetGenerated, hash, write } from './common.mjs';
import { windowsIcon } from './icon.mjs';

const defaults = JSON.parse(readFileSync(join(root, 'resources/default-settings.json'), 'utf8'));
const upstream = JSON.parse(readFileSync(join(root, 'resources/upstream.json'), 'utf8'));
const core = new Set([...defaults.emoteWheel, defaults.clickEmoteId]);
if (core.size !== 9) throw Error(`Expected 9 core emotes, got ${core.size}`);
const ui = resetGenerated('ui', 'ui');
const index = {};
for (const [key, expected] of Object.entries(upstream.files)) {
  if (!/^(emotes|sounds|textures)\/[a-zA-Z0-9_.-]+$/.test(key)) throw Error(`Invalid source asset path: ${key}`);
  const source = join(root, 'resources', key), bytes = readFileSync(source);
  if (bytes.length !== expected.size || hash(bytes) !== expected.sha256) throw Error(`Upstream asset differs: ${key}`);
  if (key.startsWith('emotes/') && !core.has(key.split('/')[1].split('.')[0])) continue;
  ensure(join(ui, key.split('/')[0]));
  copyFileSync(source, join(ui, key));
  index[key] = expected;
}
write(join(ui, 'assets.json'), JSON.stringify({ upstream: upstream.revision, coreEmotes: [...core], files: index }, null, 2) + '\n');
// Checked in for `cargo test` on a clean checkout; regenerate deterministically
// when the canonical PNG changes. It is artwork, not a compiled executable.
write(join(root, 'assets/app.ico'), windowsIcon(readFileSync(join(root, 'assets/plugin.png'))));
console.log(`Verified all upstream assets; copied ${Object.keys(index).length} native assets, including ${core.size} default emotes. No web resources.`);
