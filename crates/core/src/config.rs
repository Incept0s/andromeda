// SPDX-License-Identifier: GPL-3.0-or-later
//! User settings. The daemon owns the file
//! (`~/.config/io.github.incept0s.Andromeda/config.toml`) and publishes it as
//! JSON over D-Bus; the Settings window edits a copy and sends it back.
//!
//! `#[serde(default)]` everywhere: a missing or older file still loads, and
//! unknown keys are ignored. `sanitize()` clamps values from a hand-edited
//! file so nothing downstream has to trust them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub features: Features,
    /// Metric ids shown in the top bar, in order. `None` = not chosen yet,
    /// the daemon then picks defaults that exist on this machine.
    pub topbar: Option<Vec<String>>,
    /// Which side of the top bar the numbers sit on.
    pub topbar_side: TopBarSide,
    /// Extra top-bar icons for tools (right of the open apps).
    pub icons: Icons,
    pub shelf: Shelf,
    pub shortcuts: Shortcuts,
    pub alerts: Alerts,
    pub mixer: Mixer,
    pub awake: Awake,
}

/// One switch per feature. Off hides the tab and stops its background work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Features {
    pub awake: bool,
    pub mixer: bool,
    pub system: bool,
    pub network: bool,
    pub disk: bool,
    pub energy: bool,
    pub tools: bool,
    pub controls: bool,
    pub alerts: bool,
    pub topbar: bool,
    pub clipboard: bool,
    pub switcher: bool,
}

impl Default for Features {
    fn default() -> Self {
        Self {
            awake: true,
            mixer: true,
            system: true,
            network: true,
            disk: true,
            energy: true,
            tools: true,
            controls: true,
            alerts: true,
            topbar: true,
            clipboard: true,
            switcher: true,
        }
    }
}

/// Tool icons the Shell extension shows in the top bar, between the open
/// apps and the Andromeda icon. Each tool also stays in the Tools tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Icons {
    /// Box icon: drop files on it to park them, click it to get them back.
    pub shelf: bool,
    /// Converter: drop a file on it to see what it can be turned into.
    pub convert: bool,
    /// Clipboard: the last copied texts and images, click one to copy it.
    pub clipboard: bool,
}

