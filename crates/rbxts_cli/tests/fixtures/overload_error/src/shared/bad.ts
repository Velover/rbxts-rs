interface Foo {
	n: number;
}
interface Bar {
	s: string;
}
// H: TS5 fixes `this: Foo` from overload 1, rejects it on the return type, and overload 2 refuses `this: Foo`: TS2769
export function bad(cb: (this: Foo) => number): void;
export function bad(cb: (this: Bar) => void): void;
export function bad(cb: Callback) {}
bad(function () {});
