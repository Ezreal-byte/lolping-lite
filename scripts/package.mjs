import { chmodSync, copyFileSync, cpSync, existsSync, readFileSync, readdirSync, symlinkSync } from 'node:fs';
import { basename, dirname, join, relative } from 'node:path';
import { zipSync } from 'fflate';
import { root, version, target, hash, ensure, resetGenerated, filesBelow, executableArchitecture, run, write } from './common.mjs';
import { windowsIcon } from './icon.mjs';
import { tool } from './tools.mjs';
import { buildExtraEmotes } from './extra-emotes.mjs';
import { writeRustNotices } from './license-notices.mjs';
import { diskImage } from './disk-image.mjs';

if (!target) throw Error(`Unsupported packaging host: ${process.platform}/${process.arch}`);
const stageBase = resetGenerated(`.stage/${target}`, `.stage/${target}`);
const dist = ensure(join(root, 'dist'));
const suffix = process.platform === 'win32' ? '.exe' : '';
const profile = process.argv.includes('--debug') ? 'debug' : 'release';
const exe = join(root, 'target', profile, `lolping-lite${suffix}`);
const helper = join(root, 'native/hook-helper/build', `hook-helper${suffix}`);
for (const path of [exe, helper]) {
  if (executableArchitecture(path) !== process.arch) throw Error(`Wrong architecture: ${path}`);
}
if (!existsSync(join(root, 'ui/assets.json'))) throw Error('Run npm run build:resources first');
const stem = `LOLPing-Lite-${version}-${target}`;
const appName = 'LOLPing Lite.app';
const stage = process.platform === 'darwin' ? ensure(join(stageBase, appName)) : process.platform === 'linux' ? ensure(join(stageBase, 'AppDir')) : stageBase;
const data = process.platform === 'darwin' ? ensure(join(stage, 'Contents/Resources')) : process.platform === 'linux' ? ensure(join(stage, 'usr/share/lolping-lite')) : stage;
const bin = process.platform === 'darwin' ? ensure(join(stage, 'Contents/MacOS')) : process.platform === 'linux' ? ensure(join(stage, 'usr/bin')) : stage;
let commit = process.env.GITHUB_SHA;
if (!commit) { try { commit = run('git', ['rev-parse', 'HEAD'], { stdio: 'pipe' }).toString().trim(); } catch { commit = 'uncommitted-development-build'; } }
for (const dir of ['ui', 'assets', 'licenses']) cpSync(join(root, dir), join(data, dir), { recursive: true });
for (const name of ['LICENSE', 'THIRD_PARTY_NOTICES.md']) copyFileSync(join(root, name), join(data, name));
copyFileSync(join(root, 'resources/fonts/OFL.txt'), join(data, 'licenses/NotoSansCJK-OFL.txt'));
writeRustNotices(join(data, 'licenses'));
copyFileSync(join(root, 'resources/upstream.json'), join(data, 'upstream.json'));
write(join(data, 'build-info.json'), JSON.stringify({ product: 'LOLPing Lite', version, target, applicationId: 'io.github.ezrealbyte.lolpinglite', source: 'https://github.com/Ezreal-byte/lolping-lite', commit, renderer: 'native-rust', signing: process.platform === 'darwin' ? 'ad-hoc, not notarized' : 'unsigned' }, null, 2) + '\n');
for (const source of [exe, helper]) { const dest = join(bin, basename(source)); copyFileSync(source, dest); if (process.platform !== 'win32') chmodSync(dest, 0o755); }

