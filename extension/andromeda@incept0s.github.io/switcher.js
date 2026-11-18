// SPDX-License-Identifier: GPL-3.0-or-later
//
// Builds on GNOME Shell's own window switcher (js/ui/altTab.js, GNOME Shell
// contributors, GPL-2.0-or-later). `windowClone` is adapted from its
// `_createWindowClone`. Modified: larger previews, windows grouped by app,
// Alt+Tab key handling, the window title under every preview.

// Andromeda's Alt+Tab: one large live preview per window (minimized ones
// included), windows of the same app next to each other, the app icon on
// each preview and the window title under it (GNOME shows only the selected
// window's title, which makes similar windows hard to tell apart). Kept
// small on purpose: everything else (keyboard, mouse,
// closing with W, scrolling a long list) is GNOME's tested code.

import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as AltTab from 'resource:///org/gnome/shell/ui/altTab.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const PREVIEW_SIZE = 224;
const APP_ICON_SIZE = 48;
const BINDINGS = ['switch-applications', 'switch-applications-backward'];
// Andromeda's popup windows (panel and tool windows) never show in Alt+Tab;
// Tauri cannot mark them skip-taskbar on Wayland. The Settings window does.
const POPUP_TITLES = ['Andromeda', 'Andromeda Shelf', 'Andromeda Converter', 'Andromeda Clipboard'];

function windowClone(actor, size) {
    const [width, height] = actor.get_size();
    const scale = Math.min(1.0, size / width, size / height);
    return new Clutter.Clone({
        source: actor,
        width: width * scale,
        height: height * scale,
        x_align: Clutter.ActorAlign.CENTER,
        y_align: Clutter.ActorAlign.CENTER,
        x_expand: true,
        y_expand: true,
    });
}

// GObject subclasses of GNOME classes must be registered.
export const SwitcherPopup = GObject.registerClass(
class AndromedaSwitcherPopup extends AltTab.WindowSwitcherPopup {
    _init() {
        super._init();
        const size = PREVIEW_SIZE * St.ThemeContext.get_for_stage(global.stage).scale_factor;
        // Swap GNOME's 128 px previews for larger ones, keeping the app
        // icon overlay. WindowIcon builds them in its constructor, so this
        // replaces its children.
        for (const icon of this._items) {
            const actor = icon.window.get_compositor_private();
            if (!actor)
                continue;
            icon._icon.destroy_all_children();
            icon._icon.add_child(windowClone(actor, size));
            if (icon.app)
                icon._icon.add_child(icon._createAppIcon(icon.app, APP_ICON_SIZE));
            icon._icon.set_size(size, size);
            // The title under the preview, one line, cut with "…".
            const title = new St.Label({
                text: icon.window.get_title() ?? '',
                style_class: 'andromeda-switcher-title',
                x_align: Clutter.ActorAlign.CENTER,
            });
            title.clutter_text.ellipsize = Pango.EllipsizeMode.END;
            title.set_width(size);
            icon.add_child(title);
        }
    }

    // Groups windows by app while keeping quick Alt+Tab useful: the first
    // entry stays the current window and the second the most recent window
    // of the previous app. The current app's other windows go last.
    _getWindowList() {
        const windows = super._getWindowList().filter(w =>
            !(w.get_wm_class() === 'andromeda' && POPUP_TITLES.includes(w.get_title())));
        if (windows.length < 3)
            return windows;
        const tracker = Shell.WindowTracker.get_default();
        const key = w => tracker.get_window_app(w)?.get_id() ?? `window-${w.get_stable_sequence()}`;
        const groups = new Map();
        for (const w of windows) {
            const k = key(w);
            if (!groups.has(k))
                groups.set(k, []);
            groups.get(k).push(w);
        }
        const [current, ...rest] = groups.get(key(windows[0]));
        const others = [...groups.entries()].filter(([k]) => k !== key(windows[0])).flatMap(([, ws]) => ws);
        return [current, ...others, ...rest];
    }

    // The base class only reacts to the switch-windows binding.
    _keyPressHandler(keysym, action) {
        const A = Meta.KeyBindingAction;
        if (action === A.SWITCH_APPLICATIONS) {
            this._select(this._next());
            return Clutter.EVENT_STOP;
        }
        if (action === A.SWITCH_APPLICATIONS_BACKWARD) {
            this._select(this._previous());
            return Clutter.EVENT_STOP;
        }
        return super._keyPressHandler(keysym, action);
    }
});

function start(display, window, event, binding) {
    try {
        const popup = new SwitcherPopup();
        if (!popup.show(binding.is_reversed(), binding.get_name(), binding.get_mask()))
            popup.destroy();
    } catch (e) {
        // Never leave the user without Alt+Tab: fall back to GNOME's own.
        console.error(`Andromeda switcher failed, using GNOME's: ${e}`);
        Main.wm._startSwitcher(display, window, event, binding);
    }
}

export function enable() {
    for (const name of BINDINGS)
        Main.wm.setCustomKeybindingHandler(name, Shell.ActionMode.NORMAL, start);
}

/** Gives Alt+Tab back to GNOME, exactly as GNOME registers it. */
export function disable() {
    for (const name of BINDINGS)
        Main.wm.setCustomKeybindingHandler(name, Shell.ActionMode.NORMAL, Main.wm._startSwitcher.bind(Main.wm));
}
