// SPDX-License-Identifier: GPL-3.0-or-later
//
// Tool icons in the top bar (shelf, converter, clipboard), between the open apps and the
// Andromeda icon. Deliberately thin, like the rest of the extension:
//
// * A click asks the panel app (over D-Bus) to show or hide the tool's
//   small window; the extension then moves that window under the icon.
// * Files dragged from any app onto an icon are handed on: to the daemon
//   (shelf) or to the panel app, which opens the converter with the file.
//
// Dropping on the top bar: on Wayland a drag is owned by the compositor.
// Mutter reports where a drag is (Meta.Dnd "dnd-position-change", the
// same signal GNOME uses to open windows when you drag files over them in
// the overview) and offers the dragged data as the DND selection. So while
// a drag hovers one of our icons we read its text/uri-list, and when the
// drag ends there ("dnd-leave") the files are passed on. Nothing is moved
// or copied: the app the drag came from sees a cancelled drop.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';

const PANEL_NAME = 'io.github.incept0s.Andromeda.Panel';
const PANEL_PATH = '/io/github/incept0s/Andromeda/Panel';
const PANEL_IFACE = 'io.github.incept0s.Andromeda.Panel1';
const DAEMON_NAME = 'io.github.incept0s.Andromeda';
const DAEMON_PATH = '/io/github/incept0s/Andromeda';
const DAEMON_IFACE = 'io.github.incept0s.Andromeda1';

/**
 * One entry per tool, in top-bar order (left to right).
 * name: Panel1.ToggleTool argument and config key under `icons`.
 * title: the tool window's title; must match TOOLS in crates/ui/src/main.rs.
 * drop: where dropped files go: [bus name, path, interface, method, signature].
 *   The shelf takes every path, the converter the first one.
 */
const TOOLS = [
    {
        name: 'shelf', label: 'Shelf', icon: 'package-x-generic-symbolic', title: 'Andromeda Shelf',
        drop: [DAEMON_NAME, DAEMON_PATH, DAEMON_IFACE, 'ShelfAdd', '(as)'],
    },
    {
        name: 'convert', label: 'Converter', icon: 'media-playlist-repeat-symbolic', title: 'Andromeda Converter',
        drop: [PANEL_NAME, PANEL_PATH, PANEL_IFACE, 'ConvertFile', '(s)'],
    },
    {
        name: 'clipboard', label: 'Clipboard', icon: 'edit-paste-symbolic', title: 'Andromeda Clipboard',
        drop: null,
    },
];

/** Local paths from a text/uri-list ("#" lines are comments). */
function parseUriList(text) {
    return text.split(/\r?\n/)
        .map(l => l.trim())
        .filter(l => l && !l.startsWith('#'))
        .map(uri => {
            try {
                return Gio.File.new_for_uri(uri).get_path();
            } catch {
                return null;
            }
        })
        .filter(Boolean);
}

export class ToolIcons {
    constructor(uuid) {
        this._tools = TOOLS.map(t => {
            const button = new PanelMenu.Button(0.0, `Andromeda ${t.label}`, true);
            const box = new St.BoxLayout({style_class: 'panel-status-menu-box'});
            box.add_child(new St.Icon({icon_name: t.icon, style_class: 'system-status-icon'}));
            // Item count (the shelf); hidden while empty.
            const count = new St.Label({
                style_class: 'andromeda-count',
                y_align: Clutter.ActorAlign.CENTER,
                visible: false,
            });
            box.add_child(count);
            button.add_child(box);
            button.connect('button-press-event', () => {
                this._toggle(t.name);
                return Clutter.EVENT_STOP;
            });
            // Shown once the daemon's settings say so.
            button.visible = false;
            // Destroyed with the bar at shell shutdown: forget it.
            button.connect('destroy', () => {
                this._tools = this._tools.filter(x => x.button !== button);
            });
            Main.panel.addToStatusArea(`${uuid}-${t.name}`, button);
            return {...t, button, count};
        });

        const dnd = global.backend.get_dnd();
        this._dndIds = [
            dnd.connect('dnd-enter', () => this._dragBegin()),
            dnd.connect('dnd-position-change', (_dnd, x, y) => this._dragMotion(x, y)),
            dnd.connect('dnd-leave', () => this._dragEnd()),
        ];
        this._drag = null;
    }

