import { cpSync, realpathSync, existsSync } from 'node:fs';
import { basename, join } from 'node:path';
import { ensure, run, write } from './common.mjs';

export function bundleLinux(appdir) {
  const libdir = ensure(join(appdir, 'usr/lib'));
  const bundled = new Map();
  // glibc and GL/Vulkan drivers must match the host. Ship application libraries,
  // never the host loader, graphics driver, or glibc NSS modules.
  const hostLibrary = /^(ld-linux|lib(c|m|pthread|dl|rt|resolv|util|nss_[^.]+|anl)\.so|lib(GL|GLX|GLdispatch|EGL|GLES|OpenGL|glapi|vulkan|drm|gbm|nvidia|gallium|LLVM))/;
  const queue = [join(appdir, 'usr/bin/lolping-lite'), join(appdir, 'usr/bin/hook-helper')];
  const ldconfig = run('ldconfig', ['-p'], { stdio: 'pipe' }).toString();
  const indicator = ldconfig.split('\n').find(line => line.includes('libayatana-appindicator3.so.1 ') && line.includes('x86-64'))?.trim().split(' => ')[1];
  if (!indicator) throw Error('Install libayatana-appindicator3-1 before AppImage packaging');
  queue.push(indicator);
  const scan = new Set();
  while (queue.length) {
    const binary = queue.shift();
    if (scan.has(binary)) continue;
    scan.add(binary);
    const output = run('ldd', [binary], { stdio: 'pipe' }).toString();
    if (output.includes('not found')) throw Error(`Unresolved dependency for ${binary}:\n${output}`);
    const dependencies = output.split('\n').flatMap(line => { const match = line.match(/^\s*(\S+)\s+=>\s+(\/\S+)/); return match ? [{ soname: match[1], path: match[2] }] : []; });
    if (binary === indicator) dependencies.push({ soname: basename(binary), path: binary });
    for (const { soname, path } of dependencies) {
      if (hostLibrary.test(soname) || bundled.has(soname)) continue;
      cpSync(path, join(libdir, soname), { dereference: true });
      bundled.set(soname, path); queue.push(path);
    }
  }
  // Retain system package copyright notices for each bundled dependency.
  const noticeDir = ensure(join(appdir, 'usr/share/lolping-lite/licenses/linux-bundled'));
  cpSync('/usr/share/common-licenses', join(noticeDir, 'common-licenses'), { recursive: true, dereference: true });
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
    cpSync(copyright, join(noticeDir, `${pkg.replaceAll(':', '-')}.txt`), { dereference: true });
  }
  write(join(noticeDir, 'manifest.json'), JSON.stringify({ libraries: Object.fromEntries(bundled), packages: [...packages], hostLibraries: ['glibc', 'OpenGL/EGL/Vulkan drivers'] }, null, 2) + '\n');
  console.log(`AppImage: bundled ${bundled.size} native libraries; glibc and GPU drivers remain system dependencies.`);
}
