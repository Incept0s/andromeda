// SPDX-License-Identifier: GPL-3.0-or-later
//
// Andromeda top-bar button. Deliberately thin: it draws an icon, forwards
// clicks to the panel over D-Bus, and moves the panel window under the icon,
// because Wayland apps cannot position their own windows. The tool icons
// (shelf, ...) live in tools.js.
//
// Clipboard: Mutter offers no clipboard-manager protocol, so only code
// inside the Shell sees copies made in other apps. This file forwards
// copied text to the daemon (which stores it) and sets the clipboard when
// the daemon asks. It keeps no history itself.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import * as Switcher from './switcher.js';
import {ToolIcons} from './tools.js';

const PANEL_NAME = 'io.github.incept0s.Andromeda.Panel';
const PANEL_PATH = '/io/github/incept0s/Andromeda/Panel';
const PANEL_IFACE = 'io.github.incept0s.Andromeda.Panel1';
// Must match the main window title in crates/ui/tauri.conf.json. The
// Settings window ("Andromeda Settings") is left where Mutter puts it.
const PANEL_TITLE = 'Andromeda';
const DAEMON_NAME = 'io.github.incept0s.Andromeda';
const DAEMON_PATH = '/io/github/incept0s/Andromeda';
const DAEMON_IFACE = 'io.github.incept0s.Andromeda1';
// Password managers mark secrets with this type; they never enter history.
const SECRET_HINT = 'x-kde-passwordManagerHint';
// GNOME's own items at the right end of the bar (quick settings holds the
// network, VPN, sound and battery icons). Andromeda's icons go just before.
const SYSTEM_ROLES = ['screenRecording', 'screenSharing', 'dwellClick', 'a11y', 'keyboard', 'quickSettings'];

function firstSystemItem(box) {
    const system = SYSTEM_ROLES.map(r => Main.panel.statusArea[r]?.container).filter(Boolean);
    return box.get_children().find(c => system.includes(c)) ?? null;
}

/** Moves `actor` into `box` right before `sibling`, or to the end when null. */
function placeBefore(box, actor, sibling) {
    if (actor.get_parent() !== box) {
        actor.get_parent()?.remove_child(actor);
        if (sibling)
            box.insert_child_below(actor, sibling);
        else
            box.add_child(actor);
    } else if (sibling) {
        box.set_child_below_sibling(actor, sibling);
    } else {
        box.set_child_above_sibling(actor, null);
    }
}

/** Moves `actor` to the start of `box`. */
function placeFirst(box, actor) {
    if (actor.get_parent() !== box) {
        actor.get_parent()?.remove_child(actor);
        box.insert_child_at_index(actor, 0);
    } else {
        box.set_child_below_sibling(actor, null);
    }
}

