// SPDX-License-Identifier: MIT
import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {
    BUS_NAME, OBJECT_PATH, INTERFACE, SHORTCUTS, BridgeSession, Gesture, DesktopSession,
    parseMessage, validateDirectory, validatePrivateFile, validateFrame, validateRuntime, readFramePixels,
} from './protocol.js';

const XML = `<node><interface name="${INTERFACE}">
  <method name="Connect"><arg type="s" direction="in" name="frameDirectory"/><arg type="s" direction="out" name="capabilities"/></method>
  <method name="Configure"><arg type="s" direction="in" name="configuration"/></method>
  <method name="Disconnect"/>
  <method name="Clear"><arg type="i" direction="in" name="monitor"/></method>
  <method name="Present"><arg type="s" direction="in" name="frame"/></method>
  <signal name="Event"><arg type="s" name="event"/></signal>
</interface></node>`;

const FILE_ATTRIBUTES = 'standard::type,standard::is-symlink,standard::size,unix::uid,unix::mode';

function fileInfo(path) {
    const info = Gio.File.new_for_path(path).query_info(
        FILE_ATTRIBUTES, Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS, null);
    const type = info.get_file_type();
    return {
        type: type === Gio.FileType.DIRECTORY ? 'directory' :
            type === Gio.FileType.REGULAR ? 'regular' : 'other',
        symlink: info.get_is_symlink(),
        size: info.get_size(),
        uid: info.get_attribute_uint32('unix::uid'),
        mode: info.get_attribute_uint32('unix::mode'),
    };
}

class Bridge {
    constructor() {
        this._session = new BridgeSession();
        this._gesture = new Gesture();
        this._frames = new Map();
        this._bindings = new Map();
        this._modal = null;
        this._modalActor = null;
        this._captureId = 0;
        this._moveSource = 0;
        this._timeoutSource = 0;
        this._ownerWatch = 0;
        this._unredirect = false;
        this._destroyed = false;
        this._sleeping = false;
        this._overviewActive = Main.overview.visible;
        this._desktop = new DesktopSession(this._desktopAvailable());
        this._bus = Gio.DBus.session;
        this._exported = Gio.DBusExportedObject.wrapJSObject(XML, this);
        this._exported.export(this._bus, OBJECT_PATH);
        this._name = Gio.bus_own_name_on_connection(this._bus, BUS_NAME,
            Gio.BusNameOwnerFlags.NONE, null, () => this._disconnect('name-lost'));
        this._activationId = global.display.connect('accelerator-activated',
            (_display, action) => {
                try {
                    this._activate(action);
                } catch (error) {
                    this._cancelAndClear();
                    this._emit({type: 'error', error: String(error.message ?? error)});
                }
            });
        this._monitorsId = Main.layoutManager.connect('monitors-changed', () => {
            this._cancelAndClear();
            this._emit({type: 'monitors', monitors: this._monitors()});
        });
        this._overviewId = Main.overview.connect('showing', () => {
            this._overviewActive = true;
            this._updateAvailability();
        });
        this._overviewHiddenId = Main.overview.connect('hidden', () => {
            this._overviewActive = false;
            this._updateAvailability();
        });
        this._sessionId = Main.sessionMode.connect('updated', () => this._updateAvailability());
        this._sleepId = Gio.DBus.system.signal_subscribe('org.freedesktop.login1',
            'org.freedesktop.login1.Manager', 'PrepareForSleep', '/org/freedesktop/login1',
            null, Gio.DBusSignalFlags.NONE, (_bus, _sender, _path, _interface, _signal, args) => {
                [this._sleeping] = args.deepUnpack();
                this._updateAvailability();
            });
    }

    _monitors() {
        return Main.layoutManager.monitors.map((monitor, id) => ({
            id, x: monitor.x, y: monitor.y, width: monitor.width, height: monitor.height,
            scale: global.display.get_monitor_scale(id),
        }));
    }

    _desktopAvailable() {
        return !this._destroyed && !this._sleeping && !Main.sessionMode.isLocked &&
            Main.sessionMode.hasWindows && !this._overviewActive;
    }

