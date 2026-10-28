// only the file's leading run of import declarations is searched: an import after another statement is not reused
import { u } from "./impa";
export const first = u;
import { t } from "./impb";
export const x = [t, probeImportFrom("./impb", "t"), probeImportFrom("./impa", "u")];
