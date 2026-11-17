// SPDX-License-Identifier: GPL-3.0-or-later
//! Loads and saves `~/.config/io.github.incept0s.Andromeda/config.toml`.
//! A broken file is kept aside (`config.toml.bad`) instead of overwritten,
//! so a typo in a hand edit never silently loses the user's settings.

use std::fs;
use std::path::PathBuf;

use andromeda_core::config::Config;

pub const APP_ID: &str = "io.github.incept0s.Andromeda";

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join(APP_ID).join("config.toml"))
}

pub fn load() -> Config {
    let Some(p) = path() else {
        return Config::default();
    };
    let Ok(text) = fs::read_to_string(&p) else {
        return Config::default();
    };
    match toml::from_str::<Config>(&text) {
        Ok(mut c) => {
            c.sanitize();
            c
        }
        Err(e) => {
            eprintln!(
                "andromedad: {} is invalid ({e}); keeping it as config.toml.bad",
                p.display()
            );
            let _ = fs::rename(&p, p.with_extension("toml.bad"));
            Config::default()
        }
    }
}

/// Writes atomically: a crash mid-write leaves the old file intact.
pub fn save(c: &Config) -> std::io::Result<()> {
    let p = path().ok_or_else(|| std::io::Error::other("no HOME"))?;
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(c).map_err(std::io::Error::other)?;
    let tmp = p.with_extension("toml.tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, p)
}
