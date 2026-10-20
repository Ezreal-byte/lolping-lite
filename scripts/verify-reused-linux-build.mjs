// Manual GNOME-only runs may reuse a verified binary only when every changed
// file is test infrastructure or validation documentation, never product code.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const repository = process.env.GITHUB_REPOSITORY;
const runId = process.env.LOLPING_ARTIFACT_RUN_ID;
assert.match(repository ?? '', /^[\w.-]+\/[\w.-]+$/);
assert.match(runId ?? '', /^\d+$/);
const command = (program, args) => execFileSync(program, args, { encoding: 'utf8' }).trim();
const api = path => JSON.parse(command('gh', ['api', `repos/${repository}/${path}`]));
const run = api(`actions/runs/${runId}`);
assert.equal(run.repository.full_name, repository);
assert.match(run.head_sha, /^[a-f0-9]{40}$/);
const jobs = api(`actions/runs/${runId}/jobs?per_page=100&filter=all`).jobs;
assert.ok(jobs.some(job => job.name === 'linux-x64' && job.conclusion === 'success'), 'Source Linux build must have passed');
const artifacts = resolve(process.argv[2]), root = resolve(process.argv[3]);
const info = JSON.parse(readFileSync(join(root, 'build-info.json'), 'utf8'));
assert.equal(info.target, 'linux-x64');
assert.equal(info.commit, run.head_sha, 'Extracted application must match the specified workflow run');
const metadata = JSON.parse(readFileSync(join(artifacts, 'dist/linux-x64.artifact.json'), 'utf8'));
assert.equal(metadata.target, 'linux-x64');
for (const artifact of metadata.artifacts) {
  assert.equal(artifact.file.includes('/'), false);
  assert.equal(artifact.file.includes('\\'), false);
  const bytes = readFileSync(join(artifacts, 'dist', artifact.file));
  assert.equal(bytes.length, artifact.size, artifact.file);
  assert.equal(createHash('sha256').update(bytes).digest('hex'), artifact.sha256, artifact.file);
}
const testCommit = command('git', ['rev-parse', 'HEAD']);
if (run.head_sha !== testCommit) command('git', ['fetch', '--no-tags', '--depth=1', 'origin', run.head_sha]);
const changedFiles = command('git', ['diff', '--name-only', run.head_sha, testCommit]).split('\n').filter(Boolean);
const testOnly = /^(\.github\/workflows\/build\.yml|scripts\/(test-gnome-(input-client\.py|wayland\.(mjs|py))|gnome-private-seat\.py|capture-private-gnome\.py|diagnose-linux-egl\.py|verify-reused-linux-build\.mjs)|docs\/resize-wayland-validation\.md)$/;
assert.deepEqual(changedFiles.filter(path => !testOnly.test(path)), [], 'Reused artifacts cannot validate changed product source');
const report = { artifactRunId: runId, sourceCommit: run.head_sha, testCommit,
  reused: run.head_sha !== testCommit, changedFiles, target: info.target, hashesVerified: true };
mkdirSync('dist', { recursive: true });
writeFileSync('dist/gnome-source-verification.json', JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify(report, null, 2));