impl Default for Icons {
    fn default() -> Self {
        Self {
            shelf: true,
            convert: false,
            clipboard: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shelf {
    /// Files leave the shelf this many hours after they were added;
    /// `None` = they stay until removed.
    pub keep_hours: Option<u32>,
}

impl Default for Shelf {
    fn default() -> Self {
        Self {
            keep_hours: Some(24),
        }
    }
}

/// Keyboard shortcuts, as GTK accelerators ("<Super><Shift>t"); empty =
/// off. The Shell extension registers them (it mirrors these into its own
/// GSettings, which is what Mutter reads), so they work in every app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    /// Show or hide the panel.
    pub panel: String,
    /// Copy text (or a QR code) from an area of the screen.
    pub copy_text: String,
    /// Pick a color anywhere on screen; its hex code is copied.
    pub pick_color: String,
    /// Keep awake on/off (on for the default duration).
    pub keep_awake: String,
    /// Microphone mute on/off.
    pub mic_mute: String,
}

impl Default for Shortcuts {
    /// Free on a stock GNOME 50 (Super+A is taken from "Show apps").
    fn default() -> Self {
        Self {
            panel: "<Super>a".into(),
            copy_text: "<Super><Shift>t".into(),
            pick_color: "<Super><Shift>c".into(),
            keep_awake: "<Super><Shift>k".into(),
            mic_mute: "<Super><Shift>m".into(),
        }
    }
}

impl Shortcuts {
    /// Every shortcut string, for code that treats them alike.
    pub fn all_mut(&mut self) -> [&mut String; 5] {
        [
            &mut self.panel,
            &mut self.copy_text,
            &mut self.pick_color,
            &mut self.keep_awake,
            &mut self.mic_mute,
        ]
    }
}

/// Where the Shell extension puts the top-bar numbers. The Andromeda icon
/// itself always sits on the right, just before the system icons.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TopBarSide {
    /// Right half of the bar, after the clock and before the open apps.
    #[default]
    Right,
    /// Far left of the bar, after Activities (or the app menu).
    Left,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Above,
    Below,
}

/// "Notify when `metric` goes above/below `threshold` for 12 s."
/// Rules are generic over the registry, so any metric can get an alert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertRule {
    pub metric: String,
    pub direction: Direction,
    pub threshold: f64,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Alerts {
    pub rules: Vec<AlertRule>,
    /// Minimum time between two notifications for the same rule.
    pub cooldown_minutes: u32,
}

impl Default for Alerts {
    /// The reference's presets, all off until the user enables them.
    /// Battery and disk rules use `*` patterns that match any battery/disk.
    fn default() -> Self {
        let rule = |metric: &str, direction, threshold| AlertRule {
            metric: metric.into(),
            direction,
            threshold,
            enabled: false,
        };
        Self {
            rules: vec![
                rule("cpu.usage", Direction::Above, 90.0),
                rule("cpu.temp", Direction::Above, 90.0),
                rule("gpu.*.temp", Direction::Above, 90.0),
                rule("mem.pressure", Direction::Above, 30.0),
                rule("disk.*.free_pct", Direction::Below, 10.0),
                rule("bat.*.charge", Direction::Below, 15.0),
            ],
            cooldown_minutes: 15,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mixer {
    /// Highest per-app volume in percent: 153 (default) or up to 200.
    /// Above 100 % audio is amplified in software and can clip.
    pub max_volume: u32,
    /// Apps hidden from the list, by app key.
    pub hidden: Vec<String>,
    /// Volume (%) to drop to when headphones disconnect; `None` = off.
    pub headphones_disconnect_volume: Option<u32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            max_volume: 153,
            hidden: Vec::new(),
            headphones_disconnect_volume: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Awake {
    /// Length of a session started with one click, in minutes (0 = until off).
    pub default_minutes: u32,
    /// Stop keeping awake when the battery drops below this (%); `None` = never.
    pub stop_below_battery: Option<u32>,
    pub start_with_app: bool,
    /// Also keep running with the lid closed (no suspend on lid close).
    pub lid_closed: bool,
}

impl Default for Awake {
    fn default() -> Self {
        Self {
            default_minutes: 60,
            stop_below_battery: Some(20),
            start_with_app: false,
            lid_closed: false,
        }
    }
}

/// Top-bar entries before the user warns "too many": the bar has little room.
pub const TOPBAR_SOFT_LIMIT: usize = 4;

impl Config {
    pub fn sanitize(&mut self) {
        self.mixer.max_volume = self.mixer.max_volume.clamp(100, 200);
        if let Some(v) = &mut self.mixer.headphones_disconnect_volume {
            *v = (*v).min(100);
        }
        if let Some(b) = &mut self.awake.stop_below_battery {
            *b = (*b).clamp(1, 99);
        }
        self.awake.default_minutes = self.awake.default_minutes.min(24 * 60);
        // Accelerators are short ASCII like "<Super><Shift>t"; anything else
        // (a broken hand edit) switches that shortcut off.
        for s in self.shortcuts.all_mut() {
            let ok = s.len() <= 64 && s.chars().all(|c| c.is_ascii_graphic());
            if !ok {
                s.clear();
            }
        }
        if let Some(h) = &mut self.shelf.keep_hours {
            *h = (*h).clamp(1, 24 * 30);
        }
        self.alerts.cooldown_minutes = self.alerts.cooldown_minutes.clamp(1, 24 * 60);
        self.alerts
            .rules
            .retain(|r| r.threshold.is_finite() && !r.metric.is_empty());
        if let Some(t) = &mut self.topbar {
            let mut seen = std::collections::HashSet::new();
            t.retain(|id| seen.insert(id.clone()));
        }
    }
}

/// The user's top-bar choice, or the defaults that exist on this machine:
/// CPU temperature, memory %, laptop battery power.
pub fn topbar_or_default(c: &Config, metrics: &[crate::metrics::MetricInfo]) -> Vec<String> {
    if let Some(t) = &c.topbar {
        return t.clone();
    }
    let has = |id: &str| metrics.iter().any(|m| m.id == id);
    let mut v: Vec<String> = ["cpu.temp", "mem.used_pct"]
        .into_iter()
        .filter(|id| has(id))
        .map(String::from)
        .collect();
    // The laptop battery is the one with a health metric (not a mouse).
    if let Some(m) = metrics.iter().find(|m| {
        m.id.ends_with(".power")
            && m.id.starts_with("bat.")
            && has(&m.id.replace(".power", ".health"))
    }) {
        v.push(m.id.clone());
    }
    v
}

/// `gpu.*.temp` matches `gpu.card1.temp`; `*` stands for one id segment.
pub fn pattern_matches(pattern: &str, id: &str) -> bool {
    let (mut p, mut i) = (pattern.split('.'), id.split('.'));
    loop {
        match (p.next(), i.next()) {
            (None, None) => return true,
            (Some("*"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_input_fills_defaults() {
        let c: Config = serde_json::from_str(r#"{"mixer":{"max_volume":180}}"#).unwrap();
        assert_eq!(c.mixer.max_volume, 180);
        assert!(c.features.mixer);
        assert_eq!(c.alerts.cooldown_minutes, 15);
        assert_eq!(c.topbar_side, TopBarSide::Right);
        assert!(c.icons.shelf);
        assert!(!c.icons.convert);
        assert_eq!(c.shelf.keep_hours, Some(24));
        let left: Config = serde_json::from_str(r#"{"topbar_side":"Left"}"#).unwrap();
        assert_eq!(left.topbar_side, TopBarSide::Left);
    }

    #[test]
    fn sanitizes() {
        let mut c = Config::default();
        c.mixer.max_volume = 900;
        c.awake.stop_below_battery = Some(0);
        c.shortcuts.mic_mute = "<Super>\u{e9}\n".into();
        c.topbar = Some(vec!["a".into(), "b".into(), "a".into()]);
        c.alerts.rules.push(AlertRule {
            metric: "x".into(),
            direction: Direction::Above,
            threshold: f64::NAN,
            enabled: true,
        });
        c.sanitize();
        assert_eq!(c.mixer.max_volume, 200);
        assert_eq!(c.awake.stop_below_battery, Some(1));
        assert_eq!(c.shortcuts.copy_text, "<Super><Shift>t");
        assert_eq!(c.topbar, Some(vec!["a".into(), "b".into()]));
        assert!(c.alerts.rules.iter().all(|r| r.threshold.is_finite()));
        assert_eq!(
            c.shortcuts.mic_mute, "",
            "invalid accelerator is switched off"
        );
    }

    #[test]
    fn patterns() {
        assert!(pattern_matches("gpu.*.temp", "gpu.card1.temp"));
        assert!(pattern_matches("cpu.temp", "cpu.temp"));
        assert!(!pattern_matches("gpu.*.temp", "gpu.card1.usage"));
        assert!(!pattern_matches("bat.*.charge", "bat.BAT0.charge.x"));
    }
}
