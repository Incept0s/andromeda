// SPDX-License-Identifier: GPL-3.0-or-later
//
// Andromeda top-bar button. Deliberately thin: it draws an icon, forwards
// clicks to the panel over D-Bus, and moves the panel window under the icon,
// because Wayland apps cannot position their own windows. No data
// collection happens here.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const PANEL_NAME = 'io.github.incept0s.Andromeda.Panel';
const PANEL_PATH = '/io/github/incept0s/Andromeda/Panel';
const PANEL_IFACE = 'io.github.incept0s.Andromeda.Panel1';
// Must match the main window title in crates/ui/tauri.conf.json. The
// Settings window ("Andromeda Settings") is left where Mutter puts it.
const PANEL_TITLE = 'Andromeda';

export default class AndromedaExtension extends Extension {
    enable() {
        this._button = new PanelMenu.Button(0.0, this.metadata.name, true);
        this._button.add_child(new St.Icon({
            gicon: Gio.icon_new_for_string(`${this.path}/icons/andromeda-symbolic.svg`),
            style_class: 'system-status-icon',
        }));
        this._button.connect('button-press-event', () => {
            this._toggle();
            return Clutter.EVENT_STOP;
        });
        Main.panel.addToStatusArea(this.uuid, this._button);

        // Hiding a GTK window on Wayland destroys its surface, so every
        // show creates a new MetaWindow and we place it again.
        this._windowCreatedId = global.display.connect('window-created',
            (_display, win) => this._onWindowCreated(win));
    }

    disable() {
        global.display.disconnect(this._windowCreatedId);
        this._windowCreatedId = 0;
        this._button.destroy();
        this._button = null;
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
            // Phase 1 diagnostics: confirms which ids Tauri's window reports.
            console.log(`Andromeda: window wm_class=${win.get_wm_class()} ` +
                `app_id=${win.get_gtk_application_id()} title=${win.get_title()}`);
            if (win.get_title() === PANEL_TITLE)
                this._place(win);
        });
    }

    _place(win) {
        const [bx, by] = this._button.get_transformed_position();
        const [bw, bh] = this._button.get_transformed_size();
        const frame = win.get_frame_rect();
        const work = Main.layoutManager.getWorkAreaForMonitor(Main.layoutManager.primaryIndex);
        // Centre under the icon, clamped to the screen.
        const x = Math.max(work.x,
            Math.min(Math.round(bx + bw / 2 - frame.width / 2), work.x + work.width - frame.width));
        const y = Math.round(by + bh);
        win.move_frame(true, x, y);
        win.make_above();
        const now = win.get_frame_rect();
        console.log(`Andromeda: placed panel, asked ${x},${y}, now at ${now.x},${now.y} size ${now.width}x${now.height}`);
    }
}
