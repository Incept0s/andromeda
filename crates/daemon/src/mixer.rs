// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-app volume mixer on PipeWire, through its PulseAudio API
//! (`pipewire-pulse`, installed on every Fedora Workstation).
//!
//! libpulse is callback based and its objects are single-threaded, so one
//! dedicated thread owns the connection:
//! - any change in streams/devices sets a "dirty" flag; the thread then
//!   re-reads everything and sends a fresh `MixerState` snapshot;
//! - commands from D-Bus arrive over a channel that the thread checks at
//!   least every 50 ms.
//!
//! Volumes above 100 % are allowed by PipeWire. They are applied in
//! software, so loud audio can clip; the cap comes from the user's config.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use andromeda_core::info::{AudioDevice, MixApp, MixerState};
use andromeda_core::procs::desktop_entry;
use libpulse_binding as pa;
use pa::callbacks::ListResult;
use pa::context::subscribe::InterestMaskSet;
use pa::context::{Context, FlagSet, State};
use pa::mainloop::standard::Mainloop;
use pa::volume::{ChannelVolumes, Volume};
use tokio::sync::watch;

use crate::icons::Icons;

pub enum Cmd {
    /// App key, percent (0..=max).
    Volume(String, f64),
    Mute(String, bool),
    /// App key, sink name.
    Output(String, String),
    DefaultOutput(String),
    DefaultInput(String),
}

/// One stream as read from the server, before grouping into apps.
#[derive(Clone)]
struct Stream {
    index: u32,
    key: String,
    name: String,
    icon_names: Vec<String>,
    volume: ChannelVolumes,
    muted: bool,
    corked: bool,
    sink: u32,
}

#[derive(Default)]
struct Snapshot {
    streams: Vec<Stream>,
    sinks: Vec<(u32, AudioDevice)>,
    sources: Vec<AudioDevice>,
    default_sink: String,
    default_source: String,
    /// Outstanding list requests; the snapshot is complete at zero.
    pending: u32,
}

/// Groups streams by app. Flatpak id first (stable and unique), then the
/// binary, then whatever name the stream gave itself.
fn stream_key(p: &pa::proplist::Proplist) -> (String, String, Vec<String>) {
    let get = |k: &str| p.get_str(k).filter(|s| !s.is_empty());
    let app_id = get("pipewire.access.portal.app_id");
    let binary = get("application.process.binary");
    let app_name = get("application.name");
    let key = app_id
        .clone()
        .or_else(|| binary.as_ref().map(|b| b.to_lowercase()))
        .or(app_name.clone())
        .unwrap_or_else(|| "unknown".into());
    // Prefer the .desktop name: streams often call themselves things like
    // "WEBRTC VoiceEngine" or "Playback".
    let entry = app_id
        .as_deref()
        .and_then(desktop_entry)
        .or_else(|| binary.as_deref().and_then(desktop_entry));
    let name = entry
        .as_ref()
        .map(|(n, _)| n.clone())
        .or(app_name)
        .or(binary.clone())
        .unwrap_or_else(|| key.clone());
    let icons = [
        entry.and_then(|(_, i)| i),
        get("application.icon_name"),
        app_id,
        binary.map(|b| b.to_lowercase()),
    ]
    .into_iter()
    .flatten()
    .collect();
    (key, name, icons)
}

fn percent(v: &ChannelVolumes) -> f64 {
    f64::from(v.avg().0) / f64::from(Volume::NORMAL.0) * 100.0
}

pub fn spawn(
    max_volume: impl Fn() -> f64 + Send + 'static,
) -> (mpsc::Sender<Cmd>, watch::Receiver<MixerState>) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (state_tx, state_rx) = watch::channel(MixerState::default());
    std::thread::Builder::new()
        .name("mixer".into())
        .spawn(move || {
            // Reconnect with a pause if PipeWire restarts or is missing.
            loop {
                run(&cmd_rx, &state_tx, &max_volume);
                let _ = state_tx.send(MixerState::default());
                std::thread::sleep(Duration::from_secs(3));
            }
        })
        .expect("spawn mixer thread");
    (cmd_tx, state_rx)
}

/// One connection's lifetime. Returns when the connection fails.
fn run(cmds: &mpsc::Receiver<Cmd>, out: &watch::Sender<MixerState>, max_volume: &dyn Fn() -> f64) {
    let Some(mut ml) = Mainloop::new() else {
        return;
    };
    let Some(ctx) = Context::new(&ml, "Andromeda") else {
        return;
    };
    let ctx = Rc::new(RefCell::new(ctx));
    if ctx
        .borrow_mut()
        .connect(None, FlagSet::NOFLAGS, None)
        .is_err()
    {
        return;
    }
    // Wait until ready (or failed).
    loop {
        ml.iterate(true);
        match ctx.borrow().get_state() {
            State::Ready => break,
            State::Failed | State::Terminated => return,
            _ => {}
        }
    }

    let dirty = Rc::new(Cell::new(true));
    {
        let d = dirty.clone();
        let mut c = ctx.borrow_mut();
        c.set_subscribe_callback(Some(Box::new(move |_, _, _| d.set(true))));
        c.subscribe(
            InterestMaskSet::SINK_INPUT
                | InterestMaskSet::SINK
                | InterestMaskSet::SOURCE
                | InterestMaskSet::SERVER,
            |_| {},
        );
    }

    let snap = Rc::new(RefCell::new(Snapshot::default()));
    let mut icons = Icons::default();
    let mut refreshing = false;

    loop {
        // Block up to 50 ms waiting for server events, then handle commands.
        if ml.prepare(Some(pa::time::MicroSeconds(50_000))).is_err()
            || ml.poll().is_err()
            || ml.dispatch().is_err()
        {
            return;
        }
        if !matches!(ctx.borrow().get_state(), State::Ready) {
            return;
        }

        while let Ok(cmd) = cmds.try_recv() {
            apply(&ctx, &snap.borrow(), cmd, max_volume());
        }

        if dirty.get() && !refreshing {
            dirty.set(false);
            refreshing = true;
            request(&ctx, &snap);
        }
        if refreshing && snap.borrow().pending == 0 {
            refreshing = false;
            let _ = out.send(build(&snap.borrow(), &mut icons));
        }
    }
}

