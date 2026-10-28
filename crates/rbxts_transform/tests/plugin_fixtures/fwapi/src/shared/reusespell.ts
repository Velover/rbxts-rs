// the module specifier must be the one asked for as written: another spelling of the same module is not reused
import { u } from "../shared/impa";
export const x = [u, probeImportFrom("./impa", "u")];
