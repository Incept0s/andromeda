// SPDX-License-Identifier: GPL-3.0-or-later
//! Prints every metric the registry finds on this machine, with a value.
//! Useful for bug reports: `cargo run -p andromeda-core --example probe`.

use std::path::Path;

use andromeda_core::metrics::format;
use andromeda_core::sensors::Sensors;

fn main() {
    let mut s = Sensors::discover(Path::new("/sys"), Path::new("/proc"));
    s.sample();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let values = s.sample();
    for m in s.metrics() {
        let v = values
            .iter()
            .find(|(id, _)| *id == m.id)
            .map(|(_, v)| format(*v, m.unit));
        println!(
            "{:<28} {:<26} {:?}/{:?}  {}",
            m.id,
            m.name,
            m.group,
            m.unit,
            v.unwrap_or("-".into())
        );
    }
}