export default class AndromedaExtension extends Extension {
    enable() {
        this._button = new PanelMenu.Button(0.0, this.metadata.name, true);
        const box = new St.BoxLayout({style_class: 'panel-status-menu-box'});
        box.add_child(new St.Icon({
            gicon: Gio.icon_new_for_string(`${this.path}/icons/andromeda-symbolic.svg`),
            style_class: 'system-status-icon',
        }));
        this._button.add_child(box);
        this._button.connect('button-press-event', () => {
            this._toggle();
            return Clutter.EVENT_STOP;
        });
        Main.panel.addToStatusArea(this.uuid, this._button);

        // Plain numbers formatted by the daemon (e.g. "CPU 42°  RAM 61%").
        // The daemon publishes at most one change per second; this only
        // sets the text. Their own top-bar item, so they can sit on either
        // side of the bar (Settings → Top bar). Hidden while empty; a click
        // opens the panel like the icon does.
        this._vitals = new PanelMenu.Button(0.0, `${this.metadata.name} numbers`, true);
        this._label = new St.Label({
            style_class: 'andromeda-label',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._vitals.add_child(this._label);
        this._vitals.visible = false;
        this._vitals.connect('button-press-event', () => {
            this._toggle();
            return Clutter.EVENT_STOP;
        });
        Main.panel.addToStatusArea(`${this.uuid}-numbers`, this._vitals);

        this._tools = new ToolIcons(this.uuid);

        this._side = 'Right';
        this._layout();
        // Other extensions (tray icons) add items later; put ours back in place.
        this._rightAddedId = Main.panel._rightBox.connect('child-added', () => this._queueLayout());

        // Super+A opens the panel. GNOME uses it for "Show apps", so it is
        // taken from there while Andromeda is enabled and given back after.
        this._settings = this.getSettings();
        this._freeShortcuts();
        Main.wm.addKeybinding('toggle-panel', this._settings, Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW, () => this._toggle());

        // Hiding a GTK window on Wayland destroys its surface, so every
        // show creates a new MetaWindow and we place it again.
        this._windowCreatedId = global.display.connect('window-created',
            (_display, win) => this._onWindowCreated(win));

        this._selection = global.display.get_selection();
        this._ownerChangedId = this._selection.connect('owner-changed',
            (sel, type) => this._onClipboardChanged(sel, type));
        this._setClipboardId = Gio.DBus.session.signal_subscribe(DAEMON_NAME, DAEMON_IFACE,
            'SetClipboard', DAEMON_PATH, null, Gio.DBusSignalFlags.NONE,
            (_c, _s, _p, _i, _n, params) => this._setClipboard(...params.deep_unpack()));

        this._propsId = Gio.DBus.session.signal_subscribe(DAEMON_NAME, 'org.freedesktop.DBus.Properties',
            'PropertiesChanged', DAEMON_PATH, DAEMON_IFACE, Gio.DBusSignalFlags.NONE,
            (_c, _s, _p, _i, _n, params) => {
                const changed = params.deep_unpack()[1];
                if ('TopBar' in changed)
                    this._setText(changed.TopBar.deep_unpack());
                if ('Config' in changed)
                    this._applyConfig(changed.Config.deep_unpack());
                if ('Shelf' in changed)
                    this._tools?.setShelf(changed.Shelf.deep_unpack());
            });
        // Fetch the current text when the daemon appears; clear it when it goes.
        // Without the daemon, Alt+Tab stays GNOME's own (fail safe).
        this._watchId = Gio.bus_watch_name(Gio.BusType.SESSION, DAEMON_NAME, Gio.BusNameWatcherFlags.NONE,
            () => this._fetchState(), () => {
                this._setText('');
                this._setSwitcher(false);
                this._tools?.applyConfig(null);
            });
    }

    disable() {
        Main.wm.removeKeybinding('toggle-panel');
        this._restoreShortcuts();
        this._settings = null;
        global.display.disconnect(this._windowCreatedId);
        this._windowCreatedId = 0;
        this._selection.disconnect(this._ownerChangedId);
        this._selection = null;
        Gio.DBus.session.signal_unsubscribe(this._setClipboardId);
        Gio.DBus.session.signal_unsubscribe(this._propsId);
        Gio.bus_unwatch_name(this._watchId);
        this._setSwitcher(false);
        Main.panel._rightBox.disconnect(this._rightAddedId);
        if (this._layoutId)
            GLib.source_remove(this._layoutId);
        this._layoutId = 0;
        this._label = null;
        this._tools.destroy();
        this._tools = null;
        this._vitals.destroy();
        this._vitals = null;
        this._button.destroy();
        this._button = null;
    }

    /**
     * Top-bar order, left to right in the right half of the bar:
     *   [numbers] [open apps / tray icons] [tool icons] [Andromeda] [system icons]
     * With topbar_side = Left the numbers move to the end of the left box
     * (after Activities or an app menu) instead.
     */
    _layout() {
        if (!this._button)
            return;
        const right = Main.panel._rightBox;
        for (const c of [...this._tools.containers, this._button.container])
            placeBefore(right, c, firstSystemItem(right));
        if (this._side === 'Left')
            placeBefore(Main.panel._leftBox, this._vitals.container, null);
        else
            placeFirst(right, this._vitals.container);
    }

    _queueLayout() {
        if (this._layoutId)
            return;
        this._layoutId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._layoutId = 0;
            this._layout();
            return GLib.SOURCE_REMOVE;
        });
    }

    _freeShortcuts() {
        const shell = new Gio.Settings({schema_id: 'org.gnome.shell.keybindings'});
        const ours = this._settings.get_strv('toggle-panel');
        const apps = shell.get_strv('toggle-application-view');
        const taken = apps.filter(a => ours.includes(a));
        if (taken.length === 0)
            return;
        this._settings.set_strv('freed-shortcuts', taken);
        shell.set_strv('toggle-application-view', apps.filter(a => !taken.includes(a)));
    }

    _restoreShortcuts() {
        const freed = this._settings.get_strv('freed-shortcuts');
        if (freed.length === 0)
            return;
        const shell = new Gio.Settings({schema_id: 'org.gnome.shell.keybindings'});
        const apps = shell.get_strv('toggle-application-view');
        shell.set_strv('toggle-application-view', [...apps, ...freed.filter(a => !apps.includes(a))]);
        this._settings.set_strv('freed-shortcuts', []);
    }