if (process.platform === 'win32') {
  write(join(stage, 'app.ico'), windowsIcon(readFileSync(join(root, 'assets/plugin.png'))));
  await import('./bundle-windows.mjs').then(m => m.bundleWindows(stage));
}
if (process.platform === 'darwin') {
  const iconset = ensure(join(stageBase, 'app.iconset'));
  for (const size of [16, 32, 128, 256, 512]) for (const scale of [1, 2]) {
    run('sips', ['-z', String(size * scale), String(size * scale), join(root, 'assets/plugin.png'), '--out', join(iconset, `icon_${size}x${size}${scale === 2 ? '@2x' : ''}.png`)], { stdio: 'pipe' });
  }
  run('iconutil', ['-c', 'icns', iconset, '-o', join(data, 'app.icns')]);
  write(join(stage, 'Contents/Info.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>io.github.ezrealbyte.lolpinglite</string>
<key>CFBundleName</key><string>LOLPing Lite</string><key>CFBundleDisplayName</key><string>LOLPing Lite</string>
<key>CFBundleExecutable</key><string>lolping-lite</string><key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>${version}</string><key>CFBundleVersion</key><string>${version}</string>
<key>CFBundleIconFile</key><string>app.icns</string><key>LSMinimumSystemVersion</key><string>13.0</string>
<key>LSUIElement</key><true/><key>NSHighResolutionCapable</key><true/>
<key>NSHumanReadableCopyright</key><string>MIT; game artwork is separately licensed. See THIRD_PARTY_NOTICES.md.</string>
</dict></plist>\n`);
  run('codesign', ['--force', '--sign', '-', join(bin, 'hook-helper')]);
  run('codesign', ['--force', '--sign', '-', join(bin, 'lolping-lite')]);
  run('codesign', ['--force', '--sign', '-', stage]);
  run('codesign', ['--verify', '--deep', '--strict', stage]);
}
if (process.platform === 'linux') {
  const desktop = `[Desktop Entry]\nType=Application\nName=LOLPing Lite\nComment=Native desktop ping and emote wheel (X11)\nExec=lolping-lite\nIcon=lolping-lite\nTerminal=false\nCategories=Utility;\nStartupNotify=false\nX-AppImage-Version=${version}\n`;
  write(join(stage, 'usr/share/applications/lolping-lite.desktop'), desktop);
  write(join(stage, 'lolping-lite.desktop'), desktop);
  for (const path of ['lolping-lite.png', 'usr/share/icons/hicolor/128x128/apps/lolping-lite.png']) {
    ensure(dirname(join(stage, path))); copyFileSync(join(root, 'assets/plugin.png'), join(stage, path));
  }
  write(join(stage, 'AppRun'), `#!/bin/sh\nset -eu\nHERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\nexport LOLPING_ROOT="$HERE/usr/share/lolping-lite"\nexport LD_LIBRARY_PATH="$HERE/usr/lib\${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"\nexec "$HERE/usr/bin/lolping-lite" "$@"\n`);
  chmodSync(join(stage, 'AppRun'), 0o755);
}

function makeChecksums(base, destination) {
  const files = Object.fromEntries(filesBelow(base).filter(f => f.path !== destination).map(f => [f.key, hash(readFileSync(f.path))]));
  write(destination, JSON.stringify({ algorithm: 'sha256', files }, null, 2) + '\n');
}
// Contents are hashed after signing on macOS. This file is outside the .app to
// keep the code signature valid; package verification hashes the signed bundle.
const checksumsPath = process.platform === 'darwin' ? join(stageBase, 'checksums.json') : join(data, 'checksums.json');
makeChecksums(stage, checksumsPath);
const staged = filesBelow(stage);
console.log(`Staged ${target}: ${(staged.reduce((n, f) => n + f.size, 0) / 1048576).toFixed(2)} MiB`);
const pack = buildExtraEmotes();
if (process.argv.includes('--stage-only')) process.exit(0);
const artifacts = [];
function record(path, details = {}) { const bytes = readFileSync(path); artifacts.push({ file: basename(path), size: bytes.length, sha256: hash(bytes), ...details }); }

if (process.platform === 'win32') {
  const zipFiles = Object.fromEntries(staged.map(f => [f.key, readFileSync(f.path)]));
  const zip = join(dist, `${stem}.zip`);
  write(zip, zipSync(zipFiles, { level: 9, mtime: new Date(2020, 0, 1, 0, 0, 0) }));
  record(zip, { kind: 'portable', unsigned: true });
  const uninstall = join(stageBase, '..', 'uninstall-files.nsh');
  const dirs = new Set();
  for (const f of staged) { let dir = dirname(f.key); while (dir !== '.') { dirs.add(dir); dir = dirname(dir); } }
  const quote = s => s.replaceAll('$', '$$').replaceAll('"', '$\\"').replaceAll('/', '\\');
  write(uninstall, staged.map(f => `Delete "$INSTDIR\\${quote(f.key)}"`).concat([...dirs].sort((a, b) => b.length - a.length).map(dir => `RMDir "$INSTDIR\\${quote(dir)}"`)).join('\n') + '\n');
  const installer = join(dist, `${stem}-Setup.exe`);
  run(await tool('nsis'), ['/V2', '/INPUTCHARSET', 'UTF8', `/DVERSION=${version}`, `/DSTAGE=${stage}`, `/DOUTPUT=${installer}`, `/DICON=${join(stage, 'app.ico')}`, `/DLICENSE_FILE=${join(root, 'LICENSE')}`, `/DUNINSTALL_FILES=${uninstall}`, `/DINSTALLED_KIB=${Math.ceil(staged.reduce((n, f) => n + f.size, 0) / 1024)}`, join(root, 'packaging/windows.nsi')]);
  record(installer, { kind: 'installer', unsigned: true });
} else if (process.platform === 'darwin') {
  const dmgRoot = ensure(join(stageBase, 'dmg'));
  cpSync(stage, join(dmgRoot, appName), { recursive: true });
  copyFileSync(checksumsPath, join(dmgRoot, 'checksums.json'));
  symlinkSync('/Applications', join(dmgRoot, 'Applications'));
  const dmg = join(dist, `${stem}.dmg`);
  await diskImage(['create', '-volname', 'LOLPing Lite', '-srcfolder', dmgRoot, '-ov', '-format', 'UDZO', '-imagekey', 'zlib-level=9', dmg]);
  await diskImage(['verify', dmg]);
  record(dmg, { kind: 'disk-image', adHocSigned: true, notarized: false });
} else {
  // Debian relies on system packages; build it before bundling shared libraries
  // into the portable AppImage. Both layouts use the same native executables.
  const debRoot = ensure(join(stageBase, 'deb'));
  cpSync(join(stage, 'usr'), join(debRoot, 'usr'), { recursive: true });
  makeChecksums(debRoot, join(debRoot, 'usr/share/lolping-lite/checksums.json'));
  write(join(debRoot, 'DEBIAN/control'), `Package: lolping-lite\nVersion: ${version}\nSection: utils\nPriority: optional\nArchitecture: amd64\nMaintainer: Ezreal-byte <noreply@github.com>\nHomepage: https://github.com/Ezreal-byte/lolping-lite\nDepends: libc6 (>= 2.35), libgcc-s1, libstdc++6, libgtk-3-0, libasound2, libx11-6, libxi6, libxtst6, libxkbcommon-x11-0, libayatana-appindicator3-1, libgl1, libegl1\nDescription: Native desktop ping and emote wheel\n Rust-native settings, transparent desktop effects, and local emotes.\n Requires an X11 desktop with a compositor. Wayland gestures are unsupported.\n`);
  const deb = join(dist, `${stem}.deb`);
  run('dpkg-deb', ['--root-owner-group', '--build', debRoot, deb]);
  record(deb, { kind: 'debian', minimum: 'Ubuntu 22.04 / glibc 2.35; X11 + compositor' });
  await import('./bundle-linux.mjs').then(m => m.bundleLinux(stage));
  makeChecksums(stage, checksumsPath);
  const image = join(dist, `${stem}.AppImage`);
  const appimagetool = await tool('appimagetool'), runtime = await tool('appimage-runtime');
  run(appimagetool, ['--appimage-extract-and-run', '--runtime-file', runtime, stage, image], { env: { ...process.env, ARCH: 'x86_64', SOURCE_DATE_EPOCH: '1577836800' } });
  chmodSync(image, 0o755);
  record(image, { kind: 'appimage', minimum: 'glibc 2.35; X11 + compositor and system GL drivers' });
}

record(pack, { kind: 'optional-emotes', emotes: 35 });
const metadata = { product: 'LOLPing Lite', version, target, stagedBytes: filesBelow(stage).reduce((n, f) => n + f.size, 0), artifacts };
write(join(dist, `${target}.artifact.json`), JSON.stringify(metadata, null, 2) + '\n');
write(join(dist, `SHA256SUMS-${target}.txt`), artifacts.map(a => `${a.sha256}  ${a.file}`).join('\n') + '\n');
console.log(JSON.stringify(metadata, null, 2));
