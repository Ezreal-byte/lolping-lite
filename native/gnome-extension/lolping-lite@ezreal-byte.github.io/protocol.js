// SPDX-License-Identifier: MIT
// Shared with the Node regression tests; this module has no GNOME dependencies.
export const BUS_NAME = 'io.github.EzrealByte.LOLPingLite.Wayland';
export const OBJECT_PATH = '/io/github/EzrealByte/LOLPingLite/Wayland';
export const INTERFACE = 'io.github.EzrealByte.LOLPingLite.Wayland1';
export const PROTOCOL_VERSION = 2;
export const MAX_FRAME_BYTES = 32 * 1024 * 1024;
export const MAX_ENCODED_BYTES = MAX_FRAME_BYTES + 65536;
export const PNG_HEADER_BYTES = 33;
export const SHORTCUTS = Object.freeze({
    ping: '<Super><Alt>q',
    emote: '<Super><Alt>e',
    pause: '<Super><Alt>p',
});

export function parseMessage(text) {
    if (typeof text !== 'string' || text.length > 4096)
        throw new Error('Invalid or oversized bridge message');
    const value = JSON.parse(text);
    if (!value || typeof value !== 'object' || Array.isArray(value))
        throw new Error('Expected a bridge object');
    return value;
}

export function validateDirectory(path, runtime) {
    if (typeof path !== 'string' || typeof runtime !== 'string' ||
        !runtime.startsWith('/') || runtime.endsWith('/') || runtime.includes('\0'))
        throw new Error('Invalid user runtime directory');
    const prefix = `${runtime}/`;
    if (!path.startsWith(prefix) ||
        !/^lolping-lite-frames-[1-9][0-9]*-[A-Za-z0-9_-]{8,128}$/.test(path.slice(prefix.length)))
        throw new Error('Frame directory must be a private LOLPing Lite runtime directory');
    return path;
}

export function validatePrivateFile(info, uid, directory = false) {
    if (info.symlink || info.type !== (directory ? 'directory' : 'regular') ||
        info.uid !== uid || (info.mode & 0o7777) !== (directory ? 0o700 : 0o600))
        throw new Error('Bridge files must be owned by this user, private, and not symbolic links');
}

export function validateRuntime(runtime, uid, queryInfo) {
    if (typeof runtime !== 'string' || !runtime.startsWith('/') || runtime.includes('\0'))
        throw new Error('Invalid user runtime directory');
    const parts = runtime.slice(1).split('/');
    if (parts.some(part => !part || part === '.' || part === '..'))
        throw new Error('The runtime path must be canonical');
    let prefix = '';
    let info;
    for (const part of parts) {
        prefix += `/${part}`;
        info = queryInfo(prefix);
        if (info.symlink || info.type !== 'directory')
            throw new Error('Runtime path components cannot be symbolic links');
    }
    validatePrivateFile(info, uid, true);
}

export function validateFrame(value, monitors) {
    const monitor = monitors.find(item => item.id === value.monitor);
    if (!monitor || !Number.isInteger(value.monitor))
        throw new Error('Unknown monitor');
    for (const key of ['width', 'height']) {
        if (!Number.isInteger(value[key]) || value[key] < 1 || value[key] > 4096)
            throw new Error('Frame dimensions must be between 1 and 4096 pixels');
    }
    const bytes = value.width * value.height * 4;
    if (bytes > MAX_FRAME_BYTES)
        throw new Error('Frame exceeds the 32 MiB limit');
    if (!Number.isFinite(value.scale) || value.scale < 0.5 || value.scale > 8 ||
        Math.abs(value.scale - monitor.scale) > 0.001)
        throw new Error('Frame scale does not match the monitor');
    if (!Number.isFinite(value.x) || !Number.isFinite(value.y) ||
        value.x < monitor.x - 1 || value.y < monitor.y - 1 ||
        value.x + value.width / value.scale > monitor.x + monitor.width + 1 ||
        value.y + value.height / value.scale > monitor.y + monitor.height + 1)
        throw new Error('Frame is outside its monitor');
    if (value.file !== `frame-${value.monitor}.png`)
        throw new Error('Unexpected frame filename');
    if (!Number.isInteger(value.encodedBytes) || value.encodedBytes < PNG_HEADER_BYTES ||
        value.encodedBytes > MAX_ENCODED_BYTES)
        throw new Error('Invalid encoded frame byte length');
    return {...value, bytes};
}

export function validateFrameFile(info, uid, frame) {
    validatePrivateFile(info, uid);
    if (info.size !== frame.encodedBytes)
        throw new Error('Frame file length changed or does not match its encoded byte length');
}

