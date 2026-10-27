// Overloads whose candidates disagree on `this`. TypeScript 5 gives a function expression argument the `this` of the
// first overload it checks the function against in full, even when that overload is then rejected.
interface Foo {
	n: number;
}

// the first overload is checked and rejected on the return type; the function keeps its `this: void`
export function over(cb: (this: void) => number): void;
export function over(cb: (this: Foo) => void): void;
export function over(cb: Callback) {}

over(function () {});
over(function (this: void) {});
over((function () {}));

// the first overload takes it: `this: Foo`
export function first(cb: (this: Foo) => void): void;
export function first(cb: (this: void) => number): void;
export function first(cb: Callback) {}

first(function () {});

// another argument rules the first overload out before the function is checked: the second one's `this`
export function byArg(x: string, cb: (this: void) => void): void;
export function byArg(x: number, cb: (this: Foo) => void): void;
export function byArg(x: unknown, cb: Callback) {}

byArg(1, function () {});
byArg("a", function () {});

// an arrow function argument is left out too (context sensitive in both), the function after it is the one decided
export function withArrow(a: (x: number) => void, cb: (this: void) => number): void;
export function withArrow(a: (x: number) => void, cb: (this: Foo) => void): void;
export function withArrow(a: Callback, cb: Callback) {}

withArrow((x) => {}, function () {});

// a method call: the overloads of a method
interface Holder {
	take(cb: (this: void) => number): void;
	take(cb: (this: Foo) => void): void;
}
declare const holder: Holder;
holder.take(function () {});

// an arrow argument before the function passes the overload's first pass but not its full check: the function is
// checked against the second overload in full
export function arrowFirst(a: (x: number) => string, cb: (this: void) => void): void;
export function arrowFirst(a: (x: number) => number, cb: (this: Foo) => void): void;
export function arrowFirst(a: Callback, cb: Callback) {}
arrowFirst((x) => x, function () {});

// generic overloads
export function gen<T>(v: T, cb: (this: void) => number): void;
export function gen<T>(v: T, cb: (this: Foo) => void): void;
export function gen(v: unknown, cb: Callback) {}
gen(1, function () {});

// a spread argument
export function spr(cb: (this: void) => number, ...rest: number[]): void;
export function spr(cb: (this: Foo) => void, ...rest: number[]): void;
export function spr(cb: Callback, ...rest: number[]) {}
const nums = [1, 2];
spr(function () {}, ...nums);

// explicit type arguments
export function targ<T>(cb: (this: void) => number, v?: T): void;
export function targ<T>(cb: (this: Foo) => void, v?: T): void;
export function targ(cb: Callback, v?: unknown) {}
targ<number>(function () {});

// an optional chain
interface MaybeHolder {
	take(cb: (this: void) => number): void;
	take(cb: (this: Foo) => void): void;
}
declare const maybe: MaybeHolder | undefined;
maybe?.take(function () {});

// an object literal method in another argument
export function objArg(o: { m(): void }, cb: (this: void) => number): void;
export function objArg(o: { m(): void }, cb: (this: Foo) => void): void;
export function objArg(o: unknown, cb: Callback) {}
objArg({ m() {} }, function () {});

// `new` with construct overloads
declare const Ctor: {
	new (cb: (this: void) => number): object;
	new (cb: (this: Foo) => void): object;
};
export const made = new Ctor(function () {});