    _toggle() {
        // D-Bus activation starts the panel app if it is not running.
        Gio.DBus.session.call(PANEL_NAME, PANEL_PATH, PANEL_IFACE, 'Toggle',
            null, null, Gio.DBusCallFlags.NONE, -1, null, (conn, res) => {
                try {
                    conn.call_finish(res);
                } catch (e) {
                    console.warn(`Andromeda: Toggle failed: ${e.message}`);
                }
            });
    }

    _setText(text) {
        if (!this._label)
            return;
        this._label.text = text;
        this._vitals.visible = text.length > 0;
    }

    _fetchState() {
        const get = (prop, apply) => Gio.DBus.session.call(DAEMON_NAME, DAEMON_PATH,
            'org.freedesktop.DBus.Properties', 'Get', new GLib.Variant('(ss)', [DAEMON_IFACE, prop]),
            null, Gio.DBusCallFlags.NO_AUTO_START, -1, null, (conn, res) => {
                try {
                    apply(conn.call_finish(res).deep_unpack()[0].deep_unpack());
                } catch {
                    // Daemon went away in between; the name watcher handles it.
                }
            });
        get('TopBar', t => this._setText(t));
        get('Config', c => this._applyConfig(c));
        get('Shelf', s => this._tools?.setShelf(s));
    }

    /** Read from the settings JSON: the switcher switch, the numbers' side, the tool icons. */
    _applyConfig(json) {
        let c = {};
        try {
            c = JSON.parse(json);
        } catch {
            // Keep the defaults below.
        }
        this._setSwitcher(c.features?.switcher === true);
        this._side = c.topbar_side === 'Left' ? 'Left' : 'Right';
        this._tools.applyConfig(c);
        this._layout();
    }

    _setSwitcher(on) {
        if (on === Boolean(this._switcherOn))
            return;
        this._switcherOn = on;
        if (on)
            Switcher.enable();
        else
            Switcher.disable();
    }

    _onClipboardChanged(sel, type) {
        if (type !== Meta.SelectionType.SELECTION_CLIPBOARD)
            return;
        if (sel.get_mimetypes(type).includes(SECRET_HINT))
            return;
        St.Clipboard.get_default().get_text(St.ClipboardType.CLIPBOARD, (_clip, text) => {
            if (!text)
                return;
            // Fire and forget: if the daemon is not running, nothing is kept.
            Gio.DBus.session.call(DAEMON_NAME, DAEMON_PATH, DAEMON_IFACE, 'ClipboardAdd',
                new GLib.Variant('(s)', [text]), null, Gio.DBusCallFlags.NO_AUTO_START, -1, null, null);
        });
    }

    _setClipboard(text, mime) {
        const clip = St.Clipboard.get_default();
        if (mime.startsWith('text/plain'))
            clip.set_text(St.ClipboardType.CLIPBOARD, text);
        else
            clip.set_content(St.ClipboardType.CLIPBOARD, mime, new GLib.Bytes(new TextEncoder().encode(text)));
    }

    _isAndromeda(win) {
        const id = `${win.get_gtk_application_id() ?? ''} ${win.get_wm_class() ?? ''}`;
        return id.toLowerCase().includes('andromeda');
    }

    _onWindowCreated(win) {
        // On Wayland the app id, title, size and even the compositor actor
        // arrive after window-created, so decide when the window is first
        // shown. One short-lived handler per new window; it disconnects itself.
        const id = win.connect('shown', () => {
            win.disconnect(id);
            if (!this._button || !this._isAndromeda(win))
                return;
            const title = win.get_title();
            // A tool window goes under its icon, or under ours if that is hidden.
            const tool = this._tools.buttonForTitle(title);
            if (title === PANEL_TITLE)
                this._place(win, this._button);
            else if (tool)
                this._place(win, tool.visible ? tool : this._button);
        });
    }

    /** Under `button`, centred, clamped to the screen. */
    _place(win, button) {
        const [bx, by] = button.get_transformed_position();
        const [bw, bh] = button.get_transformed_size();
        const frame = win.get_frame_rect();
        const work = Main.layoutManager.getWorkAreaForMonitor(Main.layoutManager.primaryIndex);
        // Centre under the icon, clamped to the screen.
        const x = Math.max(work.x,
            Math.min(Math.round(bx + bw / 2 - frame.width / 2), work.x + work.width - frame.width));
        const y = Math.round(by + bh);
        win.move_frame(true, x, y);
        win.make_above();
    }
}
