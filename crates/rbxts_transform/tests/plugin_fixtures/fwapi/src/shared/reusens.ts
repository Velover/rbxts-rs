// import_named reuses a user import as Flamework's TS transformer does (addFileImport): a namespace import is not
// reused (the plugin's import is a local of its own beside it)
import * as A from "./impa";
export const x = [A.u, probeImportFrom("./impa", "u"), probeImportFrom("./impa", "t")];
