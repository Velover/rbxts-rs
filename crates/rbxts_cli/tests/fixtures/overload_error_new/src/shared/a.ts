class C {
	constructor(a: number, b: string);
	constructor(a: string, b: number);
	constructor(a: unknown, b: unknown) {}
}
// overload 1 fails on the second argument, overload 2 on the first
new C(1, 1);
const ns = { D: C };
new ns.D(1, 1);
declare function f(a: number, b: string): void;
declare function f(a: string, b: number): void;
f(1, 1);
(f)(1, 1);
export {};
