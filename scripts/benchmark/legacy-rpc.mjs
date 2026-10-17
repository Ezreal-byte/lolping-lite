// Legacy DBX framed transport from Ezreal-byte/dbx-lolping-plugin (MIT), retained
// only to benchmark the unmodified 0.1.1 release. Production has no DBX SDK.
import { spawn } from 'node:child_process';
import { EventEmitter } from 'node:events';
import { resolve, join } from 'node:path';
import { readFileSync, mkdirSync } from 'node:fs';
export class Rpc extends EventEmitter {
  next = 0; pending = new Map(); buffer = Buffer.alloc(0); history = []; log = '';
  constructor(stage, data) {
    super(); this.stage = resolve(stage); this.data = resolve(data); mkdirSync(this.data, { recursive: true });
    const manifest = this.manifest = JSON.parse(readFileSync(join(this.stage, 'manifest.json'), 'utf8'));
    this.child = spawn(join(this.stage, manifest.entrypoints.backend.executable), [], { cwd: this.stage, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'], env: { ...process.env, DBX_PLUGIN_DATA_DIR: this.data, LOLPING_ROOT: this.stage } });
    this.child.stderr.on('data', b => { this.log += b.toString(); if (this.log.length > 32768) this.log = this.log.slice(-32768); });
    this.child.stdout.on('data', b => {
      this.buffer = Buffer.concat([this.buffer, b]);
      while (this.buffer.length >= 5) {
        const kind = this.buffer[0], size = this.buffer.readUInt32BE(1); if (size > 64 * 1048576) throw new Error('Oversized protocol frame');
        if (this.buffer.length < size + 5) return;
        const payload = this.buffer.subarray(5, size + 5); this.buffer = this.buffer.subarray(size + 5);
        if (kind !== 0) continue;
        const message = JSON.parse(payload.toString());
        if ('id' in message && this.pending.has(message.id)) { const p = this.pending.get(message.id); this.pending.delete(message.id); clearTimeout(p.timer); message.error ? p.reject(new Error(message.error.message)) : p.resolve(message.result); }
        else { this.history.push(message); if (this.history.length > 100) this.history.shift(); this.emit('event', message); }
      }
    });
    this.child.on('error', e => this.rejectAll(e));
    this.child.on('exit', (code, signal) => { this.rejectAll(new Error(`Sidecar exited (${code}, ${signal}): ${this.log}`)); this.emit('exit'); });
  }
  rejectAll(error) { for (const p of this.pending.values()) { clearTimeout(p.timer); p.reject(error); } this.pending.clear(); }
  frame(kind, payload) { const h = Buffer.alloc(5); h[0] = kind; h.writeUInt32BE(payload.length, 1); this.child.stdin.write(Buffer.concat([h, payload])); }
  request(method, params = {}) { const id = ++this.next; return new Promise((resolve, reject) => { const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`RPC timed out: ${method}\n${this.log}`)); }, 30000); this.pending.set(id, { resolve, reject, timer }); this.frame(0, Buffer.from(JSON.stringify({ jsonrpc: '2.0', id, method, params }))); }); }
  binary(channel, data) { const c = Buffer.from(channel), n = Buffer.alloc(2); n.writeUInt16BE(c.length); this.frame(1, Buffer.concat([n, c, data])); }
  initialize() { return this.request('plugin/initialize', { host: { dbxVersion: '0.5.68', hostApiVersion: '1.0.0', protocolVersions: [1] }, plugin: { id: 'io.dbx.lolping.lite', version: this.manifest.version }, permissions: ['host.events', 'host.binary'] }); }
  waitFor(predicate, timeout = 15000) { const found = this.history.find(predicate); if (found) return Promise.resolve(found); return new Promise((resolve, reject) => { const timer = setTimeout(() => { this.off('event', listener); reject(new Error(`Event timeout\n${this.log}`)); }, timeout); const listener = event => { if (predicate(event)) { clearTimeout(timer); this.off('event', listener); resolve(event); } }; this.on('event', listener); }); }
  async upload(bytes, kind, extra = {}) { const { id } = await this.request('lolping/upload/begin', { size: bytes.length, kind, name: 'Smoke emote', ...extra }); for (let offset = 0; offset < bytes.length; offset += 512 * 1024) {
    const b = bytes.subarray(offset, offset + 512 * 1024), h = Buffer.alloc(8); h.writeBigUInt64BE(BigInt(offset));
    const ack = this.waitFor(e => e.method === 'lolping/upload' && e.params.id === id && (e.params.error || e.params.received === offset + b.length));
    this.binary(`upload/${id}`, Buffer.concat([h, b])); const e = await ack; if (e.params.error) throw new Error(e.params.error);
  } return this.request('lolping/upload/commit', { id }); }
  async close() { if (this.child.exitCode !== null || this.child.signalCode !== null) return; this.child.stdin.end(); await new Promise(resolve => { const timer = setTimeout(() => { this.child.kill(); resolve(); }, 3000); this.child.once('exit', () => { clearTimeout(timer); resolve(); }); }); }
}
