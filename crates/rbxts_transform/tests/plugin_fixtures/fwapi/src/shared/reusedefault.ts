// a named import beside a default one is reused; the default import itself is not (`{ default as … }` is)
import D, { named } from "./withdefault";
import { default as Dflt } from "./withdefault";
export const x = [D, Dflt, named, probeImportFrom("./withdefault", "named"), probeImportFrom("./withdefault", "default")];
