import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { root, version } from './common.mjs';
import { packageVersions } from './package-version.mjs';

test('preview release keeps its public name while OS metadata uses valid versions', () => {
  assert.deepEqual(packageVersions('0.3.0-Preview'), {
    display: '0.3.0-Preview', core: '0.3.0', prerelease: 'Preview',
    windows: '0.3.0.0', macos: '0.3.0', debian: '0.3.0~preview',
  });
});

test('stable and numbered preview versions map without losing the release label', () => {
  assert.deepEqual(packageVersions('0.3.0'), {
    display: '0.3.0', core: '0.3.0', prerelease: '',
    windows: '0.3.0.0', macos: '0.3.0', debian: '0.3.0',
  });
  const preview = packageVersions('0.3.0-Preview.2+build-10');
  assert.equal(preview.display, '0.3.0-Preview.2+build-10');
  assert.equal(preview.windows, '0.3.0.0');
  assert.equal(preview.debian, '0.3.0~preview.2+build.10');
});

test('invalid versions cannot be interpolated into installer or plist metadata', () => {
  for (const value of ['', '0.3', 'v0.3.0', '0.03.0', '0.3.0-', '0.3.0-01', '65536.0.0',
    '0.3.0-preview\n', '0.3.0-<Preview>', '0.3.0-"Preview"', '0.3.0-Preview/other']) {
    assert.throws(() => packageVersions(value), /Invalid or unsupported package version/);
  }
});

test('Cargo, npm and lockfiles agree on the exact release version', () => {
  const text = file => readFileSync(join(root, file), 'utf8');
  assert.equal(JSON.parse(text('package-lock.json')).version, version);
  assert.equal(JSON.parse(text('package-lock.json')).packages[''].version, version);
  assert.equal(text('backend/Cargo.toml').match(/^version = "([^"]+)"$/m)?.[1], version);
  assert.equal(text('Cargo.lock').match(/\[\[package\]\]\r?\nname = "lolping-lite"\r?\nversion = "([^"]+)"/)?.[1], version);
});

test('Debian previews sort before stable and after the previous stable version', { skip: process.platform !== 'linux' }, () => {
  for (const candidate of ['0.3.0-Preview', '0.3.0-Preview.2']) {
    const versions = packageVersions(candidate);
    execFileSync('dpkg', ['--validate-version', versions.debian]);
    execFileSync('dpkg', ['--compare-versions', versions.debian, 'lt', versions.core]);
    execFileSync('dpkg', ['--compare-versions', versions.debian, 'gt', '0.2.0']);
  }
});
