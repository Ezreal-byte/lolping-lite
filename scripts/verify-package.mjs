import assert from 'node:assert/strict';
import { readFileSync, existsSync, lstatSync } from 'node:fs';
import { join } from 'node:path';
import { unzipSync } from 'fflate';
import { root, version, target, hash, resetGenerated, executableArchitecture, run, write } from './common.mjs';
import { peImports, isVisualCppRuntime } from './pe-imports.mjs';
import { diskImage } from './disk-image.mjs';

const dist = join(root, 'dist');
const metadata = JSON.parse(readFileSync(join(dist, `${target}.artifact.json`), 'utf8'));
assert.equal(metadata.version, version);
assert.equal(metadata.target, target);
const checks = [];
for (const artifact of metadata.artifacts) {
  const path = join(dist, artifact.file), bytes = readFileSync(path);
  assert.equal(hash(bytes), artifact.sha256, artifact.file);
  assert.equal(bytes.length, artifact.size, artifact.file);
  checks.push(`${artifact.file}: SHA-256 and length verified`);
  if (artifact.kind === 'optional-emotes') {
    const contents = unzipSync(bytes), pack = JSON.parse(Buffer.from(contents['pack.json']).toString('utf8'));
    assert.equal(pack.id, 'lolping-extra-emotes');
    assert.equal(pack.version, 1);
    assert.equal(Object.keys(pack.files).length, 105);
    assert.equal(Object.keys(contents).length, 106);
    for (const [key, expected] of Object.entries(pack.files)) { assert.equal(hash(contents[key]), expected.sha256, key); assert.equal(contents[key].length, expected.size, key); }
    checks.push('35 optional emotes: every indexed source hash verified');
  } else if (artifact.kind === 'portable') {
    const contents = unzipSync(bytes);
    verifyContents(contents, '');
    assert.equal(Buffer.from(contents['lolping-lite.exe']).toString('ascii', 0, 2), 'MZ');
    const runtime = JSON.parse(Buffer.from(contents['licenses/windows-runtime.json']).toString('utf8'));
    assert.ok(contents[runtime.license]?.length > 1000);
    for (const [name, bytes] of Object.entries(contents).filter(([name]) => /\.(exe|dll)$/i.test(name))) {
      for (const dependency of peImports(bytes).filter(isVisualCppRuntime)) assert.ok(contents[dependency]?.length > 0, `${name} requires application-local ${dependency}`);
    }
    for (const dll of runtime.libraries) { assert.equal(hash(contents[dll.name]), dll.sha256); assert.equal(dll.signature, 'Valid'); assert.equal(dll.architecture, 'x64'); }
    checks.push('Windows VC runtime: all PE imports resolved application-locally with Microsoft-signed DLLs, hashes and license');
    checks.push('Portable ZIP: native executables, 9 core emotes, asset hashes, no browser assets');
  } else if (artifact.kind === 'installer') {
    // NSIS installers use a 32-bit bootstrap even when the bundled app is x64.
    assert.equal(bytes.toString('ascii', 0, 2), 'MZ');
    assert.ok(bytes.length > 1024 * 1024);
    checks.push('Windows user installer: PE signature and complete payload size');
  } else if (artifact.kind === 'disk-image') {
    await diskImage(['verify', path]);
    const mount = resetGenerated('.cache/verify/macos-mount', '.cache/verify/macos-mount');
    await diskImage(['attach', '-readonly', '-nobrowse', '-mountpoint', mount, path]);
    try {
      const bundle = join(mount, 'LOLPing Lite.app');
      run('codesign', ['--verify', '--deep', '--strict', bundle]);
      verifyTree(bundle, join(mount, 'checksums.json'), 'Contents/Resources/', 'Contents/MacOS/');
    } finally { await diskImage(['detach', mount]); }
    checks.push('DMG: mounted read-only, signed arm64 app and native asset hashes verified');
  } else if (artifact.kind === 'debian') {
    const dir = resetGenerated('.cache/verify/deb', '.cache/verify/deb');
    run('dpkg-deb', ['-x', path, dir]);
    verifyTree(dir, join(dir, 'usr/share/lolping-lite/checksums.json'), 'usr/share/lolping-lite/', 'usr/bin/');
    checks.push('Debian: extracted x64 executables and complete assets verified');
  } else if (artifact.kind === 'appimage') {
    const dir = resetGenerated('.cache/verify/appimage', '.cache/verify/appimage');
    run(path, ['--appimage-extract'], { cwd: dir, stdio: 'pipe' });
    const appdir = join(dir, 'squashfs-root');
    assert.ok(lstatSync(join(appdir, '.DirIcon')).isFile(), 'AppImage icon is a regular file, not an external symlink');
    assert.equal(hash(readFileSync(join(appdir, '.DirIcon'))), hash(readFileSync(join(root, 'assets/plugin.png'))));
    verifyTree(appdir, join(appdir, 'usr/share/lolping-lite/checksums.json'), 'usr/share/lolping-lite/', 'usr/bin/');
    assert.ok(existsSync(join(appdir, 'usr/lib/libgtk-3.so.0')));
    assert.ok(existsSync(join(appdir, 'usr/lib/libayatana-appindicator3.so.1')));
    assert.ok(existsSync(join(appdir, 'usr/share/lolping-lite/licenses/linux-bundled/manifest.json')));
    run(join(appdir, 'AppRun'), ['--self-test']);
    checks.push('AppImage: extracted payload, native dependencies and self-test verified');
  }
}
function verifyContents(contents, prefix) {
  const forbidden = Object.keys(contents).filter(k => /\.(html?|m?js|css|map|pdb)$/i.test(k) || /webview2|electron|chrome_elf|node_modules|\.DS_Store/i.test(k));
  assert.deepEqual(forbidden, [], 'No web runtime, development dependencies, or debug artifacts');
  const manifest = JSON.parse(Buffer.from(contents[`${prefix}checksums.json`]).toString('utf8'));
  for (const [key, expected] of Object.entries(manifest.files)) assert.equal(hash(contents[key]), expected, key);
  const assetIndex = JSON.parse(Buffer.from(contents[`${prefix}ui/assets.json`]).toString('utf8'));
  assert.equal(assetIndex.coreEmotes.length, 9);
  assert.equal(Object.keys(contents).filter(k => k.startsWith(`${prefix}ui/emotes/`) && !k.endsWith('/')).length, 27);
  for (const [key, expected] of Object.entries(assetIndex.files)) assert.equal(hash(contents[`${prefix}ui/${key}`]), expected.sha256, key);
  for (const name of ['LICENSE', 'THIRD_PARTY_NOTICES.md', 'upstream.json', 'build-info.json']) assert.ok(contents[`${prefix}${name}`]?.length > 100, name);
}
function verifyTree(base, manifestPath, dataPrefix, binPrefix) {
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  for (const [key, expected] of Object.entries(manifest.files)) assert.equal(hash(readFileSync(join(base, key))), expected, key);
  const assets = JSON.parse(readFileSync(join(base, dataPrefix, 'ui/assets.json'), 'utf8'));
  assert.equal(assets.coreEmotes.length, 9);
  for (const [key, expected] of Object.entries(assets.files)) assert.equal(hash(readFileSync(join(base, dataPrefix, 'ui', key))), expected.sha256, key);
  assert.equal(executableArchitecture(join(base, binPrefix, 'lolping-lite')), process.arch);
  assert.equal(executableArchitecture(join(base, binPrefix, 'hook-helper')), process.arch);
  for (const key of Object.keys(manifest.files)) assert.ok(!/\.(html?|m?js|css|map|pdb)$/i.test(key), key);
}
const report = { version, target, passed: true, checks, artifacts: metadata.artifacts, desktopValidation: 'See standalone-smoke and platform regression reports; package integrity is not a full desktop acceptance test.' };
write(join(dist, `${target}-package-report.json`), JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify(report, null, 2));
