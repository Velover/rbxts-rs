//! Test plugins for the compiler side of the plugin seam (DESIGN §11), registered by `rbxts_transform`'s plugin
//! tests and by `rbxts-bench build --plugin <name>`.
//!
//! - [`stamp`]: the `rbxts_plugin` crate's own test plugin (`crates/rbxts_plugin/tests/support/stamp.rs`, included
//!   by path so there is one source): JSDoc-tagged calls become strings or late strings, with a tsconfig entry, an
//!   env input, an artifact and a state file.
//! - [`probe`]: exercises every compiler primitive S1 offers: calls by symbol, by receiver and by callee marker,
//!   `new` and decorator claims, `compile_call` slots, prereqs, hoists, captures, imports and import elision, the
//!   class hook, the by-kind fallbacks, late strings, artifacts, and (in its failure modes) the phase guards,
//!   panics and the contracts the compiler enforces.

#[path = "../../rbxts_plugin/tests/support/stamp.rs"]
pub mod stamp;

pub mod probe;

use rbxts_plugin::erased::{DynPlugin, erase};

/// The plugin names `by_name` knows, for usage messages.
pub const NAMES: &[&str] = &[
    "stamp",
    "probe",
    "probe-b",
    "probe-panic",
    "probe-refuse",
    "probe-unresolved",
    "probe-fail",
    "probe-err",
    "probe-artifact-outside",
];

/// A test plugin by name (`rbxts-bench build --plugin <name>`).
pub fn by_name(name: &str) -> Option<Box<dyn DynPlugin>> {
    use probe::{Mode, Probe};
    Some(match name {
        "stamp" => erase(stamp::Stamp),
        "probe" => erase(Probe::new("probe", Mode::Normal)),
        "probe-b" => erase(Probe::new("probe-b", Mode::Normal)),
        "probe-panic" => erase(Probe::new("probe", Mode::Panic)),
        "probe-refuse" => erase(Probe::new("probe", Mode::Refuse)),
        "probe-unresolved" => erase(Probe::new("probe", Mode::Unresolved)),
        "probe-fail" => erase(Probe::new("probe", Mode::Fail)),
        "probe-err" => erase(Probe::new("probe", Mode::ErrWithoutError)),
        "probe-artifact-outside" => erase(Probe::new("probe", Mode::ArtifactOutside)),
        _ => return None,
    })
}
