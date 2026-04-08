//! Stable stderr diagnostics (`tril:<area>:`) for sync, network, persistence, and sealing.
//! User-facing stdout lines (for example the listener address) stay unchanged for CLI/E2E.

use std::fmt::Display;

pub fn line(area: &'static str, msg: impl Display) {
    eprintln!("tril:{area}: {msg}");
}
