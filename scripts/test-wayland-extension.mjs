import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs';
import {
    BUS_NAME, OBJECT_PATH, INTERFACE, PROTOCOL_VERSION, MAX_FRAME_BYTES, MAX_ENCODED_BYTES, SHORTCUTS,
    parseMessage, validateDirectory, validatePrivateFile, validateFrame,
    validateConfiguration, validateRuntime, validateFrameFile, readPngHeader, BridgeSession, Gesture, DesktopSession,
} from '../native/gnome-extension/lolping-lite@ezreal-byte.github.io/protocol.js';

const runtime = '/run/user/1000';
const directory = `${runtime}/lolping-lite-frames-1234-1234abcd`;
const monitors = [
    {id: 0, x: 0, y: 0, width: 1920, height: 1080, scale: 1},
    {id: 1, x: -1280, y: -100, width: 1280, height: 720, scale: 2},
];
const frame = {monitor: 0, x: 100, y: 100, width: 300, height: 300, scale: 1, file: 'frame-0.png', encodedBytes: 1000};
const privateFile = {type: 'regular', symlink: false, uid: 1000, mode: 0o100600};
const config = {enabled: true, suspended: false, emotesEnabled: true};

test('packaged interface and GNOME version are explicit', () => {
    assert.equal(BUS_NAME, 'io.github.EzrealByte.LOLPingLite.Wayland');
    assert.equal(OBJECT_PATH, '/io/github/EzrealByte/LOLPingLite/Wayland');
    assert.equal(INTERFACE, `${BUS_NAME}1`);
    assert.equal(PROTOCOL_VERSION, 2);
    assert.deepEqual(Object.keys(SHORTCUTS), ['ping', 'emote', 'pause']);
    const metadata = JSON.parse(fs.readFileSync(new URL('../native/gnome-extension/lolping-lite@ezreal-byte.github.io/metadata.json', import.meta.url), 'utf8'));
    assert.deepEqual(metadata['shell-version'], ['46']);
    assert.equal(metadata.uuid, 'lolping-lite@ezreal-byte.github.io');
    assert.equal(metadata['session-modes'], undefined, 'never run on the lock screen');
});

test('bounded messages reject arrays, primitives, invalid JSON and oversized text', () => {
    assert.deepEqual(parseMessage('{"enabled":true}'), {enabled: true});
    for (const value of ['null', 'true', '1', '[]', '{} trailing', '{', ' '.repeat(4097), null])
        assert.throws(() => parseMessage(value));
});

test('frame directory must be one canonically named direct runtime child', () => {
    assert.equal(validateDirectory(directory, runtime), directory);
    for (const path of [
        '/tmp/lolping-lite-frames-1234-1234abcd', `${runtime}/../1001/lolping-lite-frames-1234-1234abcd`,
        `${directory}/child`, `${directory}/..`, `${runtime}//lolping-lite-frames-1234-1234abcd`,
        `${runtime}/lolping-lite-frames-0-1234abcd`, `${runtime}/lolping-lite-frames-1-short`,
        `${directory}\0`, `${runtime}0/lolping-lite-frames-1234-1234abcd`,
    ])
        assert.throws(() => validateDirectory(path, runtime), path);
});

test('files and directories must be private, regular and owned by the runtime user', () => {
    assert.doesNotThrow(() => validatePrivateFile(privateFile, 1000));
    assert.doesNotThrow(() => validatePrivateFile({...privateFile, type: 'directory', mode: 0o40700}, 1000, true));
    for (const patch of [
        {uid: 1001}, {symlink: true}, {type: 'directory'}, {type: 'other'},
        {mode: 0o644}, {mode: 0o666}, {mode: 0o700}, {mode: 0o4600}, {mode: 0o400},
    ])
        assert.throws(() => validatePrivateFile({...privateFile, ...patch}, 1000));
    assert.throws(() => validatePrivateFile({...privateFile, type: 'directory', mode: 0o755}, 1000, true));
});

