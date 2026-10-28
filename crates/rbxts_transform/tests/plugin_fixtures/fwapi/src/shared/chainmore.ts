// More of the outer offer (`calls_chain_outer`): a call TypeScript counts in an optional chain that roblox-ts's
// `flattenOptionalChain` cuts at a `!`; a `Callee::Method` replacement, with and without the call's own `?.`; and a
// send as Flamework's networking makes it, which reads the chain's base, runs the arguments, then reads the receiver
// chain again (`compile_call_with` with deferred slots).
declare const maybe: ChainTarget | undefined;
declare const target: ChainTarget;
declare const box: { ev: ChainTarget | undefined };
declare function get(): ChainTarget | undefined;
declare function sideEffect(): number;
// sets `box.ev` to undefined
declare function clearBox(): number;

// through a `!`
export const n1 = get()?.inner!.inline();
export const n2 = maybe?.inner!.rewrite(sideEffect());
export const n3 = get()?.inner!.inner.rewrite?.(sideEffect());
export const n4 = maybe?.inner!.inner!.inline();
// declined through a `!`: lowered as usual
export const n5 = get()?.inner!.other(sideEffect());

// Callee::Method
export const m1 = maybe?.methodop(sideEffect());
export const m2 = maybe?.inner.methodop?.(sideEffect());
export const m3 = target.methodop?.(1);
export function methods() {
	get()?.inner.methodop?.(2);
	maybe?.methodop(3);
}

// a send: the base, the arguments, the receiver again
export function send(make: () => number) {
	box.ev?.inner.resend(clearBox() + make());
	const sent = box.ev?.inner.resend(sideEffect());
	// declined: lowered as usual
	box.ev?.inner.other(clearBox() + make());
	return sent;
}
