// Local-only portable UI preview. Run the normal release build and --stage-only
// first; never modifies release artifacts, creates an installer or launches UI.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { unzipSync, zipSync } from 'fflate';
import { root, version, hash, ensure, resetGenerated, filesBelow, executableArchitecture, run, write } from './common.mjs';
import { peImports, isVisualCppRuntime } from './pe-imports.mjs';

export function verifyPreviewContents(contents) {
  const keys = Object.keys(contents).sort();
  assert.deepEqual(keys.filter(k => /\.(html?|m?js|css|map|pdb)$/i.test(k) || /webview2|electron|chrome_elf|node_modules|\.DS_Store/i.test(k)), [], 'Preview must contain no browser runtime or development files');
  const json = path => JSON.parse(Buffer.from(contents[path]).toString('utf8'));
  const checksums = json('checksums.json');
  assert.equal(checksums.algorithm, 'sha256');
  assert.deepEqual(Object.keys(checksums.files).sort(), keys.filter(k => k !== 'checksums.json'), 'Every preview file, including launchers and README, must be indexed');
  for (const [key, expected] of Object.entries(checksums.files)) assert.equal(hash(contents[key]), expected, key);
  const assets = json('ui/assets.json');
  assert.equal(assets.coreEmotes.length, 9);
  assert.equal(keys.filter(k => k.startsWith('ui/emotes/')).length, 27);
  for (const [key, expected] of Object.entries(assets.files)) {
    assert.equal(hash(contents[`ui/${key}`]), expected.sha256, key);
    assert.equal(contents[`ui/${key}`].length, expected.size, key);
  }
  const runtime = json('licenses/windows-runtime.json');
  assert.ok(contents[runtime.license]?.length > 1000);
  for (const [name, bytes] of Object.entries(contents).filter(([name]) => /\.(exe|dll)$/i.test(name))) {
    for (const dependency of peImports(bytes).filter(isVisualCppRuntime)) assert.ok(contents[dependency]?.length > 0, `${name} needs ${dependency}`);
  }
  for (const dll of runtime.libraries) {
    assert.equal(hash(contents[dll.name]), dll.sha256);
    assert.equal(dll.signature, 'Valid');
    assert.equal(dll.architecture, 'x64');
  }
  for (const path of ['LICENSE', 'THIRD_PARTY_NOTICES.md', 'upstream.json', 'README-UI-PREVIEW.txt']) assert.ok(contents[path]?.length > 100, path);
  for (const path of ['licenses/Fluent-System-Icons-MIT.txt', 'licenses/Inter-OFL.txt', 'licenses/NotoSansCJK-OFL.txt']) {
    assert.ok(contents[path]?.length > 1000, `Native UI attribution is required: ${path}`);
  }
  for (const path of ['Start-UI-Preview.cmd', 'Stop-UI-Preview.cmd']) {
    const launcher = Buffer.from(contents[path]).toString('utf8');
    assert.ok(launcher.includes('setlocal DisableDelayedExpansion'));
    assert.ok(launcher.includes('--data-dir "%LOCALAPPDATA%\\EzrealByte\\LOLPingLite-UI-Preview\\data"'));
    assert.ok(launcher.includes('--root "%~dp0."'));
    assert.equal(launcher.includes('--quit'), path.startsWith('Stop-'));
  }
  const build = json('build-info.json');
  assert.equal(build.channel, 'ui-preview');
  assert.equal(build.version, version);
  assert.equal(build.renderer, 'native-rust');
  assert.match(build.sourceState.baseCommit, /^[a-f0-9]{40}$/);
  assert.match(build.sourceState.fingerprintSha256, /^[a-f0-9]{64}$/);
  assert.equal(build.sourceState.dirty, build.sourceState.changedFiles.length > 0);
  assert.equal(build.commit, build.sourceState.dirty ? null : build.sourceState.baseCommit, 'A dirty preview must not present the old release commit as its exact source');
  assert.equal(build.executableSha256, hash(contents['lolping-lite.exe']));
  return { build, indexedFiles: Object.keys(checksums.files).length };
}