    _updateAvailability() {
        const available = this._desktopAvailable();
        const events = this._desktop.update(available, this._gesture);
        if (!available) {
            this._releaseModal();
            this._clear();
        }
        for (const event of events)
            this._emit(event);
    }

    _reply(invocation, fn, connect = false) {
        try {
            if (!connect)
                this._session.authorize(invocation.get_sender());
            const result = fn();
            invocation.return_value(connect ? new GLib.Variant('(s)', [JSON.stringify(result)]) : null);
        } catch (error) {
            invocation.return_dbus_error(`${INTERFACE}.Error`, String(error.message ?? error));
        }
    }

    ConnectAsync([directory], invocation) {
        this._reply(invocation, () => {
            if (!Meta.is_wayland_compositor())
                throw new Error('This bridge requires a GNOME Wayland session');
            const runtime = GLib.get_user_runtime_dir();
            validateDirectory(directory, runtime);
            if (GLib.canonicalize_filename(directory, null) !== directory)
                throw new Error('Frame directory must be canonical');
            const uid = Gio.Credentials.new().get_unix_user();
            validateRuntime(runtime, uid, fileInfo);
            validatePrivateFile(fileInfo(directory), uid, true);
            this._session.connect(invocation.get_sender(), directory);
            this._uid = uid;
            this._ownerWatch = Gio.bus_watch_name_on_connection(this._bus, this._session.owner,
                Gio.BusNameWatcherFlags.NONE, null, () => this._disconnect('application-exited'));
            return {
                protocol: 1, monitors: this._monitors(), session: 'gnome-wayland',
                shortcuts: SHORTCUTS, available: this._desktopAvailable(),
            };
        }, true);
    }

    ConfigureAsync([text], invocation) {
        this._reply(invocation, () => {
            this._session.configure(invocation.get_sender(), parseMessage(text));
            if (!this._session.config.enabled || this._session.config.suspended ||
                this._gesture.wheel === 'emote' && !this._session.config.emotesEnabled)
                this._cancelAndClear();
            try {
                this._syncBindings();
            } catch (error) {
                this._session.config.enabled = false;
                this._ungrabBindings();
                this._cancelAndClear();
                throw error;
            }
        });
    }

    DisconnectAsync(_args, invocation) {
        this._reply(invocation, () => this._disconnect('disconnected'));
    }

    ClearAsync([monitor], invocation) {
        this._reply(invocation, () => {
            if (monitor !== -1 && !this._monitors().some(item => item.id === monitor))
                throw new Error('Unknown monitor');
            this._clear(monitor);
        });
    }

    PresentAsync([text], invocation) {
        this._reply(invocation, () => {
            if (!this._desktopAvailable())
                throw new Error('desktopUnavailable');
            if (this._session.config.suspended)
                throw new Error('inputSuspended');
            const frame = validateFrame(parseMessage(text), this._monitors());
            const path = `${this._session.directory}/${frame.file}`;
            validatePrivateFile(fileInfo(this._session.directory), this._uid, true);
            const info = fileInfo(path);
            validatePrivateFile(info, this._uid);
            if (info.size !== frame.bytes)
                throw new Error('Frame file length does not match its dimensions');
            const pixels = readFramePixels(path, frame.bytes, filename => GLib.file_get_contents(filename));
            let entry = this._frames.get(frame.monitor);
            // GNOME 46 Clutter.Image derives its preferred size from the current
            // texture. St.ImageContent fixes that size at construction, which
            // does not fit cropped overlay frames whose dimensions can change.
            const content = entry?.content ?? new Clutter.Image();
            // GJS accounts for this Uint8Array's full allocation. Per-frame
            // MappedFile/GBytes wrappers would retain mappings until GC, and
            // retaining GBytes while Rust rewrites its data breaks immutability.
            // GNOME 46 set_data copies into texture memory before returning, so
            // this reply acknowledges that Rust may rewrite its private file.
            // https://gitlab.gnome.org/GNOME/mutter/-/blob/gnome-46/clutter/clutter/clutter-image.c
            if (!content.set_data(pixels, Cogl.PixelFormat.RGBA_8888_PRE,
                frame.width, frame.height, frame.width * 4))
                throw new Error('Cannot upload the native overlay frame');
            if (!entry) {
                const actor = new Clutter.Actor({reactive: false, content});
                Main.uiGroup.add_child(actor);
                entry = {actor, content};
                this._frames.set(frame.monitor, entry);
            }
            const {actor} = entry;
            actor.set_position(frame.x, frame.y);
            actor.set_size(frame.width / frame.scale, frame.height / frame.scale);
            actor.show();
            Main.uiGroup.set_child_above_sibling(actor, null);
            if (!this._unredirect) {
                Meta.disable_unredirect_for_display(global.display);
                this._unredirect = true;
            }
        });
    }

