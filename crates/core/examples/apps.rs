// SPDX-License-Identifier: GPL-3.0-or-later
// Prints the busiest apps over one second: `cargo run -p andromeda-core --example apps`.
fn main() {
    let mut p = andromeda_core::procs::Procs::new(std::path::Path::new("/proc"));
    p.sample();
    std::thread::sleep(std::time::Duration::from_secs(1));
    for a in p.sample().iter().take(8) {
        println!(
            "{:>5.1}%  {:>6.0} MiB  {:>3}  {:<28} {}",
            a.cpu,
            a.memory / 1048576.0,
            a.processes,
            a.name,
            a.key
        );
    }
}
