// The calls the probe plugin claims by symbol: `@probe <op>` in the JSDoc.

/** @probe hoist */
declare function probeHoist<T>(value: T): T;

/** @probe prereq */
declare function probePrereq<T>(value: T): T;

/** @probe order */
declare function probeOrder(...values: Array<unknown>): Array<unknown>;

/** @probe inject */
declare function probeInject(value: unknown, extra?: string, more?: unknown): void;

/** @probe swap */
declare function probeSwap(a: number, b: string): number;

/** @probe late */
declare function probeLate(): string;

/** @probe capture */
declare function probeCapture<T>(value: T): T;

/** @probe root */
declare function probeRoot(): void;

/** @probe decorator */
declare function probeDecorator(label?: string): (ctor: object) => void;

/** @probe new */
declare class ProbeWidget {
	constructor(size: number);
}

/** @probe boom */
declare function probeBoom(): void;

/** @probe fail */
declare function probeFail(): void;

/** @probe err */
declare function probeErr(): void;

/** Claimed by receiver: the type has the marker member. */
interface Marked {
	readonly __probeMarker: true;
	probeMethod(value: number): number;
	probed(value: number): number;
}

/** Not claimed: no marker. */
interface Unmarked {
	probeMethod(value: number): number;
}

declare const marked: Marked;
declare const maybeMarked: Marked | undefined;
declare const unmarked: Unmarked;

/** Claimed by callee marker. */
declare const callable: ((value: number) => number) & { readonly __probeCallee: true };

/** `void probeVoid` is claimed by the expression fallback. */
declare const probeVoid: number;

/** @probe facts {@link marked} */
declare function probeFacts(): string;