    _emit(event) {
        if (!event || !this._session.owner || this._destroyed)
            return;
        this._bus.emit_signal(this._session.owner, OBJECT_PATH, INTERFACE,
            'Event', new GLib.Variant('(s)', [JSON.stringify(event)]));
    }

    _syncBindings() {
        const wanted = Object.keys(SHORTCUTS).filter(kind => this._session.canTrigger(kind));
        for (const [action, kind] of this._bindings) {
            if (!wanted.includes(kind)) {
                global.display.ungrab_accelerator(action);
                Main.wm.allowKeybinding(Meta.external_binding_name_for_action(action), Shell.ActionMode.NONE);
                this._bindings.delete(action);
            }
        }
        for (const kind of wanted) {
            if ([...this._bindings.values()].includes(kind))
                continue;
            const action = global.display.grab_accelerator(SHORTCUTS[kind], Meta.KeyBindingFlags.IGNORE_AUTOREPEAT);
            if (action === Meta.KeyBindingAction.NONE)
                throw new Error(`Shortcut ${SHORTCUTS[kind]} is already used by another application or GNOME`);
            this._bindings.set(action, kind);
            Main.wm.allowKeybinding(Meta.external_binding_name_for_action(action), Shell.ActionMode.NORMAL);
        }
    }

    _ungrabBindings() {
        for (const action of this._bindings.keys()) {
            global.display.ungrab_accelerator(action);
            Main.wm.allowKeybinding(Meta.external_binding_name_for_action(action), Shell.ActionMode.NONE);
        }
        this._bindings.clear();
    }

