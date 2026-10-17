// Production-path single-instance, --quit and settings-process memory regression.
// Uses fresh app data; never touches an installed user's settings or processes.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { Rpc, processAlive, stagedPaths } from './standalone-smoke.mjs';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const execute = promisify(execFile);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const median = values => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];

function memoryTrend(samples) {
  const y = samples.map(s => s.privateCommitBytes), n = y.length;
  const xMean = (n - 1) / 2, yMean = y.reduce((sum, value) => sum + value, 0) / n;
  let numerator = 0, denominator = 0;
  for (let i = 0; i < n; i++) { numerator += (i - xMean) * (y[i] - yMean); denominator += (i - xMean) ** 2; }
  const first = median(y.slice(0, 5)), last = median(y.slice(-5));
  return {
    privateCommitFirstFiveMedianBytes: first,
    privateCommitLastFiveMedianBytes: last,
    privateCommitMedianChangeBytes: last - first,
    privateCommitMedianChangePercent: (last - first) / first * 100,
    privateCommitSlopeBytesPerCycle: numerator / denominator,
    privateCommitMinimumBytes: Math.min(...y), privateCommitMaximumBytes: Math.max(...y),
    privateCommitIncreasingEveryCycle: y.every((v, i) => i === 0 || v > y[i - 1]),
    workingSetFirstFiveMedianBytes: median(samples.slice(0, 5).map(s => s.workingSetBytes)),
    workingSetLastFiveMedianBytes: median(samples.slice(-5).map(s => s.workingSetBytes)),
    interpretation: 'Descriptive measurements, not a pass/fail memory budget. Working set varies with Windows trimming and shared-page residency. Review private-commit medians and slope for sustained growth; process cleanup is asserted separately.',
  };
}

async function main() {
  assert.equal(process.platform, 'win32', 'Requires an interactive Windows desktop');
  const args = process.argv.slice(2);
  const option = (name, fallback) => {
    const index = args.indexOf(name);
    if (index < 0) return fallback;
    assert.ok(args[index + 1] && !args[index + 1].startsWith('--'), `${name} needs a value`);
    return resolve(args[index + 1]);
  };
  const paths = stagedPaths();
  const root = option('--root', paths.root), executable = option('--executable', paths.executable);
  const data = option('--data-dir', join(repository, '.test-data', `windows-lifecycle-${Date.now()}`));
  const output = option('--report', join(repository, 'dist', 'windows-lifecycle-report.json'));
  const report = {
    product: 'LOLPing Lite', version: JSON.parse(readFileSync(join(repository, 'package.json'), 'utf8')).version,
    target: 'windows-x64', complete: false, date: new Date().toISOString(),
    method: 'Normal tray, single-instance listener and startup behavior; the benchmark flag adds only an inherited control pipe. Fresh data directory, no synthetic keyboard or mouse input. Memory samples are the main process alone after settings exit.',
    passed: [], memorySamples: [],
  };
  const save = () => { mkdirSync(dirname(output), { recursive: true }); writeFileSync(output, JSON.stringify(report, null, 2) + '\n'); };
  const rpc = new Rpc(root, data, executable, { production: true });
  const secondary = async extra => execute(executable, [...extra, '--root', root, '--data-dir', data], { cwd: root, windowsHide: true, timeout: 15000, encoding: 'utf8', maxBuffer: 32768 });
  const gone = async pid => {
    for (let attempt = 0; attempt < 80 && processAlive(pid); attempt++) await delay(100);
    assert.equal(processAlive(pid), false, 'An application-owned process survived shutdown');
  };
  const sample = async cycle => {
    const { stdout } = await execute(process.env.CODEX_PWSH_PATH || 'pwsh', ['-NoProfile', '-File', join(repository, 'scripts/benchmark/measure-windows.ps1'), '-RootProcess', String(rpc.child.pid), '-ProcessOnly'], { windowsHide: true, encoding: 'utf8' });
    report.memorySamples.push({ cycle, ...JSON.parse(stdout.replace(/^\uFEFF/, '')) });
    save();
  };
  try {
    await rpc.poll('state', value => value.status.state === 'running');
    await rpc.request('settings', { muted: true, trigger: { vk: 117 }, emoteTrigger: 'off' });
    let diagnostics = await rpc.poll('diagnostics', value => value.helperPid > 0);
    const mainPid = rpc.child.pid, helperPid = diagnostics.helperPid;
    assert.equal(diagnostics.settingsPid, null, '--background must not open first-run settings');
    report.passed.push('production background launch automatically starts the input helper and tray without opening settings');

    const firstMark = rpc.sequence;
    await secondary(['--background']);
    await rpc.waitFor(event => event.event === 'ui/ready', 30000, firstMark);
    diagnostics = await rpc.poll('diagnostics', value => value.settingsPid > 0);
    const initialSettings = diagnostics.settingsPid;
    assert.equal(diagnostics.helperPid, helperPid);
    await Promise.all([secondary(['--background']), secondary(['--background']), secondary(['--background'])]);
    diagnostics = await rpc.request('diagnostics');
    assert.equal(rpc.child.pid, mainPid);
    assert.equal(diagnostics.helperPid, helperPid);
    assert.equal(diagnostics.settingsPid, initialSettings);
    report.passed.push('a second production launch opens existing settings; three concurrent launches reuse the same main, helper and settings processes');
    await rpc.request('ui/close');
    await rpc.poll('diagnostics', value => value.settingsPid === null);
    await gone(initialSettings);
    assert.equal((await rpc.request('diagnostics')).helperPid, helperPid);
    report.passed.push('closing settings keeps the original input helper alive');
    await sample(0);

    const settingsChildren = new Set([initialSettings]);
    for (let cycle = 1; cycle <= 20; cycle++) {
      const mark = rpc.sequence;
      await rpc.request('ui/open');
      await rpc.waitFor(event => event.event === 'ui/ready', 30000, mark);
      diagnostics = await rpc.poll('diagnostics', value => value.settingsPid > 0);
      settingsChildren.add(diagnostics.settingsPid);
      assert.equal(diagnostics.helperPid, helperPid);
      const child = diagnostics.settingsPid;
      await rpc.request('ui/close');
      await rpc.poll('diagnostics', value => value.settingsPid === null);
      await gone(child);
      assert.equal((await rpc.request('diagnostics')).helperPid, helperPid);
      await delay(250);
      await sample(cycle);
      console.log(`Production settings lifecycle ${cycle}/20 complete`);
    }
    report.passed.push('20 production settings open/close cycles preserve the helper and leave no settings descendants');
    report.memoryTrend = memoryTrend(report.memorySamples.filter(s => s.cycle > 0));
    for (const child of settingsChildren) await gone(child);

    const finalMark = rpc.sequence;
    await secondary(['--background']);
    await rpc.waitFor(event => event.event === 'ui/ready', 30000, finalMark);
    diagnostics = await rpc.poll('diagnostics', value => value.settingsPid > 0);
    const finalSettings = diagnostics.settingsPid;
    await secondary(['--quit']);
    const exit = await rpc.exited;
    assert.equal(exit.code, 0, 'Production --quit should terminate the original process cleanly');
    for (const child of [mainPid, helperPid, finalSettings]) await gone(child);
    report.passed.push('--quit delivered through the production single-instance channel terminates the main process, helper and open settings');
    report.complete = true;
    save();
    console.log(JSON.stringify({ passed: report.passed, memoryTrend: report.memoryTrend }, null, 2));
  } finally { await rpc.close(); }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
