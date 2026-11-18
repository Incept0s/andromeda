// SPDX-License-Identifier: GPL-3.0-or-later
//! Tools tab: a list of tools like the original's Utilities; each opens a
//! sub-view with a back button. Files dropped on the panel go to the
//! converter when it is open, otherwise to the shelf.
//!
//! Also the small tool windows opened from the top-bar icons
//! (`ShelfWindow`), which reuse the same pieces.

use andromeda_core::config::Config;
use andromeda_core::metrics::format_bytes;
use andromeda_core::tools::{CleanItem, ClipEntry, ConvertOption, Package, PkgSource, ShelfItem};
use leptos::prelude::*;
use serde::Serialize;

use crate::Icon;
use crate::tauri;

#[derive(Clone, Copy, PartialEq)]
enum View {
    List,
    Clipboard,
    Convert,
    Cleaner,
    Packages,
    Shelf,
}

#[derive(Serialize)]
struct Query {
    query: String,
}
#[derive(Serialize)]
struct Id {
    id: u64,
}
#[derive(Serialize)]
struct Pin {
    id: u64,
    pinned: bool,
}
#[derive(Serialize)]
struct PathArg {
    path: String,
}
#[derive(Serialize)]
struct Paths {
    paths: Vec<String>,
}
#[derive(Serialize)]
struct ConvertArg {
    path: String,
    target: String,
}
#[derive(Serialize)]
struct Ids {
    ids: Vec<u32>,
}
#[derive(Serialize)]
struct Search {
    source: &'static str,
    query: String,
}
#[derive(Serialize)]
struct Action {
    source: &'static str,
    action: &'static str,
    id: String,
}

#[derive(Serialize)]
struct Choose {
    title: &'static str,
    multiple: bool,
}

fn now_secs() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// "23 h left", "40 min left"; empty when files never expire.
fn time_left(added: i64, keep_hours: Option<u32>) -> String {
    let Some(h) = keep_hours else {
        return String::new();
    };
    let left = (added + i64::from(h) * 3600 - now_secs()).max(0);
    if left >= 3600 {
        format!("{} h left", left / 3600)
    } else {
        format!("{} min left", (left / 60).max(1))
    }
}

fn file_name(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_owned()
}

fn parent(p: &str) -> String {
    p.rsplit_once('/').map_or("/".into(), |(d, _)| {
        if d.is_empty() {
            "/".into()
        } else {
            d.to_owned()
        }
    })
}

#[component]
fn Back(view: RwSignal<View>, title: &'static str) -> impl IntoView {
    view! {
        <div class="row back">
            <button class="iconbtn" title="Back" on:click=move |_| view.set(View::List)>"‹"</button>
            <strong>{title}</strong>
        </div>
    }
}

#[component]
fn ToolCard(
    icon: &'static str,
    title: &'static str,
    desc: &'static str,
    on_click: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <button class="toolcard" on:click=move |_| on_click()>
            <Icon name=icon />
            <span class="grow">
                <strong>{title}</strong>
                <span class="muted small">{desc}</span>
            </span>
            <span class="muted">"›"</span>
        </button>
    }
}

