import { Modding } from "@flamework-experimental/core";

/** @metadata macro */
export function hashOf(name?: Modding.Target.Hash<"playerJoined">, scoped?: Modding.Target.Hash<"playerJoined", "remotes">) {
	return [name, scoped];
}

// `Modding.Hash` is a fresh hash on every clean build (random in TS5, derived from a per-build seed here).
export const hashes = hashOf();