test('runtime validation rejects symlinks in ancestors as well as the final directory and checks process uid', () => {
    const ordinary = {type: 'directory', symlink: false, uid: 0, mode: 0o40755};
    const privateDirectory = {...ordinary, uid: 1000, mode: 0o40700};
    const query = path => path === runtime ? privateDirectory : ordinary;
    assert.doesNotThrow(() => validateRuntime(runtime, 1000, query));
    assert.throws(() => validateRuntime(runtime, 1001, query));
    for (const path of ['/run', '/run/user', runtime]) {
        assert.throws(() => validateRuntime(runtime, 1000, candidate => ({
            ...query(candidate), symlink: candidate === path,
        })), path);
        assert.throws(() => validateRuntime(runtime, 1000, candidate => ({
            ...query(candidate), type: candidate === path ? 'regular' : 'directory',
        })), path);
    }
    for (const path of ['/run//user/1000', '/run/./user/1000', '/run/user/../1000', runtime + '/', 'run/user/1000'])
        assert.throws(() => validateRuntime(path, 1000, query));
});

test('valid cropped frames support negative monitor origins and mixed scale', () => {
    assert.equal(validateFrame(frame, monitors).bytes, 360000);
    const second = {...frame, monitor: 1, x: -1270, y: -90, width: 600, height: 600, scale: 2, file: 'frame-1.png'};
    assert.equal(validateFrame(second, monitors).bytes, 1440000);
    assert.equal(validateFrame({...frame, x: -0.5}, monitors).x, -0.5, 'outward pixel rounding is allowed');
});

test('invalid sizes, scale, coordinates, monitors and traversal are rejected', () => {
    for (const patch of [
        {width: 0}, {height: -1}, {width: 1.5}, {height: NaN}, {width: Infinity},
        {width: 4097}, {monitor: 99}, {monitor: '0'}, {scale: 2}, {scale: 0},
        {scale: Infinity}, {x: NaN}, {x: -2}, {y: 1000}, {x: 1900},
        {file: '../frame-0.png'}, {file: '/tmp/frame-0.png'}, {file: 'frame-1.png'},
        {file: 'frame-0.png\0'}, {file: 'frame-0.rgba'}, {encodedBytes: undefined},
        {encodedBytes: 32}, {encodedBytes: 1.5}, {encodedBytes: Infinity},
        {encodedBytes: MAX_ENCODED_BYTES + 1},
    ])
        assert.throws(() => validateFrame({...frame, ...patch}, monitors), JSON.stringify(patch));
    assert.throws(() => validateFrame({...frame, width: 4096, height: 4096},
        [{...monitors[0], width: 5000, height: 5000}]));
    assert.equal(MAX_FRAME_BYTES, 32 * 1024 * 1024);
});

test('configuration uses booleans and ignores unrelated fields', () => {
    assert.deepEqual(validateConfiguration({...config, extra: true}), config);
    for (const key of Object.keys(config)) {
        for (const invalid of [undefined, null, 1, 'true'])
            assert.throws(() => validateConfiguration({...config, [key]: invalid}));
    }
});

test('encoded files require unchanged byte length and permissions before and after decoding', () => {
    const info = {...privateFile, size: frame.encodedBytes};
    assert.doesNotThrow(() => validateFrameFile(info, 1000, frame));
    for (const patch of [{size: 999}, {size: 1001}, {size: undefined}, {symlink: true}, {mode: 0o644}, {uid: 1001}])
        assert.throws(() => validateFrameFile({...info, ...patch}, 1000, frame));
    assert.equal(validateFrame({...frame, encodedBytes: 33}, monitors).encodedBytes, 33);
    assert.equal(validateFrame({...frame, encodedBytes: MAX_ENCODED_BYTES}, monitors).encodedBytes, MAX_ENCODED_BYTES);
});

function pngHeader() {
    // PNG signature followed by a 300 x 300, 8-bit RGBA IHDR. Cairo validates
    // CRCs and image data; this bounded prefix check precedes native allocation.
    return Uint8Array.from(Buffer.from('89504e470d0a1a0a0000000d494844520000012c0000012c0806000000' + '00000000', 'hex'));
}

