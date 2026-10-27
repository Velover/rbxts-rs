// A generator with an explicit `this` parameter whose yield is context sensitive: TypeScript 7 takes the generator
// for context sensitive, TypeScript 5 does not
declare function g<T>(make: (this: void) => Generator<(x: T) => void>, value: T): T;
export const r = g(function* (this: void) {
	yield (x) => {
		const n: number = x;
	};
}, "s");

declare function h<T>(value: T, make: (this: void) => Generator<(x: T) => void>): T;
export const q = h("s", function* (this: void) {
	yield (x) => {
		const n: number = x;
	};
});