/// Clipboard history, newest first (pinned on top), with a search box.
/// `on_copy` runs after an entry was put back on the clipboard.
#[component]
fn ClipList(#[prop(optional)] on_copy: Option<Callback<()>>) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let items = RwSignal::new(Vec::<ClipEntry>::new());
    let copied = RwSignal::new(None::<u64>);
    let refresh = move || {
        tauri::spawn(async move {
            if let Ok(v) = tauri::invoke(
                "clipboard_list",
                &Query {
                    query: query.get_untracked(),
                },
            )
            .await
            {
                items.set(v);
            }
        });
    };
    refresh();
    // No polling: the service announces every change, and a tool window
    // (loaded while hidden) is told when it is shown again.
    tauri::listen::<()>("clipboard", move |_| refresh());
    tauri::listen::<()>("shown", move |_| refresh());
    view! {
        // The search line appears once there is something to search.
        <Show when=move || !items.with(Vec::is_empty) || !query.with(String::is_empty)>
            <input class="search wide" type="search" placeholder="Search copied text"
                prop:value=move || query.get()
                on:input=move |e| { query.set(event_target_value(&e)); refresh(); } />
        </Show>
        <Show when=move || !items.with(Vec::is_empty)
            fallback=|| view! { <p class="muted small">"Copied text and images show up here. They stay on this computer."</p> }>
            <For each=move || items.get() key=|e| (e.id, e.pinned, e.time) let:e>
                <div class="clip" class:pinned=e.pinned>
                    <button class="cliptext" title="Copy" on:click=move |_| {
                        tauri::call("clipboard_copy", Id { id: e.id });
                        copied.set(Some(e.id));
                        if let Some(f) = on_copy { f.run(()) }
                    }>
                        {if e.thumb.is_empty() {
                            view! { <span>{e.text.chars().take(240).collect::<String>()}</span> }.into_any()
                        } else {
                            view! {
                                <span class="clipimage">
                                    <img src=e.thumb.clone() alt="" />
                                    <span class="muted small">{e.text.clone()}</span>
                                </span>
                            }.into_any()
                        }}
                    </button>
                    <div class="clipactions">
                        <span class="small accent-text" class:hidden=move || copied.get() != Some(e.id)>"Copied"</span>
                        <button class="iconbtn" title=if e.pinned { "Unpin" } else { "Pin (never removed automatically)" }
                            on:click=move |_| { tauri::call("clipboard_pin", Pin { id: e.id, pinned: !e.pinned }); refresh(); }>
                            {if e.pinned { "★" } else { "☆" }}
                        </button>
                        <button class="iconbtn" title="Delete" on:click=move |_| { tauri::call("clipboard_delete", Id { id: e.id }); refresh(); }>"✕"</button>
                    </div>
                </div>
            </For>
            <button class="btn small-btn" on:click=move |_| { tauri::call("clipboard_clear", ()); refresh(); }>"Clear history (keeps pinned)"</button>
        </Show>
    }
}

#[component]
fn Clipboard(view: RwSignal<View>) -> impl IntoView {
    view! {
        <Back view title="Clipboard" />
        <ClipList />
    }
}

/// The small window under the clipboard icon in the top bar: click an
/// entry and it becomes the clipboard; the window closes so you can paste.
#[component]
pub fn ClipboardWindow() -> impl IntoView {
    view! {
        <main class="glass tool">
            <header class="tool-head">
                <Icon name="clipboard" />
                <strong class="grow">"Clipboard"</strong>
            </header>
            <section class="content">
                <ClipList on_copy=Callback::new(|_| tauri::call("hide", ())) />
            </section>
            <footer class="muted small tool-foot">"Click one to copy it, then paste where you like."</footer>
        </main>
    }
}

/// Opens GNOME's file chooser for one file to convert.
fn choose_for_convert(file: RwSignal<Option<String>>) {
    tauri::spawn(async move {
        if let Ok(paths) = tauri::invoke::<_, Vec<String>>(
            "choose_files",
            &Choose {
                title: "Choose a file to convert",
                multiple: false,
            },
        )
        .await
            && let Some(p) = paths.into_iter().next()
        {
            file.set(Some(p));
        }
    });
}

#[component]
fn Convert(view: RwSignal<View>, file: RwSignal<Option<String>>) -> impl IntoView {
    view! {
        <Back view title="Convert files" />
        <ConvertBody file />
    }
}

