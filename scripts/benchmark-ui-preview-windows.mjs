// Explicit opt-in real-desktop A/B test. Compares the immutable native 0.1.0
// release with a local UI preview, using the existing PID-reuse-safe sampler.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { unzipSync } from 'fflate';
import { hash, root as repository } from './common.mjs';
import { Rpc, processAlive } from './standalone-smoke.mjs';

const execute = promisify(execFile);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const median = values => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const publishedZipSha256 = '04c57dc1e4afa57ca7ed808c6e62917fbf7d713eb248cf9746d559d737c7f555';

export function summarize(runs) {
  const output = {};
  for (const kind of ['release', 'preview']) {
    output[kind] = {};
    for (const scene of ['idle', 'settingsOpen', 'settingsClosed']) {
      const rows = runs.filter(run => run.kind === kind).map(run => run.samples[scene]);
      assert.ok(rows.length >= 3 && rows.every(Boolean), 'Every product/scenario requires three complete samples');
      output[kind][scene] = { samples: rows.length,
        workingSetBytes: median(rows.map(row => row.workingSetBytes)),
        privateCommitBytes: median(rows.map(row => row.privateCommitBytes)),
        cpuOneCorePercent: median(rows.map(row => row.cpuOneCorePercent)),
        processCounts: rows.map(row => row.processCount) };
    }
  }
  return output;
}

function extractPublishedArchive(path) {
  const bytes = readFileSync(path);
  assert.equal(hash(bytes), publishedZipSha256, 'Baseline must be the verified public v0.1.0 Windows ZIP, not a rebuilt copy');
  const entries = unzipSync(bytes);
  const destination = join(repository, '.cache', `ui-preview-baseline-${publishedZipSha256.slice(0, 16)}`);
  for (const [name, contents] of Object.entries(entries)) {
    assert.ok(!name.includes('\\') && !name.includes(':') && !name.startsWith('/') && !name.split('/').includes('..'), 'Unsafe baseline ZIP path');
    const output = resolve(destination, name), rel = relative(destination, output);
    assert.ok(!isAbsolute(rel) && !rel.startsWith('..'));
    mkdirSync(dirname(output), { recursive: true });
    writeFileSync(output, contents);
  }
  const manifest = JSON.parse(Buffer.from(entries['checksums.json']).toString('utf8'));
  for (const [name, digest] of Object.entries(manifest.files)) assert.equal(hash(readFileSync(join(destination, name))), digest, name);
  const build = JSON.parse(Buffer.from(entries['build-info.json']).toString('utf8'));
  assert.equal(build.version, '0.1.0');
  assert.equal(build.renderer, 'native-rust');
  return { root: destination, executable: join(destination, 'lolping-lite.exe'), archiveSha256: publishedZipSha256, build };
}

