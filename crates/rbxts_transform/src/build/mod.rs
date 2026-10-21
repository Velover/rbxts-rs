//! Clean, incremental and watch builds (DESIGN §14): the build session and what it decides with.
//!
//! - [`session`]: [`BuildSession`], the lifecycle over one project across builds.
//! - [`graph`]: file versions, references and the invalidation rules (which files a build transforms).
//! - [`outputs`]: assets, the keep set and the clean of stale outputs (roblox-ts's `getProjectOutputs`).
//! - [`state`]: the incremental state on disk.
//! - [`parity`]: the watch-parity harness's driver (DESIGN §14 W1).

pub mod graph;
pub mod outputs;
pub mod parity;
pub mod session;
pub mod state;

pub use session::{BuildSession, BuildSummary};
