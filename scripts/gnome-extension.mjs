// A GNOME Shell extension runs inside the compositor, never in a browser.
// Keep its allowed executable payload explicit instead of relaxing the native
// package's general prohibition on JavaScript/web runtimes.
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, lstatSync } from 'node:fs';
import { join } from 'node:path';
import { zipSync, unzipSync } from 'fflate';

export const extensionUuid = 'lolping-lite@ezreal-byte.github.io';
export const extensionFiles = ['extension.js', 'metadata.json', 'protocol.js'];
export const extensionShellVersions = ['46'];
export const extensionRelativeDirectory = `gnome-extension/${extensionUuid}`;

export function validateExtension(files) {
  assert.deepEqual(Object.keys(files).sort(), extensionFiles, 'Unexpected or missing GNOME extension file');
  const metadata = JSON.parse(Buffer.from(files['metadata.json']).toString('utf8'));
  assert.equal(metadata.uuid, extensionUuid, 'GNOME extension UUID');
  assert.deepEqual(metadata['shell-version'], extensionShellVersions, 'Only validated GNOME Shell versions may be advertised');
  assert.ok(Number.isSafeInteger(metadata.version) && metadata.version > 0, 'GNOME extension version must be a positive integer');
  assert.ok(typeof metadata.name === 'string' && metadata.name.includes('LOLPing'), 'GNOME extension name');
  for (const file of extensionFiles) assert.ok(files[file].length > 0, `Empty extension payload: ${file}`);
  return metadata;
}

export function readExtension(directory) {
  const files = {};
  for (const name of readdirSync(directory).sort()) {
    assert.ok(extensionFiles.includes(name), `Unexpected GNOME extension file: ${name}`);
    const path = join(directory, name);
    assert.ok(lstatSync(path).isFile(), `GNOME extension payload must be a regular file: ${name}`);
    files[name] = readFileSync(path);
  }
  validateExtension(files);
  return files;
}

export function packExtension(files) {
  validateExtension(files);
  return zipSync(files, { level: 9, mtime: new Date(2020, 0, 1, 0, 0, 0) });
}

export function verifyExtensionArchive(bytes) {
  const files = unzipSync(bytes);
  validateExtension(files);
  return files;
}

export function isExtensionScript(path, dataPrefix) {
  return ['extension.js', 'protocol.js'].some(name => path === `${dataPrefix}${extensionRelativeDirectory}/${name}`);
}