test('only 33 PNG header bytes enter JavaScript, and read errors or truncation are explicit', () => {
    const header = pngHeader();
    const read = (path, count) => {
        assert.equal(path, `${directory}/frame-0.png`);
        assert.equal(count, 33, 'never read full frame pixel arrays into GJS');
        return header;
    };
    assert.equal(readPngHeader(`${directory}/frame-0.png`, frame, read), header);
    for (const result of [null, [], 'pixels', [...header], new Uint32Array(33),
        new Uint8Array(0), new Uint8Array(32), new Uint8Array(34)])
        assert.throws(() => readPngHeader('frame', frame, () => result), /truncated or invalid/);
    assert.throws(() => readPngHeader('frame', frame, () => {
        throw new Error('Permission denied');
    }), /Cannot read the native overlay PNG header: Permission denied/);
    const subarray = new Uint8Array(40);
    subarray.set(header, 3);
    assert.equal(readPngHeader('frame', frame, () => subarray.subarray(3, 36)).length, 33);
});

test('PNG signature, IHDR, dimensions and encoding are checked before native decoding', () => {
    for (const index of [0, 1, 2, 3, 4, 5, 6, 7, 8, 11, 12, 15]) {
        const header = pngHeader();
        header[index] ^= 1;
        assert.throws(() => readPngHeader('frame', frame, () => header), /PNG IHDR/);
    }
    for (const [offset, value] of [[16, 0], [16, 301], [16, 0xffffffff], [20, 0], [20, 299], [20, 100000]]) {
        const header = pngHeader();
        new DataView(header.buffer).setUint32(offset, value);
        assert.throws(() => readPngHeader('frame', frame, () => header), /dimensions/);
    }
    for (const [index, value] of [[24, 16], [25, 2], [25, 3], [26, 1], [27, 1], [28, 1]]) {
        const header = pngHeader();
        header[index] = value;
        assert.throws(() => readPngHeader('frame', frame, () => header), /8-bit non-interlaced RGBA/);
    }
});

test('unique session owner is required for every operation and cannot be replaced', () => {
    const session = new BridgeSession();
    assert.throws(() => session.authorize(':1.42'));
    assert.throws(() => session.connect('fake.bus.name', directory));
    session.connect(':1.42', directory);
    assert.throws(() => session.connect(':1.43', directory));
    assert.throws(() => session.connect(':1.42', directory));
    assert.throws(() => session.configure(':1.43', config));
    assert.equal(session.config.enabled, false);
    session.configure(':1.42', config);
    assert.equal(session.canTrigger('ping'), true);
    session.disconnect();
    assert.equal(session.canTrigger('ping'), false);
    assert.equal(session.canTrigger('pause'), false);
    assert.throws(() => session.authorize(':1.42'));
    session.connect(':1.43', directory);
    assert.equal(session.config.enabled, false);
});

test('pause shortcut survives pause, suspension disables all, emotes are optional', () => {
    const session = new BridgeSession();
    session.connect(':1.42', directory);
    session.configure(':1.42', {...config, emotesEnabled: false});
    assert.equal(session.canTrigger('emote'), false);
    assert.equal(session.canTrigger('ping'), true);
    session.configure(':1.42', {...config, enabled: false});
    assert.equal(session.canTrigger('ping'), false);
    assert.equal(session.canTrigger('pause'), true);
    session.configure(':1.42', {...config, suspended: true});
    for (const kind of ['ping', 'emote', 'pause'])
        assert.equal(session.canTrigger(kind), false);
});

test('a failed grab emits nothing and repeated activation cannot replace the active wheel', () => {
    const gesture = new Gesture();
    assert.equal(gesture.begin('ping', 12, 34, false), null);
    assert.equal(gesture.wheel, null);
    assert.deepEqual(gesture.begin('ping', 12, 34, true), {type: 'wheelOpen', wheel: 'ping', x: 12, y: 34});
    assert.equal(gesture.begin('emote', 56, 78, true), null);
    assert.equal(gesture.wheel, 'ping');
});

