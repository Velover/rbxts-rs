// import_named reuses a named import of the same specifier, also under another local name
import { u, t as tt } from "./impa";
export const x = [u, tt, probeImportFrom("./impa", "u"), probeImportFrom("./impa", "t")];
