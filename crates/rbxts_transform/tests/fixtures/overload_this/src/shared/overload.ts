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