async function main() {
  const args = process.argv.slice(2);
  const option = (name, fallback) => {
    const at = args.indexOf(name);
    if (at < 0) return fallback;
    assert.ok(args[at + 1] && !args[at + 1].startsWith('--'), `${name} needs a value`);
    return args[at + 1];
  };
  const baselinePath = resolve(option('--baseline', join(repository, '.cache/release/39015448891/LOLPing-Lite-0.1.0-windows-x64.zip')));
  if (args.includes('--prepare-baseline')) {
    const prepared = extractPublishedArchive(baselinePath);
    console.log(JSON.stringify({ baselineRoot: prepared.root, archiveSha256: prepared.archiveSha256, executableSha256: hash(readFileSync(prepared.executable)), commit: prepared.build.commit }, null, 2));
    return;
  }
  if (!args.includes('--run-desktop')) {
    console.log('Opt-in desktop test; no app started. Usage: node scripts/benchmark-ui-preview-windows.mjs --run-desktop [--baseline published-v0.1.0.zip] [--root .stage/windows-x64-ui-preview] [--repetitions 3] [--sample-ms 6000] [--report dist/ui-preview-performance-windows.json]. Use --prepare-baseline to verify/extract the published baseline without launching any app.');
    return;
  }
  assert.equal(process.platform, 'win32');
  const repetitions = Number(option('--repetitions', '3')), sampleMs = Number(option('--sample-ms', '6000'));
  assert.ok(Number.isInteger(repetitions) && repetitions >= 3 && repetitions <= 10);
  assert.ok(Number.isInteger(sampleMs) && sampleMs >= 5000 && sampleMs <= 30000);
  const settleMs = 22000;
  const baseline = extractPublishedArchive(baselinePath);
  const previewRoot = resolve(option('--root', join(repository, '.stage/windows-x64-ui-preview')));
  const preview = { root: previewRoot, executable: join(previewRoot, 'lolping-lite.exe'), build: JSON.parse(readFileSync(join(previewRoot, 'build-info.json'), 'utf8')) };
  assert.equal(preview.build.channel, 'ui-preview');
  assert.match(preview.build.version, /^\d+\.\d+\.\d+(?:[-+].+)?$/, 'Candidate build must declare a version');
  assert.equal(preview.build.version, JSON.parse(readFileSync(join(repository, 'package.json'), 'utf8')).version, 'Rebuild and repackage the current version before measuring');
  assert.equal(hash(readFileSync(preview.executable)), preview.build.executableSha256);
  const reportPath = resolve(option('--report', join(repository, 'dist/ui-preview-performance-windows.json')));
  const markdownPath = reportPath.replace(/\.json$/i, '') + '.md';
  const dataRoot = join(repository, '.test-data', `ui-preview-performance-${Date.now()}`);
  const powershell = process.env.CODEX_PWSH_PATH || 'pwsh';
  const probe = async (pid, extra = []) => {
    const { stdout } = await execute(powershell, ['-NoProfile', '-File', join(repository, 'scripts/benchmark/measure-windows.ps1'), '-RootProcess', String(pid), '-DurationMs', String(sampleMs), ...extra], { windowsHide: true, encoding: 'utf8', maxBuffer: 1024 * 1024 });
    return JSON.parse(stdout.replace(/^\uFEFF/, ''));
  };
  const report = { product: 'LOLPing Lite', version: preview.build.version, channel: 'ui-preview', complete: false,
    date: new Date().toISOString(), hardware: await probe(0, ['-Hardware']),
    baseline: { archiveSha256: baseline.archiveSha256, executableSha256: hash(readFileSync(baseline.executable)), build: baseline.build },
    candidate: { executableSha256: hash(readFileSync(preview.executable)), build: preview.build },
    method: { repetitions, sampleWindowMs: sampleMs, idleSettleMs: settleMs, settingsSettleMs: 2000,
      theme: 'dark', language: 'zh-CN', productionTrayEnabled: true, muted: true,
      scope: 'Complete application descendant tree, including helper and settings while open; GPU memory excluded. Working-set sums may double-count shared resident pages.',
      processIdentity: 'Existing process-tree.ps1 birth-time validation rejects reused parent/child PIDs; sampler errors never become zero measurements.',
      sequence: 'Alternating release/preview order each repetition. Identical 12-signal renderer warm-up, 22 seconds idle; open default settings, settle 2 seconds; close settings, settle 22 seconds.',
      limitation: 'One Windows machine, end-of-window memory rather than peak. CPU is one-core equivalent. Settings layouts and default window dimensions intentionally differ. No physical gesture/audio latency measurement.' },
    runs: [], summaries: {}, comparisons: {} };
  const save = () => { mkdirSync(dirname(reportPath), { recursive: true }); writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n'); };
  async function runOne(kind, repetition) {
    const build = kind === 'release' ? baseline : preview;
    const data = join(dataRoot, `${kind}-${repetition}`);
    mkdirSync(data, { recursive: true });
    writeFileSync(join(data, 'app.json'), JSON.stringify({ theme: 'dark', autostart: false, onboarded: true }));
    const rpc = new Rpc(build.root, data, build.executable, { production: true });
    const row = { kind, repetition, samples: {} };
    report.runs.push(row);
    const sample = async scene => {
      console.log(`Native A/B ${kind} ${repetition}/${repetitions}: ${scene}`);
      const result = await probe(rpc.child.pid);
      assert.ok(result.processCount >= 2);
      assert.equal(result.exitedDuringWindow, 0, 'A child exited during measurement; retry instead of accepting incomplete CPU totals');
      assert.ok(!result.processes.some(process => /webview|webkit|chrome|msedge/i.test(process.name)));
      row.samples[scene] = result;
      save();
    };
    try {
      await rpc.poll('state', state => state.status.state === 'running');
      await rpc.request('settings', { muted: true, volume: 0, language: 'zh-CN', keepLastPing: false,
        trigger: { vk: 117 }, emoteTrigger: 'off', clickPing: false, emoteClick: false,
        pingDurationS: 3.2, pingSizePx: 110, emoteSizePx: 150, wheelSizePx: 600 });
      const helper = (await rpc.poll('diagnostics', state => state.helperPid > 0)).helperPid;
      for (let i = 0; i < 12; i++) { await rpc.request('preview', { id: i % 2 ? 'danger' : 'generic', wheel: 'ping' }); await delay(100); }
      await delay(settleMs);
      await sample('idle');
      const mark = rpc.sequence;
      await rpc.request('ui/open');
      await rpc.waitFor(event => event.event === 'ui/ready', 30000, mark);
      const settings = (await rpc.poll('diagnostics', state => state.settingsPid > 0)).settingsPid;
      assert.equal((await rpc.request('diagnostics')).helperPid, helper);
      await delay(2000);
      await sample('settingsOpen');
      await rpc.request('ui/close');
      await rpc.poll('diagnostics', state => state.settingsPid === null);
      for (let attempt = 0; attempt < 60 && processAlive(settings); attempt++) await delay(100);
      assert.equal(processAlive(settings), false, 'Closed settings must exit before the memory sample');
      assert.equal((await rpc.request('diagnostics')).helperPid, helper);
      await delay(settleMs);
      await sample('settingsClosed');
    } finally { await rpc.close(); }
  }
  for (let repetition = 1; repetition <= repetitions; repetition++) {
    for (const kind of repetition % 2 ? ['release', 'preview'] : ['preview', 'release']) {
      await runOne(kind, repetition);
      await delay(1500);
    }
  }
  report.summaries = summarize(report.runs);
  for (const scene of ['idle', 'settingsOpen', 'settingsClosed']) {
    const old = report.summaries.release[scene], current = report.summaries.preview[scene];
    report.comparisons[scene] = Object.fromEntries(['workingSetBytes', 'privateCommitBytes'].map(key => [key, { changeBytes: current[key] - old[key], changePercent: (current[key] / old[key] - 1) * 100 }]));
  }
  report.complete = true;
  save();
  const labels = { idle: '后台空闲', settingsOpen: '设置打开、无交互', settingsClosed: '关闭设置后' };
  const rows = Object.entries(labels).flatMap(([scene, label]) => ['release', 'preview'].map(kind => {
    const sample = report.summaries[kind][scene];
    return `| ${label} | ${kind === 'release' ? '正式版 0.1.0' : `UI 预览版 ${preview.build.version}`} | ${(sample.workingSetBytes / 1048576).toFixed(1)} | ${(sample.privateCommitBytes / 1048576).toFixed(1)} | ${sample.cpuOneCorePercent.toFixed(2)}% |`;
  }));
  writeFileSync(markdownPath, `# UI 预览版 ${preview.build.version} Windows 同机内存复测\n\n每个版本 ${repetitions} 轮，交替顺序，表格为中位数。两个版本都使用深色中文界面、生产托盘和独立测试数据；设置打开时保持各自默认布局及窗口大小。\n\n| 场景 | 版本 | 工作集 MiB | 私有提交 MiB | CPU（单核等效） |\n|---|---|---:|---:|---:|\n${rows.join('\n')}\n\n包含完整应用进程树，GPU 内存不计入；工作集相加可能重复计算共享页。先等待 22 秒空闲回收，再采样 ${sampleMs / 1000} 秒；设置打开等待 2 秒。数值不是峰值或所有电脑的保证，短窗口 CPU 0.00% 不代表绝对不耗 CPU。每个场景均确认没有进程在采样期间退出。\n\n环境：${report.hardware.os} ${report.hardware.osVersion}；${report.hardware.cpu}；${report.hardware.logicalProcessors} 逻辑处理器；${report.hardware.memoryMiB} MiB 内存；${report.hardware.displays.map(display => `${display.width}×${display.height} / ${display.scalePercent}%`).join('，')}。\n\n被测候选二进制 SHA-256：\`${report.candidate.executableSha256}\`。源代码和二进制哈希、完整样本见同名 JSON。此报告不修改已发布的性能报告，不代表 macOS/Linux 验收，也不测量真实手势延迟。\n`);
  console.log(JSON.stringify({ summaries: report.summaries, comparisons: report.comparisons, report: reportPath }, null, 2));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