test('movement coalesces while release uses the latest coordinates, not the pending frame', () => {
    const gesture = new Gesture();
    gesture.begin('ping', 0, 0, true);
    gesture.move(10, 10);
    gesture.move(20, 20);
    assert.deepEqual(gesture.flush(), {type: 'wheelMove', wheel: 'ping', x: 20, y: 20});
    assert.equal(gesture.flush(), null);
    gesture.move(30, 30);
    assert.deepEqual(gesture.end(false, 40, 50), {type: 'wheelRelease', wheel: 'ping', x: 40, y: 50});
    assert.equal(gesture.flush(), null);
    assert.equal(gesture.end(false, 0, 0), null);
});

test('only the triggering key release or left click confirms; Escape and right press cancel', () => {
    const gesture = new Gesture();
    gesture.begin('ping', 0, 0, true);
    assert.equal(gesture.action('key-release', 'q'), 'release');
    assert.equal(gesture.action('key-release', 'Q'), 'release');
    assert.equal(gesture.action('key-release', 'Alt_L'), null);
    assert.equal(gesture.action('key-release', 'e'), null);
    assert.equal(gesture.action('key-press', 'q'), null);
    assert.equal(gesture.action('key-press', 'Escape'), 'cancel');
    assert.equal(gesture.action('button-press', 3), 'cancel');
    assert.equal(gesture.action('button-release', 1), null, 'a press predating the wheel cannot confirm');
    assert.equal(gesture.action('button-press', 1), null);
    assert.equal(gesture.action('button-release', 1), 'release');
    assert.deepEqual(gesture.end(true, 99, 99), {type: 'cancel', wheel: 'ping'});
    assert.equal(gesture.action('key-release', 'q'), null);
    gesture.begin('emote', -100, 200, true);
    assert.equal(gesture.action('key-release', 'q'), null);
    assert.equal(gesture.action('key-release', 'e'), 'release');
});

test('left confirmation consumes its own press and release then subsequent keyboard and mouse events are unhandled', () => {
    const gesture = new Gesture();
    gesture.begin('ping', 0, 0, true);
    assert.equal(gesture.action('button-press', 1), null);
    assert.equal(gesture.action('button-release', 1), 'release');
    assert.equal(gesture.end(false, 99, 42).type, 'wheelRelease');
    assert.equal(gesture.wheel, null, 'extension removes its modal and capture when the gesture ends');
    for (const [type, value] of [
        ['key-release', 'q'], ['key-press', 'a'], ['key-release', 'a'],
        ['key-release', 'Alt_L'], ['key-release', 'Super_L'],
        ['button-press', 1], ['button-release', 1],
    ])
        assert.equal(gesture.action(type, value), null, `${type}: ${value}`);
    gesture.begin('emote', 50, 60, true);
    assert.equal(gesture.action('button-release', 1), null, 'confirmation press state never carries over');
});

test('cancellation drops queued movement and permits a fresh wheel after disconnect or display change', () => {
    const gesture = new Gesture();
    for (let i = 0; i < 20; i++) {
        assert.ok(gesture.begin('ping', -100, -100, true));
        gesture.move(100, 100);
        assert.equal(gesture.end(true, 0, 0).type, 'cancel');
        assert.equal(gesture.flush(), null);
        assert.equal(gesture.wheel, null);
    }
});

test('overview, lock and sleep cancel before temporary unavailability and recover without disconnecting', () => {
    const desktop = new DesktopSession();
    const gesture = new Gesture();
    const session = new BridgeSession();
    session.connect(':1.42', directory);
    session.configure(':1.42', config);
    for (const _reason of ['overview', 'lock', 'sleep']) {
        gesture.begin('ping', 30, 40, true);
        gesture.move(50, 60);
        assert.deepEqual(desktop.update(false, gesture), [
            {type: 'cancel', wheel: 'ping'}, {type: 'session', available: false},
        ]);
        assert.equal(gesture.wheel, null);
        assert.equal(gesture.flush(), null);
        assert.deepEqual(desktop.update(false, gesture), []);
        assert.deepEqual(desktop.update(true, gesture), [{type: 'session', available: true}]);
        assert.equal(session.owner, ':1.42');
        assert.equal(session.canTrigger('ping'), true);
    }
    assert.deepEqual(desktop.update(true, gesture), []);
    assert.equal(new DesktopSession(false).available, false, 'hello can start while the desktop is unavailable');
});
