// Cx::ordered_element_call: `o[k](o, args)` built from compiled parts, as roblox-ts orders an element call
interface Ordered {
	m(a: number, b: number): number;
	n(a: number, b: number): number;
}
declare const o: Ordered;
declare function get(): Ordered;
declare function key(): "m" | "n";
declare let k: "m" | "n";
declare const x: number | undefined;
declare function f(n: number): number;
export const c1 = probeOrderedCall(o, k, x ?? f(1), f(2));
export const c2 = probeOrderedCall(get(), "m", f(3), x ?? 4);
export const c3 = probeOrderedCall(o, key(), 5, 6);
export const c4 = probeOrderedCall(o, k, f(7), (k = "n", 8));