/// The converter itself, shared by the Tools tab and the converter window.
#[component]
fn ConvertBody(file: RwSignal<Option<String>>) -> impl IntoView {
    let options = RwSignal::new(Vec::<ConvertOption>::new());
    let busy = RwSignal::new(None::<String>);
    let result = RwSignal::new(None::<Result<String, String>>);
    Effect::new(move |_| {
        options.set(Vec::new());
        result.set(None);
        if let Some(path) = file.get() {
            tauri::spawn(async move {
                if let Ok(v) = tauri::invoke("convert_options", &PathArg { path }).await {
                    options.set(v);
                }
            });
        }
    });
    let run = move |target: String| {
        let Some(path) = file.get_untracked() else {
            return;
        };
        busy.set(Some(target.clone()));
        result.set(None);
        tauri::spawn(async move {
            result.set(Some(
                tauri::invoke::<_, String>("convert", &ConvertArg { path, target }).await,
            ));
            busy.set(None);
        });
    };
    view! {
        <div class="dropzone">
            {move || match file.get() {
                Some(p) => view! { <strong>{file_name(&p)}</strong> }.into_any(),
                None => view! { <span class="muted">"Drop a file here"</span> }.into_any(),
            }}
            <button class="btn small-btn" on:click=move |_| choose_for_convert(file)>
                {move || if file.with(Option::is_some) { "Choose another file…" } else { "Choose file…" }}
            </button>
            <div class="muted small">"Everything runs on this computer."</div>
        </div>
        <Show when=move || file.with(Option::is_some) && options.with(Vec::is_empty)>
            <p class="muted small">"No conversions for this kind of file."</p>
        </Show>
        <For each=move || options.get() key=|o| o.id.clone() let:o>
            {
                let id = o.id.clone();
                let id2 = o.id.clone();
                let available = o.missing.is_empty();
                view! {
                    <button class="option" disabled=move || !available || busy.get().is_some() on:click=move |_| run(id.clone())>
                        <span class="grow">{o.label.clone()}
                            {(!o.note.is_empty()).then(|| view! { <span class="muted small">" · "{o.note.clone()}</span> })}
                        </span>
                        {(!available).then(|| view! { <span class="muted small">{o.missing.clone()}</span> })}
                        <span class="small accent-text" class:hidden=move || busy.get().as_deref() != Some(id2.as_str())>"Working…"</span>
                    </button>
                }
            }
        </For>
        {move || result.get().map(|r| match r {
            Ok(p) if p.is_empty() => view! { <p class="small accent-text">"Text copied to the clipboard."</p> }.into_any(),
            Ok(p) => {
                let dir = parent(&p);
                let on_shelf = vec![p.clone()];
                view! {
                    <p class="small">"Saved as "<strong>{file_name(&p)}</strong>" "
                        <button class="link" on:click=move |_| tauri::call("open_path", PathArg { path: dir.clone() })>"Show folder"</button>
                        " · "
                        <button class="link" title="Put the new file on the shelf, to drag it into another app"
                            on:click=move |_| tauri::call("shelf_add", Paths { paths: on_shelf.clone() })>"Put on shelf"</button>
                    </p>
                }.into_any()
            }
            Err(e) => view! { <p class="small error">{e}</p> }.into_any(),
        })}
    }
}

