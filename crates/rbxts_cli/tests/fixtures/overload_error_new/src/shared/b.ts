class C {
	constructor(a: number);
	constructor(a: string);
	constructor(a: unknown) {}
}
// both overloads fail on the first argument: one place
new C(true);
interface Ctor {
	new (a: number): object;
	new (a: string): object;
}
declare const Made: Ctor;
new Made(true);
export {};
