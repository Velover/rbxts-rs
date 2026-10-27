// TS2769 for generic overloads: each candidate is the one inference instantiated
interface Foo {
	n: number;
}
export function gen<T>(v: T, cb: (x: T) => string): void;
export function gen<T extends Foo>(v: T, cb: (x: T) => number): void;
export function gen(v: unknown, cb: Callback) {}
gen(1, (x) => true);
