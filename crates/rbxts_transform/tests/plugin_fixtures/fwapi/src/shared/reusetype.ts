// type-only imports are not reused: `import type`, and a type-only element
import type { u } from "./impa";
import { type t } from "./impa";
type U = typeof u | typeof t;
export const x: Array<U | unknown> = [probeImportFrom("./impa", "u"), probeImportFrom("./impa", "t")];
