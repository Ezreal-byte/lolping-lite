import { copyFileSync, chmodSync, realpathSync, existsSync, statSync, readdirSync } from 'node:fs';
import { basename, join } from 'node:path';
import { ensure, run, write } from './common.mjs';

export function bundleLinux(appdir) {
  const libdir = ensure(join(appdir, 'usr/lib'));
  const bundled = new Map();
  let bundledLibraryBytes = 0;
  // glibc and GL/Vulkan drivers must match the host. Ship application libraries,
  // never the host loader, graphics driver, or glibc NSS modules.
  const hostLibrary = /^(ld-linux|lib(c|m|pthread|dl|rt|resolv|util|nss_[^.]+|anl)\.so|lib(GL|GLX|GLdispatch|EGL|GLES|OpenGL|glapi|vulkan|drm|gbm|nvidia|gallium|LLVM))/;
  const queue = [join(appdir, 'usr/bin/lolping-lite'), join(appdir, 'usr/bin/hook-helper')];
  const ldconfig = run('ldconfig', ['-p'], { stdio: 'pipe' }).toString();
  const indicator = ldconfig.split('\n').find(line => line.includes('libayatana-appindicator3.so.1 ') && line.includes('x86-64'))?.trim().split(' => ')[1];
  if (!indicator) throw Error('Install libayatana-appindicator3-1 before AppImage packaging');
  const xkb = ldconfig.split('\n').find(line => line.includes('libxkbcommon-x11.so.0 ') && line.includes('x86-64'))?.trim().split(' => ')[1];
  if (!xkb) throw Error('Install libxkbcommon-x11-0 before AppImage packaging');
  const dynamicRoots = new Set([indicator, xkb]);
  queue.push(...dynamicRoots);
  const scan = new Set();
  while (queue.length) {
    const binary = queue.shift();
    if (scan.has(binary)) continue;
    scan.add(binary);
    const output = run('ldd', [binary], { stdio: 'pipe' }).toString();
    if (output.includes('not found')) throw Error(`Unresolved dependency for ${binary}:\n${output}`);
    const dependencies = output.split('\n').flatMap(line => { const match = line.match(/^\s*(\S+)\s+=>\s+(\/\S+)/); return match ? [{ soname: match[1], path: match[2] }] : []; });
    if (dynamicRoots.has(binary)) dependencies.push({ soname: basename(binary), path: binary });
    for (const { soname, path } of dependencies) {
      if (hostLibrary.test(soname) || bundled.has(soname)) continue;
      // These are regular ELF files reached through SONAME symlinks. Resolve
      // them explicitly and use file-copy semantics; cpSync's recursive path
      // handling misclassifies library symlinks on the pinned Node 22 runner:
      // node v22.22.0 src/node_file.cc CpSyncCheckPaths treats every dereferenced
      // symlink as src_is_dir. Its trailing '/' is only error-message formatting.
      const source = realpathSync(path), info = statSync(source);
      if (!info.isFile()) throw Error(`ELF dependency is not a regular file: ${path} -> ${source}`);
      const destination = join(libdir, soname);
      copyFileSync(source, destination);
      chmodSync(destination, info.mode & 0o777);
      bundledLibraryBytes += info.size;
      bundled.set(soname, path); queue.push(path);
    }
  }
  // Retain system package copyright notices for each bundled dependency.
  const noticeDir = ensure(join(appdir, 'usr/share/lolping-lite/licenses/linux-bundled'));
  const commonLicenses = ensure(join(noticeDir, 'common-licenses'));
  // Node 22's recursive cp also preserves absolute symlinks despite dereference.
  // Materialize aliases such as GPL -> GPL-3 so notices stay self-contained.
  for (const name of readdirSync('/usr/share/common-licenses')) {
    const source = realpathSync(join('/usr/share/common-licenses', name));
    if (!statSync(source).isFile()) throw Error(`Common license is not a regular file: ${source}`);
    copyFileSync(source, join(commonLicenses, name));
  }
  const packages = new Set();
  for (const source of bundled.values()) {
    let result;
    for (const candidate of [source, realpathSync(source), `*/${basename(realpathSync(source))}`]) {
      try { result = run('dpkg-query', ['-S', candidate], { stdio: 'pipe' }).toString().trim().split('\n')[0]; break; } catch { /* /lib and /usr/lib aliases differ on usr-merged systems. */ }
    }
    if (!result) throw Error(`Cannot identify package license for ${source}`);
    const pkg = result.slice(0, result.indexOf(': '));
    if (!pkg || packages.has(pkg)) continue;
    packages.add(pkg);
    const copyright = `/usr/share/doc/${pkg.replace(/:amd64$/, '')}/copyright`;
    if (!existsSync(copyright)) throw Error(`Missing dependency license: ${pkg}`);
    copyFileSync(realpathSync(copyright), join(noticeDir, `${pkg.replaceAll(':', '-')}.txt`));
  }
  write(join(noticeDir, 'manifest.json'), JSON.stringify({ libraries: Object.fromEntries(bundled), bundledLibraryBytes, packages: [...packages], hostLibraries: ['glibc', 'OpenGL/EGL/Vulkan drivers'] }, null, 2) + '\n');
  console.log(`AppImage: bundled ${bundled.size} native libraries (${bundledLibraryBytes} bytes); glibc and GPU drivers remain system dependencies.`);
}
