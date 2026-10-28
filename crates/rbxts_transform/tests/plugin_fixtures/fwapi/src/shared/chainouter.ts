// The outermost call of an optional chain offered before the chain is compiled (`calls_chain_outer`): a value taken
// replaces the whole chain (nothing of it is evaluated), a replaced callee keeps only the call's own `?.`.
declare const maybe: ChainTarget | undefined;
declare const target: ChainTarget;
declare function get(): ChainTarget | undefined;
declare function sideEffect(): number;

// o?.m()
export const i1 = maybe?.inline();
export const r1 = maybe?.rewrite(sideEffect());
// o.m?.()
export const i2 = target.inline?.();
export const r2 = target.rewrite?.(sideEffect());
// o?.m?.()
export const i3 = maybe?.inline?.();
export const r3 = get()?.rewrite?.(sideEffect());
// deeper chains
export const i4 = get()?.inner.inner?.inline();
export const r4 = maybe?.inner.rewrite(1);

export function statements() {
	maybe?.inline();
	target.rewrite?.(sideEffect());
	get()?.rewrite(2);
}

// declined: lowered as usual
export const d1 = maybe?.other(1);
export const d2 = maybe?.inner.other?.(2);