/// Asks the server for all streams, sinks, sources and defaults. Each
/// callback decrements `pending` when its list ends.
fn request(ctx: &Rc<RefCell<Context>>, snap: &Rc<RefCell<Snapshot>>) {
    let intro = ctx.borrow().introspect();
    {
        let mut s = snap.borrow_mut();
        *s = Snapshot {
            pending: 4,
            ..Default::default()
        };
    }
    let s = snap.clone();
    intro.get_sink_input_info_list(move |r| match r {
        // Virtual loopbacks (audio routing tools) are plumbing, not apps.
        ListResult::Item(i) if i.proplist.get_str("node.virtual").as_deref() == Some("true") => {}
        ListResult::Item(i) => {
            let (key, name, icon_names) = stream_key(&i.proplist);
            s.borrow_mut().streams.push(Stream {
                index: i.index,
                key,
                name,
                icon_names,
                volume: i.volume,
                muted: i.mute,
                corked: i.corked,
                sink: i.sink,
            });
        }
        _ => s.borrow_mut().pending -= 1,
    });
    let s = snap.clone();
    intro.get_sink_info_list(move |r| match r {
        ListResult::Item(i) => {
            let dev = AudioDevice {
                name: i.name.as_deref().unwrap_or_default().to_owned(),
                description: i.description.as_deref().unwrap_or_default().to_owned(),
            };
            s.borrow_mut().sinks.push((i.index, dev));
        }
        _ => s.borrow_mut().pending -= 1,
    });
    let s = snap.clone();
    intro.get_source_info_list(move |r| match r {
        // Monitors are "what the speakers play", not microphones.
        ListResult::Item(i) if i.monitor_of_sink.is_none() => {
            s.borrow_mut().sources.push(AudioDevice {
                name: i.name.as_deref().unwrap_or_default().to_owned(),
                description: i.description.as_deref().unwrap_or_default().to_owned(),
            });
        }
        ListResult::Item(_) => {}
        _ => s.borrow_mut().pending -= 1,
    });
    let s = snap.clone();
    intro.get_server_info(move |i| {
        let mut s = s.borrow_mut();
        s.default_sink = i
            .default_sink_name
            .as_deref()
            .unwrap_or_default()
            .to_owned();
        s.default_source = i
            .default_source_name
            .as_deref()
            .unwrap_or_default()
            .to_owned();
        s.pending -= 1;
    });
}

fn build(s: &Snapshot, icons: &mut Icons) -> MixerState {
    let sink_name = |idx: u32| {
        s.sinks
            .iter()
            .find(|(i, _)| *i == idx)
            .map(|(_, d)| d.name.clone())
            .unwrap_or_default()
    };
    let mut apps: Vec<MixApp> = Vec::new();
    let mut order: HashMap<String, usize> = HashMap::new();
    for st in &s.streams {
        let i = *order.entry(st.key.clone()).or_insert_with(|| {
            apps.push(MixApp {
                key: st.key.clone(),
                name: st.name.clone(),
                icon: icons.data_url(&st.icon_names),
                volume: 0.0,
                muted: true,
                playing: false,
                output: sink_name(st.sink),
                streams: 0,
            });
            apps.len() - 1
        });
        let a = &mut apps[i];
        a.volume += percent(&st.volume);
        // An app counts as muted only if every stream is.
        a.muted &= st.muted;
        a.playing |= !st.corked;
        a.streams += 1;
    }
    for a in &mut apps {
        a.volume /= f64::from(a.streams.max(1));
    }
    apps.sort_by(|a, b| {
        b.playing
            .cmp(&a.playing)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    MixerState {
        connected: true,
        apps,
        outputs: s.sinks.iter().map(|(_, d)| d.clone()).collect(),
        inputs: s.sources.clone(),
        default_output: s.default_sink.clone(),
        default_input: s.default_source.clone(),
    }
}

fn apply(ctx: &Rc<RefCell<Context>>, s: &Snapshot, cmd: Cmd, max: f64) {
    let streams = |key: &str| {
        s.streams
            .iter()
            .filter(|st| st.key == key)
            .cloned()
            .collect::<Vec<_>>()
    };
    let mut intro = ctx.borrow().introspect();
    match cmd {
        Cmd::Volume(key, pct) => {
            let v = Volume((pct.clamp(0.0, max) / 100.0 * f64::from(Volume::NORMAL.0)) as u32);
            for st in streams(&key) {
                let mut cv = st.volume;
                let channels = cv.len();
                cv.set(channels, v);
                intro.set_sink_input_volume(st.index, &cv, None);
            }
        }
        Cmd::Mute(key, mute) => {
            for st in streams(&key) {
                intro.set_sink_input_mute(st.index, mute, None);
            }
        }
        Cmd::Output(key, sink) => {
            if let Some((idx, _)) = s.sinks.iter().find(|(_, d)| d.name == sink) {
                for st in streams(&key) {
                    intro.move_sink_input_by_index(st.index, *idx, None);
                }
            }
        }
        Cmd::DefaultOutput(name) => {
            ctx.borrow_mut().set_default_sink(&name, |_| {});
        }
        Cmd::DefaultInput(name) => {
            ctx.borrow_mut().set_default_source(&name, |_| {});
        }
    }
}
