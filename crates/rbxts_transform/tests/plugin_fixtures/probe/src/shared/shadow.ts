import { impl, other } from "./runtime";

export const a = impl("a", other);
// `impl` would mean the parameter or the local here: the import is not reused
export function shadowed(impl: string) {
	return probeSwap(3, impl);
}
export function withLocal() {
	const impl = "x";
	return probeSwap(4, impl);
}
