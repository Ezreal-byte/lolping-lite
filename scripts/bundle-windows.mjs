import { copyFileSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { root, hash, executableArchitecture, run, write } from './common.mjs';
import { peImports, isVisualCppRuntime } from './pe-imports.mjs';

export function bundleWindows(stage) {
  const powershell = process.env.CODEX_PWSH_PATH || 'pwsh';
  const discovered = JSON.parse(run(powershell, ['-NoProfile', '-NonInteractive', '-File', join(root, 'scripts/find-windows-runtime.ps1')], { stdio: 'pipe' }).toString('utf8').replace(/^\uFEFF/, ''));
  const available = new Map(discovered.libraries.map(dll => [dll.name.toLowerCase(), dll]));
  const pending = ['lolping-lite.exe', 'hook-helper.exe'], seen = new Set(), dependencies = {}, bundled = [];
  while (pending.length) {
    const filename = pending.shift();
    if (seen.has(filename)) continue;
    seen.add(filename);
    const imports = peImports(readFileSync(join(stage, filename))).filter(isVisualCppRuntime);
    dependencies[filename] = imports;
    for (const name of imports) {
      if (bundled.some(dll => dll.name === name)) continue;
      const source = available.get(name);
      if (!source) throw Error(`Visual Studio's redistributable directory is missing ${name}, imported by ${filename}`);
      if (executableArchitecture(source.path) !== 'x64') throw Error(`Wrong runtime architecture: ${name}`);
      copyFileSync(source.path, join(stage, name));
      const bytes = readFileSync(join(stage, name));
      bundled.push({ name, fileVersion: source.fileVersion, architecture: 'x64', size: bytes.length, sha256: hash(bytes), source: source.source, signer: source.signer, signature: source.signature });
      pending.push(name);
    }
  }
  const license = `Microsoft-VC-Runtime-${discovered.visualStudioMajor === 18 ? 2026 : 2022}.txt`;
  const metadata = { provider: 'Microsoft Corporation', deployment: 'Unmodified application-local Visual C++ runtime; no global installer or registry changes', visualStudioMajor: discovered.visualStudioMajor, redistributableVersion: discovered.redistributableVersion, license: `licenses/${license}`, redistribution: `https://learn.microsoft.com/en-us/visualstudio/releases/${discovered.visualStudioMajor === 18 ? 2026 : 2022}/redistribution`, dependencies, libraries: bundled };
  write(join(stage, 'licenses/windows-runtime.json'), JSON.stringify(metadata, null, 2) + '\n');
  console.log(`Bundled application-local VC runtime: ${bundled.map(dll => `${dll.name} ${dll.fileVersion}`).join(', ') || 'not required'}`);
}
