//! Which `t` the guards use (`TransformState.getGuardLibrary`): `@rbxts/t`, or core's own copy through
//! `@flamework-experimental/core/out/prelude` when the project's `@rbxts/t` is not the one core was built with.

use rbxts_plugin::Cx;

use crate::node_path;
use crate::packages::{CORE_PACKAGE, FLAMEWORK_SCOPE};

/// The guard library's module specifier for the file being transformed, with the warning the TS gives when it
/// cannot tell (`None` when there is nothing to warn about).
///
/// The TS decides once per build and keeps a successful answer; the answer depends only on module resolution, so
/// this asks again per guard and gets the same answer for every file of a project.
pub(crate) fn guard_library(cx: &Cx<'_>, package_name: &str) -> (String, Option<&'static str>) {
    const T: &str = "@rbxts/t";

    // Inside the Flamework packages themselves there is nothing to reconcile.
    if package_name.starts_with(&format!("{FLAMEWORK_SCOPE}/")) {
        return (T.to_owned(), None);
    }

    let file = cx.file().source;
    let Some(core) = cx.resolve_module(CORE_PACKAGE, file) else {
        return (T.to_owned(), Some("Flamework core was not found, guard generation may not work."));
    };
    let file_guard = cx.resolve_module(T, file).map(|m| m.file_name);
    let core_guard = cx
        .program()
        .get_source_file(&core.file_name)
        .and_then(|core_file| cx.resolve_module(T, core_file))
        .map(|m| m.file_name);
    if file_guard == core_guard {
        // core and the project use the same @rbxts/t.
        return (T.to_owned(), None);
    }

    let nested = node_path::join(&[&node_path::dirname(&core.file_name), "../node_modules/@rbxts/t"]);
    match core_guard {
        Some(core_guard) if node_path::is_path_descendant_of(&core_guard, &nested) => {
            (format!("{CORE_PACKAGE}/out/prelude"), None)
        }
        _ => (T.to_owned(), Some("Valid `@rbxts/t` was not found, guard generation may not work.")),
    }
}