#[component]
fn Cleaner(view: RwSignal<View>) -> impl IntoView {
    let items = RwSignal::new(None::<Vec<CleanItem>>);
    let selected = RwSignal::new(Vec::<u32>::new());
    let state = RwSignal::new(String::new());
    let confirm = RwSignal::new(false);
    let scan = move || {
        items.set(None);
        state.set("Scanning…".into());
        tauri::spawn(async move {
            match tauri::invoke::<_, Vec<CleanItem>>("cleaner_scan", &()).await {
                Ok(v) => {
                    selected.set(v.iter().filter(|i| i.checked).map(|i| i.id).collect());
                    items.set(Some(v));
                    state.set(String::new());
                }
                Err(e) => state.set(e),
            }
        });
    };
    scan();
    let total = move || {
        items.with(|v| {
            v.as_ref().map_or(0.0, |v| {
                v.iter()
                    .filter(|i| selected.with(|s| s.contains(&i.id)))
                    .map(|i| i.size)
                    .sum()
            })
        })
    };
    let clean = move |_| {
        if !confirm.get() {
            confirm.set(true);
            return;
        }
        confirm.set(false);
        let ids = selected.get();
        state.set("Cleaning…".into());
        tauri::spawn(async move {
            match tauri::invoke::<_, f64>("cleaner_run", &Ids { ids }).await {
                Ok(freed) => state.set(format!("Done. Freed {}.", format_bytes(freed))),
                Err(e) => state.set(e),
            }
            items.set(None);
        });
    };
    view! {
        <Back view title="Clean up" />
        <p class="muted small">"Nothing is deleted until you press Clean. Only caches that rebuild themselves start selected."</p>
        <p class="small">{move || state.get()}</p>
        {move || items.get().map(|list| {
            let mut cats: Vec<String> = Vec::new();
            for i in &list {
                if !cats.contains(&i.category) { cats.push(i.category.clone()); }
            }
            cats.into_iter().map(|cat| {
                let rows: Vec<CleanItem> = list.iter().filter(|i| i.category == cat).cloned().collect();
                view! {
                    <h3 class="sub">{cat}</h3>
                    {rows.into_iter().map(|i| {
                        let id = i.id;
                        view! {
                            <label class="check clean">
                                <input type="checkbox" prop:checked=move || selected.with(|s| s.contains(&id))
                                    on:change=move |_| selected.update(|s| if let Some(p) = s.iter().position(|x| *x == id) { s.remove(p); } else { s.push(id); }) />
                                <span class="grow"><span>{i.label.clone()}</span><span class="muted small block">{i.detail.clone()}</span></span>
                                <span class="small">{if i.size > 0.0 { format_bytes(i.size) } else { String::new() }}</span>
                            </label>
                        }
                    }).collect_view()}
                }
            }).collect_view()
        })}
        <div class="row">
            <button class="btn small-btn" on:click=move |_| scan()>"Scan again"</button>
            <button class="btn small-btn danger" disabled=move || selected.with(Vec::is_empty) || items.with(Option::is_none) on:click=clean>
                {move || if confirm.get() { format!("Really delete {}?", format_bytes(total())) } else { format!("Clean {}", format_bytes(total())) }}
            </button>
        </div>
    }
}

#[component]
fn PackageRow(
    p: Package,
    busy: RwSignal<Option<String>>,
    error: RwSignal<String>,
    after: Callback<()>,
) -> impl IntoView {
    let source = if p.source == PkgSource::Flatpak {
        "flatpak"
    } else {
        "rpm"
    };
    let (id, id2) = (p.id.clone(), p.id.clone());
    let action = if p.installed { "remove" } else { "install" };
    let act = move |_| {
        let id = id.clone();
        busy.set(Some(id.clone()));
        error.set(String::new());
        tauri::spawn(async move {
            if let Err(e) =
                tauri::invoke::<_, ()>("pkg_action", &Action { source, action, id }).await
            {
                error.set(e);
            }
            busy.set(None);
            after.run(());
        });
    };
    view! {
        <div class="pkg">
            <div class="grow">
                <div><strong>{p.name.clone()}</strong>" "<span class="muted small">{p.version.clone()}</span></div>
                <div class="muted small ellipsis">{p.summary.clone()}</div>
            </div>
            <button class="btn small-btn" class:danger=p.installed disabled=move || busy.get().is_some() on:click=act>
                {move || if busy.get().as_deref() == Some(id2.as_str()) { "Working…" } else if p.installed { "Remove" } else { "Install" }}
            </button>
        </div>
    }
}

