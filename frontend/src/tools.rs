// SPDX-License-Identifier: GPL-3.0-or-later
//! Tools tab: a list of tools like the original's Utilities; each opens a
//! sub-view with a back button. Files dropped on the panel go to the
//! converter when it is open, otherwise to the shelf.

use andromeda_core::metrics::format_bytes;
use andromeda_core::tools::{CleanItem, ClipEntry, ConvertOption, Package, PkgSource, ShelfItem};
use leptos::prelude::*;
use serde::Serialize;

use crate::Icon;
use crate::tauri;
use crate::widgets::{POLL_FAST, poll};

#[derive(Clone, Copy, PartialEq)]
enum View {
    List,
    Clipboard,
    Convert,
    Cleaner,
    Packages,
    Shelf,
}

#[derive(serde::Deserialize)]
struct Drop {
    paths: Vec<String>,
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

#[component]
fn Clipboard(view: RwSignal<View>) -> impl IntoView {
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
    poll(POLL_FAST, refresh);
    view! {
        <Back view title="Clipboard" />
        <input class="search wide" type="search" placeholder="Search copied text"
            on:input=move |e| { query.set(event_target_value(&e)); refresh(); } />
        <Show when=move || !items.with(Vec::is_empty)
            fallback=|| view! { <p class="muted small">"Copied text shows up here. It stays on this computer."</p> }>
            <For each=move || items.get() key=|e| (e.id, e.pinned) let:e>
                <div class="clip" class:pinned=e.pinned>
                    <button class="cliptext" title="Copy" on:click=move |_| {
                        tauri::call("clipboard_copy", Id { id: e.id });
                        copied.set(Some(e.id));
                    }>
                        {e.text.chars().take(240).collect::<String>()}
                    </button>
                    <div class="clipactions">
                        <span class="small accent-text" class:hidden=move || copied.get() != Some(e.id)>"Copied"</span>
                        <button class="iconbtn" title=if e.pinned { "Unpin" } else { "Pin" }
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
fn Convert(view: RwSignal<View>, file: RwSignal<Option<String>>) -> impl IntoView {
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
        <Back view title="Convert files" />
        <div class="dropzone">
            {move || match file.get() {
                Some(p) => view! { <strong>{file_name(&p)}</strong> }.into_any(),
                None => view! { <span class="muted">"Drop a file here"</span> }.into_any(),
            }}
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
                view! {
                    <p class="small">"Saved as "<strong>{file_name(&p)}</strong>" "
                        <button class="link" on:click=move |_| tauri::call("open_path", PathArg { path: dir.clone() })>"Show folder"</button>
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

#[component]
fn Shelf(
    view: RwSignal<View>,
    items: RwSignal<Vec<ShelfItem>>,
    refresh: Callback<()>,
    convert: Callback<String>,
) -> impl IntoView {
    view! {
        <Back view title="Shelf" />
        <div class="dropzone"><span class="muted">"Drop files here to keep them handy"</span></div>
        <For each=move || items.get() key=|i| i.path.clone() let:i>
            {
                let (p1, p2, p3, p4) = (i.path.clone(), i.path.clone(), i.path.clone(), i.path.clone());
                view! {
                    <div class="pkg">
                        <div class="grow">
                            <div class="ellipsis"><strong>{i.name.clone()}</strong></div>
                            <div class="muted small">{if i.is_dir { "Folder".into() } else { format_bytes(i.size) }}</div>
                        </div>
                        <button class="iconbtn" title="Open" on:click=move |_| tauri::call("open_file", PathArg { path: p1.clone() })>"↗"</button>
                        <button class="iconbtn" title="Copy (paste in Files)" on:click=move |_| tauri::call("shelf_copy", PathArg { path: p2.clone() })>"⧉"</button>
                        <button class="iconbtn" title="Convert" on:click=move |_| convert.run(p3.clone())>"⇄"</button>
                        <button class="iconbtn" title="Remove from shelf" on:click=move |_| {
                            tauri::call("shelf_remove", PathArg { path: p4.clone() });
                            refresh.run(());
                        }>"✕"</button>
                    </div>
                }
            }
        </For>
    }
}

#[component]
pub fn ToolsTab() -> impl IntoView {
    let view = RwSignal::new(View::List);
    let file = RwSignal::new(None::<String>);
    let shelf = RwSignal::new(Vec::<ShelfItem>::new());
    let refresh_shelf = Callback::new(move |_| {
        tauri::spawn(async move {
            if let Some(v) = tauri::get("shelf_list").await {
                shelf.set(v);
            }
        });
    });
    refresh_shelf.run(());
    // Files dropped anywhere on the panel.
    tauri::listen::<Drop>("tauri://drag-drop", move |d| {
        if view.get_untracked() == View::Convert {
            file.set(d.paths.into_iter().next());
        } else {
            tauri::call("shelf_add", Paths { paths: d.paths });
            view.set(View::Shelf);
            refresh_shelf.run(());
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
            View::Shelf => view! { <Shelf view items=shelf refresh=refresh_shelf convert=to_convert /> }.into_any(),
        }}
    }
}
