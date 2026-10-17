// Real desktop measurements against an isolated, immutable 0.1.1 WebView build.
// Run only after smoke tests; this script opens native settings and overlays.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync, statSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { unzipSync } from 'fflate';
import { Rpc, stagedPaths } from './standalone-smoke.mjs';
import { Rpc as LegacyRpc } from './benchmark/legacy-rpc.mjs';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const execute = promisify(execFile);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const median = values => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const mib = value => (value / 1048576).toFixed(1);

function baselineRoot(input) {
  if (statSync(input).isDirectory()) return { root: input, archiveSha256: null };
  const bytes = readFileSync(input);
  const digest = sha256(bytes);
  const destination = join(repository, '.cache', `benchmark-baseline-${digest.slice(0, 16)}`);
  mkdirSync(destination, { recursive: true });
  const archive = unzipSync(bytes);
  for (const [name, contents] of Object.entries(archive)) {
    if (name.includes('\\') || name.includes(':') || name.startsWith('/') || name.split('/').includes('..')) throw Error('Unsafe baseline archive path');
    const output = resolve(destination, name);
    const rel = relative(destination, output);
    if (isAbsolute(rel) || rel.startsWith('..')) throw Error('Baseline archive escapes its destination');
    if (name.endsWith('/')) mkdirSync(output, { recursive: true });
    else { mkdirSync(dirname(output), { recursive: true }); writeFileSync(output, contents); }
  }
  return { root: destination, archiveSha256: digest };
}

async function poll(client, method, condition, timeout = 30000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const state = await client.request(method);
    if (condition(state)) return state;
    await delay(100);
  }
  throw Error(`Timed out waiting for ${method}`);
}

