//! `util/evaluation`, the part over emitted Luau alone (`effects.ts`, `forEachChild`, reading `facts.ts`): what
//! running a Luau tree may do, and whether two evaluations can be reordered. roblox-ts's `ensureTransformOrder` and
//! its ordered property calls decide with it; the compiler re-exports it as `rbxts_transform::util::evaluation`, and
//! plugins get the non-JSX functions through `rbxts_plugin::effects`.

pub mod effects;
pub mod facts;
pub mod node;