#[component]
fn Packages(view: RwSignal<View>) -> impl IntoView {
    let source = RwSignal::new("rpm");
    let query = RwSignal::new(String::new());
    let results = RwSignal::new(Vec::<Package>::new());
    let updates = RwSignal::new(None::<Vec<Package>>);
    let busy = RwSignal::new(None::<String>);
    let status = RwSignal::new(String::new());
    let search = move || {
        let q = query.get_untracked();
        if q.trim().is_empty() {
            return;
        }
        status.set("Searching…".into());
        tauri::spawn(async move {
            match tauri::invoke::<_, Vec<Package>>(
                "pkg_search",
                &Search {
                    source: source.get_untracked(),
                    query: q,
                },
            )
            .await
            {
                Ok(v) => {
                    status.set(if v.is_empty() {
                        "Nothing found.".into()
                    } else {
                        String::new()
                    });
                    results.set(v);
                }
                Err(e) => status.set(e),
            }
        });
    };
    let check_updates = move |_| {
        updates.set(None);
        status.set("Checking for updates…".into());
        tauri::spawn(async move {
            match tauri::invoke::<_, Vec<Package>>("pkg_updates", &()).await {
                Ok(v) => {
                    status.set(String::new());
                    updates.set(Some(v));
                }
                Err(e) => status.set(e),
            }
        });
    };
    let update_all = move |_| {
        busy.set(Some("all".into()));
        status.set("Updating…".into());
        tauri::spawn(async move {
            match tauri::invoke::<_, ()>("pkg_update_all", &()).await {
                Ok(()) => {
                    status.set("Everything is up to date.".into());
                    updates.set(Some(Vec::new()));
                }
                Err(e) => status.set(e),
            }
            busy.set(None);
        });
    };
    view! {
        <Back view title="Packages" />
        <div class="chips">
            <button class="chip" class:sel=move || source.get() == "rpm" on:click=move |_| source.set("rpm")>"System (RPM)"</button>
            <button class="chip" class:sel=move || source.get() == "flatpak" on:click=move |_| source.set("flatpak")>"Flatpak"</button>
        </div>
        <div class="row">
            <input class="search wide" type="search" placeholder="Search packages"
                on:input=move |e| query.set(event_target_value(&e))
                on:keydown=move |e| if e.key() == "Enter" { search() } />
            <button class="btn small-btn" on:click=move |_| search()>"Search"</button>
        </div>
        <p class="small">{move || status.get()}</p>
        <For each=move || results.get() key=|p| (p.id.clone(), p.installed) let:p>
            <PackageRow p busy error=status after=Callback::new(move |_| search()) />
        </For>
        <h3 class="sub">"Updates"</h3>
        {move || match updates.get() {
            None => view! { <button class="btn small-btn" on:click=check_updates>"Check for updates"</button> }.into_any(),
            Some(v) if v.is_empty() => view! { <p class="muted small">"No updates."</p> }.into_any(),
            Some(v) => view! {
                <p class="small">{format!("{} updates available", v.len())}</p>
                {v.into_iter().take(30).map(|p| view! {
                    <div class="kv"><span>{p.name}</span><span class="muted small">{p.version}</span></div>
                }).collect_view()}
                <button class="btn small-btn" disabled=move || busy.get().is_some() on:click=update_all>"Update all"</button>
            }.into_any(),
        }}
    }
}

/// Shelf items. Each row can be dragged out into Files, a browser or any
/// app that takes files (a copy: the original stays where it is).
#[component]
fn ShelfList(
    items: RwSignal<Vec<ShelfItem>>,
    keep_hours: Signal<Option<u32>>,
    #[prop(optional)] convert: Option<Callback<String>>,
) -> impl IntoView {
    view! {
        <For each=move || items.get() key=|i| (i.path.clone(), i.added) let:i>
            {
                let (p1, p2, p3, p4) = (i.path.clone(), i.path.clone(), i.path.clone(), i.path.clone());
                let drag_path = i.path.clone();
                let detail = if i.is_dir { "Folder".to_owned() } else { format_bytes(i.size) };
                let added = i.added;
                view! {
                    <div class="pkg shelf-item" draggable="true" title="Drag into another app"
                        on:dragstart=move |e: leptos::ev::DragEvent| {
                            // WebKit's own drag does not offer a proper file list
                            // (Files refused most drops). Cancel it and let the
                            // app start a real GTK file drag (crates/ui/src/drag.rs).
                            e.prevent_default();
                            tauri::call("start_file_drag", Paths { paths: vec![drag_path.clone()] });
                        }>
                        <div class="grow">
                            <div class="ellipsis"><strong>{i.name.clone()}</strong></div>
                            <div class="muted small">
                                {detail.clone()}
                                {move || { let t = time_left(added, keep_hours.get()); (!t.is_empty()).then(|| format!(" · {t}")) }}
                            </div>
                        </div>
                        <button class="iconbtn" title="Open" on:click=move |_| tauri::call("open_file", PathArg { path: p1.clone() })>"↗"</button>
                        <button class="iconbtn" title="Copy (paste in Files)" on:click=move |_| tauri::call("shelf_copy", PathArg { path: p2.clone() })>"⧉"</button>
                        {convert.map(|c| view! {
                            <button class="iconbtn" title="Convert" on:click=move |_| c.run(p3.clone())>"⇄"</button>
                        })}
                        <button class="iconbtn" title="Remove from shelf" on:click=move |_| tauri::call("shelf_remove", PathArg { path: p4.clone() })>"✕"</button>
                    </div>
                }
            }
        </For>
    }
}

