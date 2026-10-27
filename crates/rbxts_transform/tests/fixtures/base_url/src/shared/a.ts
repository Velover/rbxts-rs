// `baseUrl` (tests/fixtures.rs, `base_url`): a name only `baseUrl` resolves, a `paths` pattern whose substitution is
// relative to `baseUrl`, and a declaration that names a type of a module it does not import.
import { m } from "shared/mod";
import { t, getThing } from "@lib/thing";

export const a = m + t;
export const thing = getThing();
