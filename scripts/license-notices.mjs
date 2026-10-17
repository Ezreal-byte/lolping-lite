import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { root, run, write } from './common.mjs';

// Preserve the actual copyright and redistribution text from the Cargo sources
// selected for this platform, rather than treating SPDX names as license text.
export function writeRustNotices(destination, targetOverride) {
  const triple = targetOverride || run('rustc', ['-vV'], { stdio: 'pipe' }).toString().match(/^host: (.+)$/m)?.[1];
  if (!triple) throw Error('Cannot determine Rust host target for license inventory');
  const metadata = JSON.parse(run('cargo', ['metadata', '--locked', '--format-version', '1', '--filter-platform', triple], { stdio: 'pipe', maxBuffer: 32 * 1024 * 1024 }));
  // cargo metadata can retain optional edges in its unified workspace graph.
  // cargo tree supplies the dependency set actually enabled for this target.
  const tree = run('cargo', ['tree', '--locked', '--target', triple, '--edges', 'normal', '--prefix', 'none', '--format', '{p}'], { stdio: 'pipe', maxBuffer: 8 * 1024 * 1024 }).toString();
  const selected = new Set(tree.split('\n').flatMap(line => { const m = line.match(/^(\S+) v(\S+)/); return m ? [`${m[1]}@${m[2]}`] : []; }));
  const records = [];
  for (const pkg of metadata.packages) {
    if (!selected.has(`${pkg.name}@${pkg.version}`) || metadata.workspace_members.includes(pkg.id)) continue;
    const dir = dirname(pkg.manifest_path);
    const paths = readdirSync(dir, { withFileTypes: true }).filter(entry => entry.isFile() && /^(licen[cs]e|copying|copyright|notice)([-_.]|$)/i.test(entry.name)).map(entry => join(dir, entry.name));
    if (pkg.license_file) { const path = join(dir, pkg.license_file); if (existsSync(path) && !paths.includes(path)) paths.push(path); }
    if (pkg.name === 'epaint_default_fonts') {
      for (const file of readdirSync(join(dir, 'fonts')).filter(f => f.endsWith('.txt'))) paths.push(join(dir, 'fonts', file));
    }
    const overrides = [
      [/^https:\/\/github.com\/emilk\/egui(?:\/|$)/, 'egui-MIT.txt'],
      [/^https:\/\/github.com\/AccessKit\/accesskit(?:\/|$)/, 'AccessKit-MIT.txt'],
      [/^https:\/\/github.com\/rustaudio\/sample/, 'dasp-MIT.txt'],
      [/^https:\/\/github.com\/aclysma\/profiling/, 'profiling-MIT.txt'],
      [/^https:\/\/github.com\/pdeljanov\/Symphonia/, 'Symphonia-MPL-2.0.txt'],
      [/^https:\/\/github.com\/DoumanAsh\/clipboard-win/, 'clipboard-win-BSL-1.0.txt'],
      [/^https:\/\/github.com\/madsmtm\/objc2/, 'objc2-MIT.txt'],
      [/^https:\/\/github.com\/OpenByteDev\/dlopen2/, 'dlopen2-MIT.txt'],
    ];
    const override = pkg.name === 'libappindicator-sys' ? 'libappindicator-MIT.txt' : overrides.find(([pattern]) => pattern.test(pkg.repository || ''))?.[1];
    if (override) paths.push(join(root, 'licenses', override));
    const label = path => path.startsWith(dir) ? path.slice(dir.length + 1) : `project/licenses/${path.split(/[\\/]/).at(-1)}`;
    const texts = paths.map(path => `--- ${label(path)} ---\n${readFileSync(path, 'utf8').trim()}`).join('\n\n');
    records.push({ name: pkg.name, version: pkg.version, license: pkg.license, repository: pkg.repository, authors: pkg.authors, licenseFiles: paths.map(label), text: texts });
  }
  records.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
  const missing = records.filter(r => !r.licenseFiles.length);
  const body = ['LOLPing Lite — Cargo dependency notices', `Target: ${triple}`, 'This inventory follows normal dependencies for the target, including compile-time macros. Build tools and development tests are not distributed.', ...records.map(r => `\n${'='.repeat(72)}\n${r.name} ${r.version}\nLicense: ${r.license || 'See source'}\nAuthors (Cargo metadata): ${r.authors.join(', ')}\nSource: ${r.repository || `https://crates.io/crates/${r.name}/${r.version}`}\n\n${r.text || 'The published crate contains no standalone license file. Consult its source repository and SPDX license declaration above.'}`)].join('\n');
  write(join(destination, 'rust-dependencies.txt'), body + '\n');
  write(join(destination, 'rust-dependencies.json'), JSON.stringify({ target: triple, crates: records.map(({ text, ...r }) => r), missingStandaloneLicenseFiles: missing.map(r => `${r.name}@${r.version}`) }, null, 2) + '\n');
  console.log(`Retained license notices for ${records.length} Cargo packages (${missing.length} have no standalone license file in their published crate).`);
  return missing;
}
