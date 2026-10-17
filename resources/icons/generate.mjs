// Development-only: extract unmodified Fluent paths from the pinned npm package.
// Usage: node generate.mjs <unpacked-package-dir> [absolute-path-to-sharp]
import assert from 'node:assert/strict';
import { readFile, readdir, mkdir, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const sharp = require(process.argv[3] || 'sharp');
const packageRoot = path.resolve(process.argv[2] || '');
const metadata = JSON.parse(await readFile(path.join(packageRoot, 'package.json'), 'utf8'));
assert.equal(metadata.name, '@fluentui/react-icons');
assert.equal(metadata.version, '2.0.343');
assert.equal(metadata.license, 'MIT');
const sourceDir = path.join(packageRoot, 'lib', 'sizedIcons');
const sources = (await Promise.all((await readdir(sourceDir)).filter(p => p.endsWith('.js'))
  .map(p => readFile(path.join(sourceDir, p), 'utf8')))).join('\n');
const icons = {
  trigger: 'Cursor24Regular', hotkey: 'Keyboard24Regular', sound: 'Speaker224Regular',
  wheel: 'DataPie24Regular', emotes: 'Emoji24Regular', app: 'Settings24Regular',
  play: 'Play24Regular', pause: 'Pause24Regular', stop: 'Stop24Regular',
  check: 'Checkmark24Regular', download: 'ArrowDownload24Regular', upload: 'ArrowUpload24Regular',
  search: 'Search24Regular', record: 'Record24Regular', close: 'Dismiss24Regular',
};
const output = path.dirname(fileURLToPath(import.meta.url));
await mkdir(output, { recursive: true });
for (const [name, symbol] of Object.entries(icons)) {
  const match = sources.match(new RegExp(`createFluentIcon\\('${symbol}', "24", (\\[[^\\n]+?\\])\\)`));
  assert(match, `Missing static SVG paths for ${symbol}`);
  const paths = JSON.parse(match[1]);
  assert(paths.length && paths.every(p => typeof p === 'string' && /^[A-Za-z0-9., \-]+$/.test(p)));
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="#fff">${paths.map(d => `<path d="${d}"/>`).join('')}</svg>`;
  await sharp(Buffer.from(svg), { density: 288 }).resize(96, 96)
    .png({ compressionLevel: 9, adaptiveFiltering: false }).toFile(path.join(output, `${name}.png`));
}
await writeFile(path.join(output, 'sources.json'), JSON.stringify({
  package: metadata.name, version: metadata.version, license: metadata.license,
  repository: 'https://github.com/microsoft/fluentui-system-icons',
  tarball: 'https://registry.npmjs.org/@fluentui/react-icons/-/react-icons-2.0.343.tgz',
  pixelSize: 96, originalViewBox: '0 0 24 24', icons,
}, null, 2) + '\n');
console.log(`Generated ${Object.keys(icons).length} transparent Fluent PNGs at 96 × 96.`);