function sourceState() {
  const git = args => run('git', args, { stdio: 'pipe' });
  const baseCommit = git(['rev-parse', 'HEAD']).toString().trim();
  const changed = git(['status', '--porcelain=v1', '-z', '--untracked-files=all']).toString('utf8').split('\0').filter(Boolean);
  const names = git(['ls-files', '-z', '--cached', '--others', '--exclude-standard']).toString('utf8').split('\0').filter(Boolean).sort();
  // Hash current source bytes, including new files. Git HEAD alone would falsely
  // identify a dirty preview binary as the already-published 0.1.0 source.
  const files = Object.fromEntries(names.map(name => {
    try { return [name, hash(readFileSync(join(root, name)))]; }
    catch (error) { if (error.code === 'ENOENT') return [name, null]; throw error; }
  }));
  return { baseCommit, dirty: changed.length > 0, changedFiles: changed,
    fingerprintSha256: hash(Buffer.from(JSON.stringify(files))),
    trackedAndUntrackedFileCount: names.length,
    meaning: 'Source-tree snapshot at packaging time, not a released commit or reproducible-build attestation.' };
}

function main() {
  assert.equal(process.platform, 'win32');
  assert.equal(process.arch, 'x64');
  const args = process.argv.slice(2);
  const at = args.indexOf('--stage');
  if (at >= 0) assert.ok(args[at + 1] && !args[at + 1].startsWith('--'), '--stage needs a directory');
  const stage = at < 0 ? join(root, '.stage/windows-x64') : resolve(args[at + 1]);
  assert.notEqual(stage.toLowerCase(), join(root, '.stage/windows-x64-ui-preview').toLowerCase(), 'Input stage must differ from the generated preview output');
  const original = Object.fromEntries(filesBelow(stage).map(file => [file.key, readFileSync(file.path)]));
  const oldChecksums = JSON.parse(Buffer.from(original['checksums.json']).toString('utf8'));
  for (const [name, digest] of Object.entries(oldChecksums.files)) assert.equal(hash(original[name]), digest, `Staged payload changed: ${name}`);
  for (const executable of ['lolping-lite.exe', 'hook-helper.exe']) assert.equal(executableArchitecture(join(stage, executable)), 'x64');
  assert.equal(hash(original['lolping-lite.exe']), hash(readFileSync(join(root, 'target/release/lolping-lite.exe'))), 'Stage the current release executable before packaging');
  const destination = resetGenerated('.stage/windows-x64-ui-preview', '.stage/windows-x64-ui-preview');
  for (const [name, bytes] of Object.entries(original)) write(join(destination, name), bytes);
  const info = JSON.parse(Buffer.from(original['build-info.json']).toString('utf8'));
  assert.equal(info.version, version);
  const source = sourceState();
  write(join(destination, 'build-info.json'), JSON.stringify({ ...info, channel: 'ui-preview',
    commit: source.dirty ? null : source.baseCommit, sourceState: source, executableSha256: hash(original['lolping-lite.exe']),
    packagedAt: new Date().toISOString(), preview: { localOnly: true, dataDirectory: '%LOCALAPPDATA%\\EzrealByte\\LOLPingLite-UI-Preview\\data',
      start: 'Start-UI-Preview.cmd', stop: 'Stop-UI-Preview.cmd', unsigned: true } }, null, 2) + '\n');
  const command = extra => `@echo off\r\nsetlocal DisableDelayedExpansion\r\nstart "" "%~dp0lolping-lite.exe" ${extra}--root "%~dp0." --data-dir "%LOCALAPPDATA%\\EzrealByte\\LOLPingLite-UI-Preview\\data"\r\n`;
  write(join(destination, 'Start-UI-Preview.cmd'), command(''));
  write(join(destination, 'Stop-UI-Preview.cmd'), command('--quit '));
  write(join(destination, 'README-UI-PREVIEW.txt'), `LOLPing Lite ${version} — UI Preview / 界面预览\n\n` +
    '这是本地 Windows x64 免安装预览包，不是新的正式发行版。完整解压后，双击 Start-UI-Preview.cmd。请使用此入口，不要直接双击 exe，否则会使用正式版的数据目录。\n' +
    '预览版使用独立目录 %LOCALAPPDATA%\\EzrealByte\\LOLPingLite-UI-Preview\\data，不读取或修改正式版配置。首次启动会显示设置；已在运行时再次启动会打开现有预览的设置，其他情况可从托盘打开。\n' +
    '关闭设置后仍可打信号；在托盘选择“退出”，或运行 Stop-UI-Preview.cmd 完全退出。测试前建议退出正式版，避免两个实例同时响应全局手势。\n' +
    '预览启动入口不安装、不修改系统启动项；请不要在预览测试中启用“开机自启”。预览结束后可删除解压目录和独立数据目录。\n' +
    `采用 Rust 原生设置与覆盖层，不包含 WebView、React 或 Electron。版本 ${version}；build-info.json 标注 ui-preview、源代码工作树状态及哈希。未商业签名。\n\n` +
    'This is a local, unsigned Windows x64 UI preview, not a new public release. Extract the complete ZIP and run Start-UI-Preview.cmd. Do not launch the executable directly: that uses the regular app data directory.\n' +
    'The launcher uses an independent data directory under %LOCALAPPDATA%\\EzrealByte\\LOLPingLite-UI-Preview\\data. Existing release settings are left untouched. First launch shows settings; launching an already-running preview reopens its settings. Otherwise, open settings from the tray.\n' +
    'Closing settings keeps pings running. Choose Quit in the tray or run Stop-UI-Preview.cmd to stop the whole preview. Quit the regular version before gesture testing so both instances do not respond together.\n' +
    `No installer is run. Leave Start at login disabled during preview testing. Remove the extracted folder and preview data directory when finished. Version ${version}, with explicit source-tree and binary hashes in build-info.json.\n`);
  const checksumFiles = Object.fromEntries(filesBelow(destination).filter(file => file.key !== 'checksums.json').map(file => [file.key, hash(readFileSync(file.path))]));
  write(join(destination, 'checksums.json'), JSON.stringify({ algorithm: 'sha256', files: checksumFiles }, null, 2) + '\n');
  const payload = Object.fromEntries(filesBelow(destination).map(file => [file.key, readFileSync(file.path)]));
  verifyPreviewContents(payload);
  const dist = ensure(join(root, 'dist'));
  const name = `LOLPing-Lite-${version}-ui-preview-windows-x64.zip`;
  const zip = zipSync(payload, { level: 9, mtime: new Date(2020, 0, 1) });
  write(join(dist, name), zip);
  const verification = verifyPreviewContents(unzipSync(readFileSync(join(dist, name))));
  const artifact = { file: name, size: zip.length, sha256: hash(zip), kind: 'portable-ui-preview', unsigned: true };
  const metadata = { product: 'LOLPing Lite', version, channel: 'ui-preview', target: 'windows-x64',
    stagedBytes: filesBelow(destination).reduce((total, file) => total + file.size, 0), sourceState: verification.build.sourceState, artifacts: [artifact] };
  write(join(dist, 'windows-x64-ui-preview.artifact.json'), JSON.stringify(metadata, null, 2) + '\n');
  write(join(dist, 'SHA256SUMS-ui-preview-windows-x64.txt'), `${artifact.sha256}  ${name}\n`);
  write(join(dist, 'windows-x64-ui-preview-package-report.json'), JSON.stringify({ passed: true, ...metadata,
    checks: ['Every archive entry has a SHA-256 manifest entry', '9 core emotes and all source assets verified',
      'x64 native executables and app-local Microsoft VC runtime verified', 'Licenses preserved',
      'No browser runtime or development code', 'Separate preview startup and shutdown data directory',
      'Local preview provenance records current source tree rather than claiming a release commit'], indexedFiles: verification.indexedFiles }, null, 2) + '\n');
  console.log(JSON.stringify({ artifact, stage: destination, sourceState: metadata.sourceState }, null, 2));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
