// SPDX-License-Identifier: GPL-3.0-or-later
//
// The sustained-reading gate below is adapted from Vorssaint Utils
// (Sources/Vorssaint/Services/Metrics/SustainedAlertGate.swift),
// Copyright (C) 2026 Vorssaint, GPL-3.0-or-later. Modified: translated to
// Rust, generic over any metric and direction, cooldown added here.

//! Threshold alerts. A single reading over the limit proves nothing (a
//! short burst can spike CPU load or a core sensor), so a rule fires only
//! after the reading has held for `SUSTAINED` across separate samples, and
//! then not again for the configured cooldown.
//!
//! Times come from `Instant`, a monotonic clock that does not advance while
//! the machine is suspended, so a suspend in the middle of a stretch cannot
//! pass for a long sustained reading.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use andromeda_core::config::{self, Direction};
use andromeda_core::metrics::{MetricInfo, format};

pub const SUSTAINED: Duration = Duration::from_secs(12);

#[derive(Default)]
struct Gate {
    held_since: Option<Instant>,
    last_sent: Option<Instant>,
}

#[derive(Default)]
pub struct Alerts {
    /// Keyed by (rule index, concrete metric id): one rule like
    /// `gpu.*.temp` watches every GPU separately.
    gates: HashMap<(usize, String), Gate>,
}

pub struct Notice {
    pub title: String,
    pub body: String,
}

impl Alerts {
    pub fn check(
        &mut self,
        cfg: &config::Alerts,
        metrics: &[MetricInfo],
        values: &HashMap<String, f64>,
        now: Instant,
    ) -> Vec<Notice> {
        let cooldown = Duration::from_secs(u64::from(cfg.cooldown_minutes) * 60);
        let mut out = Vec::new();
        for (i, rule) in cfg.rules.iter().enumerate().filter(|(_, r)| r.enabled) {
            for m in metrics.iter().filter(|m| config::pattern_matches(&rule.metric, &m.id)) {
                let gate = self.gates.entry((i, m.id.clone())).or_default();
                let over = values.get(&m.id).is_some_and(|v| match rule.direction {
                    Direction::Above => *v >= rule.threshold,
                    Direction::Below => *v <= rule.threshold,
                });
                if !over {
                    gate.held_since = None;
                    continue;
                }
                let since = *gate.held_since.get_or_insert(now);
                let cooled = gate.last_sent.is_none_or(|t| now.duration_since(t) >= cooldown);
                if now.duration_since(since) >= SUSTAINED && cooled {
                    gate.last_sent = Some(now);
                    let v = values[&m.id];
                    let word = if rule.direction == Direction::Above { "above" } else { "below" };
                    out.push(Notice {
                        title: format!("{} is {}", m.name, format(v, m.unit)),
                        body: format!("It has been {word} {} for a while.", format(rule.threshold, m.unit)),
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use andromeda_core::config::AlertRule;
    use andromeda_core::metrics::{Group, Unit};

    fn setup() -> (config::Alerts, Vec<MetricInfo>) {
        let cfg = config::Alerts {
            rules: vec![AlertRule { metric: "gpu.*.temp".into(), direction: Direction::Above, threshold: 90.0, enabled: true }],
            cooldown_minutes: 15,
        };
        let metrics = vec![
            MetricInfo::new("gpu.card1.temp", "Radeon temperature", Unit::Celsius, Group::Gpu),
            MetricInfo::new("gpu.nvidia0.temp", "RTX temperature", Unit::Celsius, Group::Gpu),
        ];
        (cfg, metrics)
    }

    fn vals(a: f64, b: f64) -> HashMap<String, f64> {
        HashMap::from([("gpu.card1.temp".into(), a), ("gpu.nvidia0.temp".into(), b)])
    }

    #[test]
    fn fires_only_after_sustained_and_respects_cooldown() {
        let (cfg, m) = setup();
        let mut a = Alerts::default();
        let t0 = Instant::now();
        assert!(a.check(&cfg, &m, &vals(95.0, 50.0), t0).is_empty());
        assert!(a.check(&cfg, &m, &vals(95.0, 50.0), t0 + Duration::from_secs(6)).is_empty());
        let fired = a.check(&cfg, &m, &vals(95.0, 50.0), t0 + Duration::from_secs(12));
        assert_eq!(fired.len(), 1);
        assert!(fired[0].title.starts_with("Radeon temperature is 95"));
        // Still hot, but inside the cooldown.
        assert!(a.check(&cfg, &m, &vals(95.0, 50.0), t0 + Duration::from_secs(60)).is_empty());
        assert_eq!(a.check(&cfg, &m, &vals(95.0, 50.0), t0 + Duration::from_secs(12 + 15 * 60)).len(), 1);
    }

    #[test]
    fn a_dip_resets_the_stretch() {
        let (cfg, m) = setup();
        let mut a = Alerts::default();
        let t0 = Instant::now();
        a.check(&cfg, &m, &vals(95.0, 50.0), t0);
        a.check(&cfg, &m, &vals(80.0, 50.0), t0 + Duration::from_secs(8));
        assert!(a.check(&cfg, &m, &vals(95.0, 50.0), t0 + Duration::from_secs(13)).is_empty());
    }

    #[test]
    fn disabled_rules_and_missing_values_stay_quiet() {
        let (mut cfg, m) = setup();
        let mut a = Alerts::default();
        let t0 = Instant::now();
        a.check(&cfg, &m, &HashMap::new(), t0);
        assert!(a.check(&cfg, &m, &HashMap::new(), t0 + Duration::from_secs(20)).is_empty());
        cfg.rules[0].enabled = false;
        a.check(&cfg, &m, &vals(99.0, 99.0), t0);
        assert!(a.check(&cfg, &m, &vals(99.0, 99.0), t0 + Duration::from_secs(20)).is_empty());
    }
}