/// Opens GNOME's file chooser and puts the chosen files on the shelf.
fn choose_for_shelf() {
    tauri::spawn(async move {
        if let Ok(paths) = tauri::invoke::<_, Vec<String>>(
            "choose_files",
            &Choose {
                title: "Put files on the shelf",
                multiple: true,
            },
        )
        .await
        {
            tauri::call("shelf_add", Paths { paths });
        }
    });
}

/// Live shelf contents and the expiry setting, following daemon events.
fn shelf_state() -> (RwSignal<Vec<ShelfItem>>, RwSignal<Option<u32>>) {
    let items = RwSignal::new(Vec::<ShelfItem>::new());
    let keep = RwSignal::new(Some(24));
    tauri::spawn(async move {
        if let Some(v) = tauri::get("shelf_list").await {
            items.set(v);
        }
        if let Some(c) = tauri::get::<Config>("config").await {
            keep.set(c.shelf.keep_hours);
        }
    });
    tauri::listen::<Vec<ShelfItem>>("shelf", move |v| items.set(v));
    tauri::listen::<Config>("config", move |c| keep.set(c.shelf.keep_hours));
    (items, keep)
}

fn keep_text(keep: Option<u32>) -> String {
    match keep {
        Some(1) => "Files leave the shelf after 1 hour.".into(),
        Some(h) => format!("Files leave the shelf after {h} hours."),
        None => "Files stay until you remove them.".into(),
    }
}

#[component]
fn Shelf(
    view: RwSignal<View>,
    items: RwSignal<Vec<ShelfItem>>,
    keep: RwSignal<Option<u32>>,
    convert: Callback<String>,
) -> impl IntoView {
    view! {
        <Back view title="Shelf" />
        <div class="dropzone">
            <span class="muted">"Drop files here or on the box icon in the top bar"</span>
            <button class="btn small-btn" on:click=|_| choose_for_shelf()>"Choose files…"</button>
        </div>
        <ShelfList items keep_hours=keep.into() convert />
        <Show when=move || !items.with(Vec::is_empty)>
            <button class="btn small-btn" on:click=|_| tauri::call("shelf_clear", ())>"Clear shelf"</button>
        </Show>
        <p class="muted small">{move || keep_text(keep.get())}</p>
    }
}

/// The small window under the converter icon in the top bar. A file
/// dropped on the icon arrives as `convert-file` (or, while this page is
/// still loading, from `take_convert_file`).
#[component]
pub fn ConvertWindow() -> impl IntoView {
    let file = RwSignal::new(None::<String>);
    tauri::spawn(async move {
        if let Some(Some(p)) = tauri::get::<Option<String>>("take_convert_file").await {
            file.set(Some(p));
        }
    });
    tauri::listen::<String>("convert-file", move |p| {
        file.set(Some(p));
        tauri::call("take_convert_file", ());
    });
    tauri::on_file_drop(move |paths| file.set(paths.into_iter().next()));
    let (shelf, _) = shelf_state();
    view! {
        <main class="glass tool">
            <header class="tool-head">
                <Icon name="file-symlink" />
                <strong class="grow">"Convert"</strong>
            </header>
            <section class="content">
                <ConvertBody file />
                <Show when=move || file.with(Option::is_none) && !shelf.with(Vec::is_empty)>
                    <h3 class="sub">"From the shelf"</h3>
                    <div class="chips">
                        <For each=move || shelf.get() key=|i| i.path.clone() let:i>
                            {
                                let p = i.path.clone();
                                view! { <button class="chip" on:click=move |_| file.set(Some(p.clone()))>{i.name.clone()}</button> }
                            }
                        </For>
                    </div>
                </Show>
            </section>
            <footer class="muted small tool-foot">"Drop a file on the converter icon in the top bar, or here."</footer>
        </main>
    }
}

