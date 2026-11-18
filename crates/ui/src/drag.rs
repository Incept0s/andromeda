// SPDX-License-Identifier: GPL-3.0-or-later
//! Dragging files out of our windows (shelf items) as a real file drag.
//!
//! Letting WebKit do it (an HTML drag with `text/uri-list` in its
//! DataTransfer) did not work reliably: most drops on Files ended with GTK
//! reporting "failed: Error", i.e. the target refused what was offered.
//! So the page cancels WebKit's drag and asks for this one instead: a GTK
//! drag started on our window with exactly two formats, a file list
//! (text/uri-list: Files, browsers, chat apps) and the plain paths (text
//! fields, terminals). Dropping copies; the original stays where it is.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use gtk::glib::Propagation;
use gtk::prelude::*;
use tauri::Manager;

/// Target ids in the list below; GTK passes them back in drag-data-get.
const URIS: u32 = 0;
const TEXT: u32 = 1;

/// Starts dragging `paths` out of the calling window. Must run on the GTK
/// main thread, which is where Tauri runs non-async commands. Called from
/// the page's dragstart, while the mouse button is still down: Wayland only
/// starts a drag during a press.
#[tauri::command]
pub fn start_file_drag(window: tauri::WebviewWindow, paths: Vec<String>) -> Result<(), String> {
    let paths: Vec<String> = paths
        .into_iter()
        .filter(|p| Path::new(p).is_absolute() && Path::new(p).exists())
        .collect();
    let uris: Vec<String> = paths
        .iter()
        .filter_map(|p| gtk::glib::filename_to_uri(p, None).ok())
        .map(|u| u.to_string())
        .collect();
    if uris.is_empty() {
        return Err("Nothing to drag: the file is gone.".into());
    }
    let icon = match paths.as_slice() {
        [one] if Path::new(one).is_dir() => "folder",
        [_] => "text-x-generic",
        _ => "edit-copy",
    };
    let gtk_window = window.gtk_window().map_err(|e| e.to_string())?;
    let (app, label) = (window.app_handle().clone(), window.label().to_owned());

    let targets = gtk::TargetList::new(&[]);
    targets.add_uri_targets(URIS);
    targets.add_text_targets(TEXT);

    // Signal handlers for this one drag; all removed when it ends.
    let handlers = Rc::new(RefCell::new(Vec::new()));
    let text = paths.join("\n");
    handlers.borrow_mut().push(
        gtk_window.connect_drag_data_get(move |_, _, data, info, _| {
            if info == URIS {
                let refs: Vec<&str> = uris.iter().map(String::as_str).collect();
                data.set_uris(&refs);
            } else if info == TEXT {
                data.set_text(&text);
            }
        }),
    );
    let label2 = label.clone();
    handlers
        .borrow_mut()
        .push(gtk_window.connect_drag_failed(move |_, _, result| {
            eprintln!("andromeda: file drag out of {label2} failed: {result:?}");
            Propagation::Proceed
        }));
    let (done, win) = (handlers.clone(), gtk_window.clone());
    let (app2, label3) = (app.clone(), label.clone());
    handlers
        .borrow_mut()
        .push(gtk_window.connect_drag_end(move |_, _| {
            eprintln!("andromeda: file drag out of {label3} ended");
            for id in done.borrow_mut().drain(..) {
                win.disconnect(id);
            }
            crate::drag_finished(&app2, &label3);
        }));

    // Keep the window open while the drag leaves it (see watch_drags).
    crate::set_dragging(&app, &label, true);
    eprintln!(
        "andromeda: file drag out of {label} started ({} file(s))",
        paths.len()
    );
    match gtk_window.drag_begin_with_coordinates(
        &targets,
        gtk::gdk::DragAction::COPY,
        1,
        None,
        -1,
        -1,
    ) {
        Some(ctx) => {
            ctx.drag_set_icon_name(icon, 0, 0);
            Ok(())
        }
        None => {
            for id in handlers.borrow_mut().drain(..) {
                gtk_window.disconnect(id);
            }
            crate::set_dragging(&app, &label, false);
            eprintln!("andromeda: file drag out of {label} could not start");
            Err("The drag could not start.".into())
        }
    }
}
