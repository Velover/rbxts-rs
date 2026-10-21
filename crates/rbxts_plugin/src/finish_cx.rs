//! `FinishCx`: what [`crate::Plugin::finish`] can ask of the compiler (DESIGN §11.3, §14).
//!
//! Nothing here touches the disk at once: artifacts, removals and state files are queued and written in the
//! compiler's single write phase, only when the build has no errors (DESIGN §14 "Commit").

use std::path::Path;

use rbxts_project::ProjectServices;

use crate::{BuildKind, FileIndex, Utf16Pos, escape_luau_string};

/// The compiler's side of [`FinishCx`]. One is made per plugin and build, on the finishing thread.
///
/// **Paths.** `write_artifact`, `write_state_file`, `remove_artifact` and `watch_dir` take absolute paths, or paths
/// relative to the project directory (the tsconfig's directory, `project().data.project_path`). Artifacts and
/// state files must lie under the project directory, the `outDir` or the include path (`--includePath`); the
/// compiler fails the build, naming the plugin, for any other path, and for a path that is also a Luau output.
pub trait FinishBackend {
    /// The project services.
    fn project(&self) -> &ProjectServices;
    /// The kind of build.
    fn build_kind(&self) -> BuildKind;
    /// Every source file was transformed this build (see [`crate::SetupBackend::transforms_all_files`]).
    fn transforms_all_files(&self) -> bool;
    /// Sets the value of a late string of this plugin. `escaped` is already Luau-escaped
    /// ([`escape_luau_string`]): the renderer puts it between quotes as it is. A later call for the same key
    /// replaces the value. After `finish` returns, the compiler fails the build, naming the plugin and the key,
    /// if a key used by a current file (transformed this build or cached) has no value; values for keys no file
    /// uses are ignored. A cached file whose values changed is re-rendered (contract on
    /// [`crate::CxBackend::late_string`]).
    fn resolve_late(&mut self, key: &str, escaped: String);
    /// Queues an artifact, written in commit step 2 (after the Luau outputs) only if its bytes changed. The
    /// compiler remembers the artifact set: an artifact written last build but not this one is deleted in step 3.
    fn write_artifact(&mut self, path: &Path, bytes: Vec<u8>);
    /// Queues the plugin's persistent-state file (Flamework's `flamework.build`), written in commit step 4,
    /// before the compiler's own incremental state, so a crash never leaves the compiler's state newer.
    fn write_state_file(&mut self, path: &Path, bytes: Vec<u8>);
    /// Queues the deletion of a file the plugin owns but did not write this build and may not have written in
    /// this process (a stale artifact from an earlier toolchain or project type); commit step 3.
    fn remove_artifact(&mut self, path: &Path);
    /// Watches a directory recursively from now on (a glob root): an add, delete or rename under it starts a
    /// rebuild that re-runs `finish`, though no source file changed. The set is replaced every build.
    fn watch_dir(&mut self, path: &Path);
    /// A warning at a position of a file, printed with the build's diagnostics; does not fail the build.
    fn warn_at(&mut self, file: FileIndex, pos: Utf16Pos, message: &str, hint: Option<&str>);
    /// An informational message, shown with `--verbose`.
    fn info(&mut self, message: &str);
}

/// The context of [`crate::Plugin::finish`]; forwards to [`FinishBackend`] and Luau-escapes late strings.
///
/// **Paths.** `write_artifact`, `write_state_file`, `remove_artifact` and `watch_dir` take absolute paths, or paths
/// relative to the project directory (the tsconfig's directory, `project().data.project_path`). Artifacts and
/// state files must lie under the project directory, the `outDir` or the include path (`--includePath`); the
/// compiler fails the build, naming the plugin, for any other path, and for a path that is also a Luau output.
pub struct FinishCx<'a> {
    backend: &'a mut dyn FinishBackend,
}

impl<'a> FinishCx<'a> {
    /// Wraps the compiler's backend for one `finish` call.
    pub fn new(backend: &'a mut dyn FinishBackend) -> FinishCx<'a> {
        FinishCx { backend }
    }

    /// The project services ([`FinishBackend::project`]).
    pub fn project(&self) -> &ProjectServices {
        self.backend.project()
    }

    /// The kind of build ([`FinishBackend::build_kind`]).
    pub fn build_kind(&self) -> BuildKind {
        self.backend.build_kind()
    }

    /// Whether every file was transformed this build ([`FinishBackend::transforms_all_files`]).
    pub fn transforms_all_files(&self) -> bool {
        self.backend.transforms_all_files()
    }

    /// Gives the late string `key` (see [`crate::Cx::late_string`]) its value: `text` is plain text, which this
    /// method escapes for Luau. Every key used by a current file must be resolved ([`FinishBackend::resolve_late`]).
    pub fn resolve_late(&mut self, key: &str, text: &str) {
        self.backend.resolve_late(key, escape_luau_string(text));
    }

    /// Queues an artifact ([`FinishBackend::write_artifact`]).
    pub fn write_artifact(&mut self, path: impl AsRef<Path>, bytes: Vec<u8>) {
        self.backend.write_artifact(path.as_ref(), bytes);
    }

    /// Queues the persistent-state file ([`FinishBackend::write_state_file`]).
    pub fn write_state_file(&mut self, path: impl AsRef<Path>, bytes: Vec<u8>) {
        self.backend.write_state_file(path.as_ref(), bytes);
    }

    /// Queues a deletion ([`FinishBackend::remove_artifact`]).
    pub fn remove_artifact(&mut self, path: impl AsRef<Path>) {
        self.backend.remove_artifact(path.as_ref());
    }

    /// Watches a glob root ([`FinishBackend::watch_dir`]).
    pub fn watch_dir(&mut self, path: impl AsRef<Path>) {
        self.backend.watch_dir(path.as_ref());
    }

    /// A positioned warning ([`FinishBackend::warn_at`]).
    pub fn warn_at(&mut self, file: FileIndex, pos: Utf16Pos, message: &str, hint: Option<&str>) {
        self.backend.warn_at(file, pos, message, hint);
    }

    /// A verbose message ([`FinishBackend::info`]).
    pub fn info(&mut self, message: &str) {
        self.backend.info(message);
    }
}