/// The small window under the box icon in the top bar.
#[component]
pub fn ShelfWindow() -> impl IntoView {
    let (items, keep) = shelf_state();
    tauri::on_file_drop(|paths| tauri::call("shelf_add", Paths { paths }));
    let empty = move || items.with(Vec::is_empty);
    view! {
        <main class="glass tool">
            <header class="tool-head">
                <Icon name="package" />
                <strong class="grow">"Shelf"</strong>
                <button class="btn small-btn" title="Choose files to put on the shelf" on:click=|_| choose_for_shelf()>"Add…"</button>
                <button class="btn small-btn" disabled=empty on:click=|_| tauri::call("shelf_clear", ())>"Clear"</button>
            </header>
            <section class="content">
                <Show when=move || !empty()
                    fallback=|| view! {
                        <div class="dropzone grow-zone">
                            <span>"Drop files on the box icon"</span>
                            <span class="muted small">"or here, to keep them handy. Drag them out again into any app."</span>
                        </div>
                    }>
                    <ShelfList items keep_hours=keep.into() />
                </Show>
            </section>
            <footer class="muted small tool-foot">{move || keep_text(keep.get())}</footer>
        </main>
    }
}

#[component]
pub fn ToolsTab() -> impl IntoView {
    let view = RwSignal::new(View::List);
    let file = RwSignal::new(None::<String>);
    let (shelf, keep) = shelf_state();
    // Files dropped anywhere on the panel.
    tauri::on_file_drop(move |paths| {
        if view.get_untracked() == View::Convert {
            file.set(paths.into_iter().next());
        } else {
            tauri::call("shelf_add", Paths { paths });
            view.set(View::Shelf);
        }
    });
    let to_convert = Callback::new(move |p: String| {
        file.set(Some(p));
        view.set(View::Convert);
    });

    view! {
        {move || match view.get() {
            View::List => view! {
                <div class="toollist">
                    <ToolCard icon="clipboard" title="Clipboard" desc="Text you copied. Stays on this computer." on_click=move || view.set(View::Clipboard) />
                    <ToolCard icon="scan-text" title="Copy text from screen" desc="Select an area; its text or QR code is copied." on_click=|| tauri::call("ocr", ()) />
                    <ToolCard icon="pipette" title="Pick a color" desc="Click any pixel; its hex code is copied." on_click=|| tauri::call("pick_color", ()) />
                    <ToolCard icon="file-symlink" title="Convert files" desc="Images, audio, video, documents. Compress, make GIFs." on_click=move || view.set(View::Convert) />
                    <ToolCard icon="sparkles" title="Clean up" desc="Caches, Trash, leftovers of removed apps." on_click=move || view.set(View::Cleaner) />
                    <ToolCard icon="package" title="Packages" desc="Install, remove and update RPM and Flatpak apps." on_click=move || view.set(View::Packages) />
                    <ToolCard icon="inbox" title="Shelf" desc="A place to park files you are about to use." on_click=move || view.set(View::Shelf) />
                </div>
            }.into_any(),
            View::Clipboard => view! { <Clipboard view /> }.into_any(),
            View::Convert => view! { <Convert view file /> }.into_any(),
            View::Cleaner => view! { <Cleaner view /> }.into_any(),
            View::Packages => view! { <Packages view /> }.into_any(),
            View::Shelf => view! { <Shelf view items=shelf keep convert=to_convert /> }.into_any(),
        }}
    }
}
