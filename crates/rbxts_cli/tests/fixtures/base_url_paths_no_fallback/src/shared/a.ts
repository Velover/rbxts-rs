// `baseUrl` with `paths` (tests/cli.rs): a name a `paths` pattern matches but does not resolve does not fall back
// to `baseUrl` (TS2307), though `src/@lib/fallback.ts` exists.
import { m } from "shared/mod";
import { t } from "@lib/thing";
import { f } from "@lib/fallback";

export const a = m + t + f;
