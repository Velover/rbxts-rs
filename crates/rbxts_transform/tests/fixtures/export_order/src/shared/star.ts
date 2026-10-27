// `export *`: the module's own exports first (functions first), then the names the star brings, in its order.
export const own = 1;
export * from "./starred";
export function ownFn() {
	return own;
}