    _activate(action) {
        const kind = this._bindings.get(action);
        if (!kind || !this._desktopAvailable() || !this._session.canTrigger(kind))
            return;
        if (kind === 'pause') {
            this._session.config.enabled = !this._session.config.enabled;
            if (!this._session.config.enabled)
                this._cancelAndClear();
            try {
                this._syncBindings();
                this._emit({type: 'toggled', enabled: this._session.config.enabled});
            } catch (error) {
                this._session.config.enabled = false;
                this._ungrabBindings();
                this._emit({type: 'error', error: String(error.message ?? error)});
            }
            return;
        }
        if (this._gesture.wheel)
            return;
        this._modalActor = new St.Widget({reactive: true, can_focus: true, width: 1, height: 1, opacity: 0});
        Main.uiGroup.add_child(this._modalActor);
        this._modal = Main.pushModal(this._modalActor, {actionMode: Shell.ActionMode.NONE});
        const required = Clutter.GrabState.KEYBOARD | Clutter.GrabState.POINTER;
        if ((this._modal.get_seat_state() & required) !== required) {
            this._releaseModal();
            this._emit({type: 'error', error: 'GNOME could not grab the pointer and keyboard for the wheel'});
            return;
        }
        const [x, y] = global.get_pointer();
        const event = this._gesture.begin(kind, x, y, true);
        // A Clutter seat grab routes events only inside the grabbed actor's
        // subtree; ancestors such as global.stage do not receive capture.
        // Attach to the same actor passed to pushModal, as GNOME menus do.
        this._captureId = this._modalActor.connect('captured-event', (_actor, input) => {
            try {
                return this._capture(input);
            } catch (error) {
                this._cancelAndClear();
                this._emit({type: 'error', error: String(error.message ?? error)});
                return Clutter.EVENT_STOP;
            }
        });
        this._timeoutSource = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 15, () => {
            this._timeoutSource = 0;
            // A held or interrupted gesture expires without disabling the
            // bridge; the next shortcut must still be able to open a wheel.
            this._cancelAndClear();
            return GLib.SOURCE_REMOVE;
        });
        this._emit(event);
    }

    _capture(event) {
        if (!this._modal || !this._gesture.wheel)
            return Clutter.EVENT_PROPAGATE;
        const type = event.type();
        if (type === Clutter.EventType.MOTION) {
            const [x, y] = event.get_coords();
            this._gesture.move(x, y);
            if (!this._moveSource) {
                this._moveSource = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 16, () => {
                    this._moveSource = 0;
                    this._emit(this._gesture.flush());
                    return GLib.SOURCE_REMOVE;
                });
            }
            return Clutter.EVENT_STOP;
        }
        let action = null;
        if (type === Clutter.EventType.KEY_PRESS || type === Clutter.EventType.KEY_RELEASE) {
            const key = event.get_key_symbol();
            const name = key === Clutter.KEY_Escape ? 'Escape' :
                String.fromCodePoint(Clutter.keysym_to_unicode(key));
            action = this._gesture.action(type === Clutter.EventType.KEY_PRESS ? 'key-press' : 'key-release', name);
        } else if (type === Clutter.EventType.BUTTON_PRESS || type === Clutter.EventType.BUTTON_RELEASE) {
            action = this._gesture.action(type === Clutter.EventType.BUTTON_PRESS ? 'button-press' : 'button-release', event.get_button());
        } else {
            return Clutter.EVENT_PROPAGATE;
        }
        if (action) {
            const [x, y] = global.get_pointer();
            const result = this._gesture.end(action === 'cancel', x, y);
            this._releaseModal();
            this._emit(result);
        }
        return Clutter.EVENT_STOP;
    }

    _releaseModal() {
        if (this._captureId) {
            this._modalActor.disconnect(this._captureId);
            this._captureId = 0;
        }
        for (const property of ['_moveSource', '_timeoutSource']) {
            if (this[property]) {
                GLib.Source.remove(this[property]);
                this[property] = 0;
            }
        }
        if (this._modal) {
            Main.popModal(this._modal);
            this._modal = null;
        }
        if (this._modalActor) {
            this._modalActor.destroy();
            this._modalActor = null;
        }
    }

    _clear(monitor = -1) {
        for (const [id, {actor}] of this._frames) {
            if (monitor === -1 || id === monitor) {
                actor.set_content(null);
                actor.destroy();
                this._frames.delete(id);
            }
        }
        if (this._frames.size === 0 && this._unredirect) {
            Meta.enable_unredirect_for_display(global.display);
            this._unredirect = false;
        }
    }

    _cancelAndClear() {
        const event = this._gesture.end(true, 0, 0);
        this._releaseModal();
        this._clear();
        this._emit(event);
    }

    _disconnect(reason) {
        this._cancelAndClear();
        this._emit({type: 'gone', reason});
        this._ungrabBindings();
        if (this._ownerWatch) {
            Gio.bus_unwatch_name(this._ownerWatch);
            this._ownerWatch = 0;
        }
        this._session.disconnect();
    }

    destroy() {
        this._disconnect('extension-disabled');
        this._destroyed = true;
        global.display.disconnect(this._activationId);
        Main.layoutManager.disconnect(this._monitorsId);
        Main.overview.disconnect(this._overviewId);
        Main.overview.disconnect(this._overviewHiddenId);
        Main.sessionMode.disconnect(this._sessionId);
        Gio.DBus.system.signal_unsubscribe(this._sleepId);
        this._exported.unexport();
        Gio.bus_unown_name(this._name);
    }
}

export default class LOLPingLiteWaylandExtension extends Extension {
    enable() {
        this._bridge = new Bridge();
    }

    disable() {
        this._bridge?.destroy();
        this._bridge = null;
    }
}