export function readPngHeader(path, frame, readBytes) {
    let header;
    try {
        header = readBytes(path, PNG_HEADER_BYTES);
    } catch (error) {
        throw new Error(`Cannot read the native overlay PNG header: ${error.message ?? error}`);
    }
    if (!(header instanceof Uint8Array) || header.byteLength !== PNG_HEADER_BYTES)
        throw new Error('The native overlay PNG header is truncated or invalid');
    const signature = [137, 80, 78, 71, 13, 10, 26, 10];
    const view = new DataView(header.buffer, header.byteOffset, header.byteLength);
    if (signature.some((value, index) => header[index] !== value) ||
        view.getUint32(8) !== 13 || view.getUint32(12) !== 0x49484452)
        throw new Error('The native overlay must begin with a PNG IHDR header');
    if (view.getUint32(16) !== frame.width || view.getUint32(20) !== frame.height)
        throw new Error('PNG dimensions do not match the bounded frame');
    // Restrict native decoding to our encoder's 8-bit non-interlaced RGBA
    // output, after checking the dimensions and before allocating image data.
    if (header[24] !== 8 || header[25] !== 6 ||
        header[26] !== 0 || header[27] !== 0 || header[28] !== 0)
        throw new Error('The native overlay must be an 8-bit non-interlaced RGBA PNG');
    return header;
}

export function validateConfiguration(value) {
    for (const key of ['enabled', 'suspended', 'emotesEnabled']) {
        if (typeof value[key] !== 'boolean')
            throw new Error(`Configuration field ${key} must be a boolean`);
    }
    return {enabled: value.enabled, suspended: value.suspended, emotesEnabled: value.emotesEnabled};
}

export class BridgeSession {
    constructor() {
        this.owner = null;
        this.directory = null;
        this.config = {enabled: false, suspended: false, emotesEnabled: false};
    }

    connect(sender, directory) {
        if (typeof sender !== 'string' || !/^:[0-9]+\.[0-9]+$/.test(sender))
            throw new Error('A unique D-Bus sender is required');
        if (this.owner)
            throw new Error('A LOLPing Lite application is already connected');
        this.owner = sender;
        this.directory = directory;
    }

    authorize(sender) {
        if (!this.owner || sender !== this.owner)
            throw new Error('Only the connected LOLPing Lite application may use this bridge');
    }

    configure(sender, value) {
        this.authorize(sender);
        this.config = validateConfiguration(value);
    }

    canTrigger(wheel) {
        return Boolean(this.owner && !this.config.suspended &&
            (wheel === 'pause' || this.config.enabled &&
                (wheel === 'ping' || wheel === 'emote' && this.config.emotesEnabled)));
    }

    disconnect() {
        this.owner = null;
        this.directory = null;
        this.config = {enabled: false, suspended: false, emotesEnabled: false};
    }
}

export class Gesture {
    constructor() {
        this.wheel = null;
        this.key = null;
        this.pending = null;
        this.leftPressed = false;
    }

    begin(wheel, x, y, grabbed) {
        if (!grabbed || this.wheel || !['ping', 'emote'].includes(wheel))
            return null;
        this.wheel = wheel;
        this.key = wheel === 'ping' ? 'q' : 'e';
        this.leftPressed = false;
        return {type: 'wheelOpen', wheel, x, y};
    }

    move(x, y) {
        if (this.wheel)
            this.pending = {type: 'wheelMove', wheel: this.wheel, x, y};
    }

    flush() {
        const event = this.pending;
        this.pending = null;
        return event;
    }

    action(type, value) {
        if (!this.wheel)
            return null;
        if (type === 'button-press' && value === 1) {
            this.leftPressed = true;
            return null;
        }
        if ((type === 'key-press' && value === 'Escape') || (type === 'button-press' && value === 3))
            return 'cancel';
        if ((type === 'key-release' && String(value).toLowerCase() === this.key) ||
            (type === 'button-release' && value === 1 && this.leftPressed))
            return 'release';
        return null;
    }

    end(cancel, x, y) {
        if (!this.wheel)
            return null;
        const event = cancel ? {type: 'cancel', wheel: this.wheel} :
            {type: 'wheelRelease', wheel: this.wheel, x, y};
        this.wheel = null;
        this.key = null;
        this.pending = null;
        this.leftPressed = false;
        return event;
    }
}

export class DesktopSession {
    constructor(available = true) {
        this.available = available;
    }

    update(available, gesture) {
        if (available === this.available)
            return [];
        this.available = available;
        const cancel = !available ? gesture.end(true, 0, 0) : null;
        return [...(cancel ? [cancel] : []), {type: 'session', available}];
    }
}
