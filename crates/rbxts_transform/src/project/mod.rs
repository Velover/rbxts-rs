//! Build steps of roblox-ts's `src/Project/functions` that run around the transform, one module per TypeScript
//! file.
//!
//! - [`copy_include`]: `copyInclude.ts` and `getIncludeFiles.ts`, the runtime library files copied into the
//!   project's include folder.

pub mod copy_include;
