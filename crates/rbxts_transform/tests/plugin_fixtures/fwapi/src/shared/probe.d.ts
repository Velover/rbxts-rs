/** @probe importfrom */
declare function probeImportFrom(path: string, name: string): unknown;

/** @probe latesynth */
declare function probeLateSynth(): string;

/** @probe inject */
declare function probeDecoInject(label: string, extra?: string, more?: unknown): (target: object, key?: string) => void;

/** @probe objemitted */
declare function probeObjEmitted<T>(literal: T): T & { w: number };

/** @probe importfrom */
declare function probeDecoImportFrom(path: string, name: string): (ctor: object) => void;

/** @probe importatclass */
declare function probeImportAtClass(path: string, name: string): unknown;

/** Accesses answered through `property_key_answer` with the element access spelling: `elem` as `k_elem`, `elemsame`
 * as itself. (`k_elem` is declared so that the TS reference, which rewrites `x.elem` to `x["k_elem"]`, checks.) */
interface ElemKeyed {
	readonly __probeKey: true;
	elem(x: number): number;
	k_elem(x: number): number;
	elemsame(x: number): number;
}

/** @probe deferred */
declare function probeDeferred(a: number, b: number, c: number): number;

/** The chain ops (`calls_chain_outer`): `inline()` is replaced by `"inlined"`, `rewrite(…)` by `Rewrite.impl(…)`. */
interface ChainTarget {
	readonly __probeChain: true;
	inline(): string;
	rewrite(value: number): unknown;
	other(value: number): number;
	inner: ChainTarget;
}

declare const Rewrite: { impl: (value: number) => unknown };

/** @probe orderedcall */
declare function probeOrderedCall(receiver: object, key: string, ...args: Array<number>): number;
