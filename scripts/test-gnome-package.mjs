import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, rmdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { extensionUuid, isExtensionScript, packExtension, readExtension, validateExtension, verifyExtensionArchive } from './gnome-extension.mjs';

const fixture = () => ({
  'extension.js': Buffer.from('export default class LOLPingExtension {}'),
  'metadata.json': Buffer.from(JSON.stringify({ uuid: extensionUuid, name: 'LOLPing Lite', version: 1, 'shell-version': ['46'] })),
  'protocol.js': Buffer.from('export const VERSION = 1;'),
});

test('installable extension ZIP keeps files at its root and is deterministic', () => {
  const files = fixture(), first = packExtension(files), second = packExtension(files);
  assert.deepEqual(first, second);
  const actual = verifyExtensionArchive(first);
  for (const [path, bytes] of Object.entries(files)) assert.deepEqual(Buffer.from(actual[path]), bytes);
});

test('extension payload rejects unrelated scripts, missing files and unvalidated Shell versions', () => {
  assert.throws(() => validateExtension({ ...fixture(), 'runtime.js': Buffer.from('unexpected') }), /Unexpected or missing/);
  const missing = fixture(); delete missing['protocol.js'];
  assert.throws(() => validateExtension(missing), /Unexpected or missing/);
  const wrongShell = fixture();
  wrongShell['metadata.json'] = Buffer.from(JSON.stringify({ uuid: extensionUuid, name: 'LOLPing Lite', version: 1, 'shell-version': ['42', '46'] }));
  assert.throws(() => validateExtension(wrongShell), /Only validated/);
  const wrongUuid = fixture();
  wrongUuid['metadata.json'] = Buffer.from(JSON.stringify({ uuid: 'other@example.com', name: 'LOLPing Lite', version: 1, 'shell-version': ['46'] }));
  assert.throws(() => validateExtension(wrongUuid), /UUID/);
});

test('native package allows only the two compositor scripts at the exact extension path', () => {
  const prefix = 'usr/share/lolping-lite/';
  for (const name of ['extension.js', 'protocol.js']) assert.ok(isExtensionScript(`${prefix}gnome-extension/${extensionUuid}/${name}`, prefix));
  for (const path of [
    `${prefix}gnome-extension/${extensionUuid}/extra.js`,
    `${prefix}gnome-extension/${extensionUuid}/nested/extension.js`,
    `${prefix}gnome-extension/other@example.com/extension.js`,
    `${prefix}gnome-extension/${extensionUuid}/../extension.js`,
    `Contents/Resources/gnome-extension/${extensionUuid}/extension.js`,
    `${prefix}ui/extension.js`,
  ]) assert.equal(isExtensionScript(path, prefix), false, path);
});

test('extension source reader refuses directories and extraneous data', () => {
  const directory = mkdtempSync(join(tmpdir(), 'lolping-extension-package-'));
  try {
    for (const [path, bytes] of Object.entries(fixture())) writeFileSync(join(directory, path), bytes);
    assert.equal(readExtension(directory)['protocol.js'].length, fixture()['protocol.js'].length);
    mkdirSync(join(directory, 'nested'));
    assert.throws(() => readExtension(directory), /Unexpected/);
    rmdirSync(join(directory, 'nested'));
    rmSync(join(directory, 'extension.js'));
    mkdirSync(join(directory, 'extension.js'));
    assert.throws(() => readExtension(directory), /regular file/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