    destroy() {
        const dnd = global.backend.get_dnd();
        this._dndIds.forEach(id => dnd.disconnect(id));
        this._dndIds = [];
        this._drag = null;
        this._tools.forEach(t => t.button.destroy());
        this._tools = [];
    }

    /** Status-area containers in top-bar order, for the extension's layout. */
    get containers() {
        return this._tools.map(t => t.button.container);
    }

    /** The button a tool window belongs to, by window title. */
    buttonForTitle(title) {
        return this._tools.find(t => t.title === title)?.button ?? null;
    }

    /** Visibility from the settings JSON; `null` (no daemon) hides all. */
    applyConfig(c) {
        for (const t of this._tools) {
            t.button.visible = c !== null &&
                c.features?.tools !== false &&
                c.icons?.[t.name] === true;
        }
    }

    /** The daemon's Shelf property (JSON list): shows the item count. */
    setShelf(json) {
        let n = 0;
        try {
            n = JSON.parse(json).length ?? 0;
        } catch {
            // Keep 0.
        }
        const t = this._tools.find(x => x.name === 'shelf');
        if (t) {
            t.count.text = n > 0 ? `${n}` : '';
            t.count.visible = n > 0;
        }
    }

    _toggle(name) {
        // D-Bus activation starts the panel app if it is not running.
        Gio.DBus.session.call(PANEL_NAME, PANEL_PATH, PANEL_IFACE, 'ToggleTool',
            new GLib.Variant('(s)', [name]), null, Gio.DBusCallFlags.NONE, -1, null, (conn, res) => {
                try {
                    conn.call_finish(res);
                } catch (e) {
                    console.warn(`Andromeda: ToggleTool ${name} failed: ${e.message}`);
                }
            });
    }

    _dragBegin() {
        this._drag = {target: null, paths: null, reading: false, deliver: null};
    }

    _dragMotion(x, y) {
        if (!this._drag)
            this._dragBegin();
        const actor = global.stage.get_actor_at_pos(Clutter.PickMode.REACTIVE, x, y);
        const tool = actor
            ? this._tools.find(t => t.drop && t.button.visible && t.button.contains(actor)) ?? null
            : null;
        if (tool !== this._drag.target) {
            this._drag.target?.button.remove_style_class_name('andromeda-drop');
            tool?.button.add_style_class_name('andromeda-drop');
            this._drag.target = tool;
        }
        if (tool && !this._drag.reading)
            this._readDraggedFiles(this._drag);
    }

    _dragEnd() {
        const drag = this._drag;
        this._drag = null;
        if (!drag?.target)
            return;
        const tool = drag.target;
        tool.button.remove_style_class_name('andromeda-drop');
        const deliver = paths => {
            if (paths.length > 0)
                this._dropOn(tool, paths);
        };
        if (drag.paths)
            deliver(drag.paths);
        else
            drag.deliver = deliver; // Still reading; hand over when done.
    }

    /** Reads the dragged files once per drag, as soon as it hovers an icon. */
    _readDraggedFiles(drag) {
        drag.reading = true;
        const selection = global.display.get_selection();
        const type = Meta.SelectionType.SELECTION_DND;
        if (!selection.get_mimetypes(type).includes('text/uri-list')) {
            drag.paths = [];
            return;
        }
        const out = Gio.MemoryOutputStream.new_resizable();
        selection.transfer_async(type, 'text/uri-list', -1, out, null, (sel, res) => {
            let paths = [];
            try {
                sel.transfer_finish(res);
                out.close(null);
                paths = parseUriList(new TextDecoder().decode(out.steal_as_bytes().toArray()));
            } catch (e) {
                console.warn(`Andromeda: could not read the dragged files: ${e.message}`);
            }
            drag.paths = paths;
            drag.deliver?.(paths);
        });
    }

    _dropOn(tool, paths) {
        const [name, path, iface, method, signature] = tool.drop;
        const args = signature === '(as)' ? [paths] : [paths[0]];
        // D-Bus activation starts the service or panel app if needed.
        Gio.DBus.session.call(name, path, iface, method, new GLib.Variant(signature, args),
            null, Gio.DBusCallFlags.NONE, -1, null, (conn, res) => {
                try {
                    conn.call_finish(res);
                } catch (e) {
                    console.warn(`Andromeda: ${method} failed: ${e.message}`);
                }
            });
    }
}