async function main() {
  assert.equal(process.platform, 'win32', 'This benchmark requires an interactive Windows desktop');
  const args = process.argv.slice(2);
  const option = (name, fallback) => {
    const at = args.indexOf(name);
    if (at < 0) return fallback;
    assert.ok(args[at + 1] && !args[at + 1].startsWith('--'), `${name} needs a value`);
    return args[at + 1];
  };
  const repetitions = Number(option('--repetitions', '3'));
  assert.ok(Number.isInteger(repetitions) && repetitions >= 3 && repetitions <= 10, 'Use at least three repetitions');
  const sampleMs = Number(option('--sample-ms', '6000'));
  assert.ok(sampleMs >= 5000 && sampleMs <= 30000, 'Sampling windows must last 5–30 seconds');
  const settleMs = 22000;
  const defaults = stagedPaths();
  const root = resolve(option('--root', defaults.root));
  const executable = resolve(option('--executable', defaults.executable));
  const baseline = baselineRoot(resolve(option('--baseline', join(repository, '..', 'dbx-lolping-plugin', 'dist', 'io.dbx.lolping.lite-0.1.1-windows-x64.dbxp'))));
  const oldManifest = JSON.parse(readFileSync(join(baseline.root, 'manifest.json'), 'utf8'));
  assert.equal(oldManifest.version, '0.1.1', 'The comparison baseline must be the WebView 0.1.1 release');
  const version = JSON.parse(readFileSync(join(repository, 'package.json'), 'utf8')).version;
  const reportPath = resolve(option('--report', join(repository, 'dist', 'performance-windows.json')));
  const markdownPath = resolve(option('--markdown', join(repository, 'docs', 'performance-results-windows.md')));
  const dataRoot = join(repository, '.test-data', `benchmark-${Date.now()}`);
  const powershell = process.env.CODEX_PWSH_PATH || 'pwsh';
  const psScript = join(repository, 'scripts/benchmark/measure-windows.ps1');
  const probe = async (pid, extra = []) => {
    const { stdout } = await execute(powershell, ['-NoProfile', '-File', psScript, '-RootProcess', String(pid), '-DurationMs', String(sampleMs), ...extra], { windowsHide: true, encoding: 'utf8', maxBuffer: 1024 * 1024 });
    return JSON.parse(stdout.replace(/^\uFEFF/, ''));
  };
  const hardware = await probe(0, ['-Hardware']);
  const animatedPath = join(repository, 'resources/emotes/bee_mad.webp');
  const animation = readFileSync(animatedPath);
  assert.ok(animation.includes(Buffer.from('ANIM')), 'The test image must be animated WebP');
  const settings = { muted: true, volume: 0, keepLastPing: false, trigger: { vk: 117 }, emoteTrigger: 'off',
    clickPing: false, emoteClick: false, pingDurationS: 3.2, pingSizePx: 110, emoteSizePx: 150, wheelSizePx: 600 };
  const report = {
    product: 'LOLPing Lite', version, complete: false, date: new Date().toISOString(), hardware,
    baseline: { product: 'DBX lolPing Lite plugin', version: '0.1.1', renderer: 'WebView2', archiveSha256: baseline.archiveSha256,
      executableSha256: sha256(readFileSync(join(baseline.root, oldManifest.entrypoints.backend.executable))) },
    candidate: { renderer: 'Rust native', executableSha256: sha256(readFileSync(executable)), productionTrayEnabled: true },
    method: { repetitions, sampleWindowMs: sampleMs, idleSettleMs: settleMs, signalIntervalMs: 200, emoteIntervalMs: 1800,
      settings, animation: { name: 'bee_mad.webp', sha256: sha256(animation) },
      scope: 'Complete application-descendant tree, including helper, WebView2 descendants and native settings when open. The shared DBX host is excluded from the legacy baseline.',
      processIdentity: 'Every descendant must be created no earlier than its active parent. Get-Process StartTime must match CIM CreationDate within a 1ms comparison tolerance; PID reuse during sampling fails the measurement. Missing roots or unreadable counters are errors, not zero samples.',
      memory: 'Working set sums resident pages across processes and can double-count shared pages. Private commit is virtual committed memory, not resident RAM. GPU memory is excluded.',
      cpu: 'One-core-equivalent CPU percentage from process CPU-time deltas over the measured interval, not Task Manager normalized utilization. Exited children can make this an underestimate; the report records their count.',
      sampling: 'Memory is the end-of-window sample, not a peak or upper-bound guarantee. The two products run sequentially in alternating order on the same desktop.' },
    limitations: [
      'Old plugin settings-open and settings-closed scenarios are not measured: its React settings run inside the shared DBX host and cannot be isolated by launching the old sidecar.',
      'The old WebView2 runtime is the runtime currently installed on this machine, not a pinned historical browser runtime.',
      'Programmatic preview commands exercise the real renderers without synthesizing OS keyboard or mouse input; input-to-display latency and physical gesture feel are outside this benchmark.',
      'Both candidates are muted to use a consistent visual workload; these results do not quantify audio playback memory.',
      'Settings-window open/close samples use the normal native settings process. No private paths, command lines, usernames or machine identifiers are included.',
    ], runs: [], summaries: {}, comparisons: {},
  };
  const save = () => { mkdirSync(dirname(reportPath), { recursive: true }); writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n'); };
  async function runOne(kind, repetition) {
    const native = kind === 'native';
    const client = native ? new Rpc(root, join(dataRoot, `${kind}-${repetition}`), executable, { production: true })
      : new LegacyRpc(baseline.root, join(dataRoot, `${kind}-${repetition}`));
    const request = (method, params = {}) => client.request(native ? method : `lolping/${method}`, params);
    const run = { kind, repetition, samples: {}, skipped: native ? [] : ['settingsOpen', 'settingsClosed'] };
    report.runs.push(run);
    const sample = async scene => {
      console.log(`Measuring ${kind}, repetition ${repetition}/${repetitions}, ${scene}`);
      const result = await probe(client.child.pid);
      assert.ok(result.processCount >= 2, 'Application and input helper must both be in the process tree');
      if (native) assert.ok(!result.processes.some(p => /webview|webkit|chrome|msedge/i.test(p.name)), 'Native candidate must not start a browser process');
      else assert.ok(result.processes.some(p => /msedgewebview2/i.test(p.name)), 'Legacy baseline must actually include WebView2');
      run.samples[scene] = result;
      save();
      return result;
    };
    const load = async (scene, params, interval) => {
      let keepRunning = true;
      let count = 0;
      let loopError;
      const loop = (async () => {
        while (keepRunning) {
          const started = Date.now();
          await request('preview', params);
          count++;
          await delay(Math.max(0, interval - (Date.now() - started)));
        }
      })().catch(error => { loopError = error; });
      try {
        await delay(900);
        const measured = await sample(scene);
        measured.previewCount = count;
      } finally { keepRunning = false; await loop; }
      if (loopError) throw loopError;
    };
    try {
      if (!native) await client.initialize();
      await request('settings', settings);
      await request('start');
      await poll(client, native ? 'state' : 'lolping/state', state => state.status.state === 'running');
      const imported = native ? await request('import', { kind: 'image', path: animatedPath }) : await client.upload(animation, 'image');
      for (let i = 0; i < 12; i++) { await request('preview', { id: i % 2 ? 'danger' : 'generic', wheel: 'ping' }); await delay(100); }
      await delay(settleMs);
      await sample('idle');
      await load('signals', { id: 'danger', wheel: 'ping' }, 200);
      await delay(4000);
      await load('animatedEmote', { id: imported.id, wheel: 'emote' }, 1800);
      await delay(4000);
      if (native) {
        const mark = client.sequence;
        await request('ui/open');
        await client.waitFor(event => event.event === 'ui/ready', 30000, mark);
        const opened = await poll(client, 'diagnostics', state => state.settingsPid > 0);
        await delay(1500);
        await sample('settingsOpen');
        await request('ui/close');
        await poll(client, 'diagnostics', state => state.settingsPid === null);
        assert.notEqual(opened.settingsPid, null);
        await delay(settleMs);
        await sample('settingsClosed');
      }
      await request('stop');
    } finally { await client.close(); }
  }
  for (let repetition = 1; repetition <= repetitions; repetition++) {
    const order = repetition % 2 ? ['webview', 'native'] : ['native', 'webview'];
    for (const kind of order) { await runOne(kind, repetition); await delay(1500); }
  }
  for (const kind of ['webview', 'native']) {
    report.summaries[kind] = {};
    for (const scene of ['idle', 'signals', 'animatedEmote', 'settingsOpen', 'settingsClosed']) {
      const samples = report.runs.filter(r => r.kind === kind).map(r => r.samples[scene]).filter(Boolean);
      if (!samples.length) continue;
      report.summaries[kind][scene] = { samples: samples.length,
        workingSetBytes: median(samples.map(s => s.workingSetBytes)),
        privateCommitBytes: median(samples.map(s => s.privateCommitBytes)),
        cpuOneCorePercent: median(samples.map(s => s.cpuOneCorePercent)),
        processCounts: samples.map(s => s.processCount), };
    }
  }
  for (const scene of ['idle', 'signals', 'animatedEmote']) {
    const before = report.summaries.webview[scene], after = report.summaries.native[scene];
    report.comparisons[scene] = Object.fromEntries(['workingSetBytes', 'privateCommitBytes'].map(key => [key.replace('Bytes', 'ReductionPercent'), 100 * (before[key] - after[key]) / before[key]]));
  }
  report.complete = true;
  save();
  writeFileSync(join(repository, 'docs/performance-windows.json'), JSON.stringify(report, null, 2) + '\n');
  const labels = { idle: '后台空闲', signals: '连续信号（5 次/秒）', animatedEmote: '动态表情', settingsOpen: '设置打开、无交互', settingsClosed: '关闭设置后空闲' };
  const rows = [];
  for (const scene of Object.keys(labels)) {
    for (const kind of ['webview', 'native']) {
      const s = report.summaries[kind][scene];
      rows.push(`| ${labels[scene]} | ${kind === 'native' ? `独立版 ${version}` : '旧 WebView 插件 0.1.1'} | ${s ? mib(s.workingSetBytes) : '未测'} | ${s ? mib(s.privateCommitBytes) : '未测'} | ${s ? s.cpuOneCorePercent.toFixed(2) + '%' : '未测'} |`);
    }
  }
  const reduction = report.comparisons.idle;
  const signalCpuBefore = report.summaries.webview.signals.cpuOneCorePercent;
  const signalCpuAfter = report.summaries.native.signals.cpuOneCorePercent;
  const settingsCpuSamples = report.runs.filter(r => r.kind === 'native').map(r => r.samples.settingsOpen.cpuOneCorePercent.toFixed(2) + '%');
  const noCpuDecrease = signalCpuAfter >= signalCpuBefore && report.summaries.native.animatedEmote.cpuOneCorePercent >= report.summaries.webview.animatedEmote.cpuOneCorePercent;
  const cpuText = `连续信号的 CPU 中位数从单核等效 ${signalCpuBefore.toFixed(2)}% 变为 ${signalCpuAfter.toFixed(2)}%；动态表情从 ${report.summaries.webview.animatedEmote.cpuOneCorePercent.toFixed(2)}% 变为 ${report.summaries.native.animatedEmote.cpuOneCorePercent.toFixed(2)}%。${noCpuDecrease ? '本组样本没有显示 CPU 占用下降。' : ''}不能将内存降幅解释为 CPU 改善。设置打开的 CPU 样本为 ${settingsCpuSamples.join('、')}，表中展示中位数。0.00% 表示短采样窗内未观察到可分辨的 CPU 时间增量，不代表进程绝对不消耗 CPU。\n\n`;
  let lifecycleText = '';
  const lifecyclePath = join(repository, 'dist/windows-lifecycle-report.json');
  if (existsSync(lifecyclePath)) {
    const lifecycle = JSON.parse(readFileSync(lifecyclePath, 'utf8'));
    if (lifecycle.complete && lifecycle.version === version) {
      writeFileSync(join(repository, 'docs/windows-lifecycle-report.json'), JSON.stringify(lifecycle, null, 2) + '\n');
      const trend = lifecycle.memoryTrend;
      lifecycleText = `生产模式另完成 20 次设置窗口打开／关闭：主进程私有提交的前五次中位数为 ${mib(trend.privateCommitFirstFiveMedianBytes)} MiB、后五次为 ${mib(trend.privateCommitLastFiveMedianBytes)} MiB，相差 ${(trend.privateCommitMedianChangeBytes / 1024).toFixed(0)} KiB（${trend.privateCommitMedianChangePercent.toFixed(2)}%）。设置子进程均已退出，输入 helper 全程未重启。重复启动、并发启动及 \`--quit\` 检查通过。完整序列见 [生命周期原始记录](windows-lifecycle-report.json)。\n\n`;
    }
  }
  const markdown = `# Windows 同机性能实测\n\n测量时间：${new Intl.DateTimeFormat('en-CA', { timeZone: 'Asia/Shanghai', year: 'numeric', month: '2-digit', day: '2-digit' }).format(new Date())}（Asia/Shanghai）。每种方案重复 ${repetitions} 次，表中取中位数。\n\n` +
    `后台空闲的进程树工作集由 **${mib(report.summaries.webview.idle.workingSetBytes)} MiB** 降至 **${mib(report.summaries.native.idle.workingSetBytes)} MiB**，降低 **${reduction.workingSetReductionPercent.toFixed(1)}%**；私有提交降低 **${reduction.privateCommitReductionPercent.toFixed(1)}%**。这是一台 Windows 机器上的测量，不是所有平台或负载的保证。\n\n` +
    `| 场景 | 方案 | 工作集 MiB | 私有提交 MiB | CPU（单核等效） |\n|---|---|---:|---:|---:|\n${rows.join('\n')}\n\n` +
    cpuText + lifecycleText + `环境：${hardware.os} ${hardware.osVersion}；${hardware.cpu}；${hardware.logicalProcessors} 逻辑处理器；${hardware.memoryMiB} MiB 内存。显示器：${hardware.displays.map(d => `${d.width}×${d.height}，${d.scalePercent}% 缩放`).join('；')}。\n\n` +
    `统计完整应用进程树，包含输入 helper、旧 WebView2 子进程和打开时的原生设置进程；新应用启用生产托盘。旧方案排除无法单独归属的共享 DBX 宿主，因此其“设置打开／关闭”两项未测，不能据此作设置页的同比结论。\n\n` +
    `两种方案使用相同尺寸、默认 9 表情及同一份 bee_mad.webp 动画，静音；连续信号通过真实渲染器的预览命令每 200 ms 发出，动态表情每 1.8 秒重新播放。空闲等待 22 秒回收缓存，采样窗 ${sampleMs / 1000} 秒。设置开关另由桌面验收执行 20 次。\n\n` +
    `工作集相加可能重复计算共享页，私有提交不等于驻留物理内存；GPU 内存未计入。内存为采样窗末的瞬时读数，不代表峰值上限。CPU 为进程累计 CPU 时间差折算的单核占用，和任务管理器按整机归一化的百分比不同。若采样期间有子进程退出，CPU 可能低估，原始数据记录了退出计数。\n\n` +
    `测试使用当前安装的 WebView2 runtime，未固定历史浏览器版本；未模拟实际鼠标手势或统计输入到屏幕延迟。完整匿名原始数据：[performance-windows.json](performance-windows.json)，同时作为 Release 附件；方法与复现要求见 [性能测试说明](performance.md)。\n`;
  mkdirSync(dirname(markdownPath), { recursive: true });
  writeFileSync(markdownPath, markdown);
  console.log(JSON.stringify({ summaries: report.summaries, comparisons: report.comparisons }, null, 2));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
