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

/** @probe self */
declare function probeSelf(a: number): number;

/** A chain of calls the probe claims with `compile_expr` of the call itself. */
interface Chain {
	/** @probe self */
	s(a: number): number;
	inner: Chain;
}
declare const maybeChain: Chain | undefined;
declare function getChain(): Chain;
declare function sideEffect(): number;

// Declined after compile_expr of the call, a hoist, or a prereq: each an internal error (used by the tests only).

/** @probe declineafter */
declare function probeDeclineAfter(a: number): number;

/** @probe hoistdecline */
declare function probeHoistDecline(): number;

/** @probe prereqdecline */
declare function probePrereqDecline(): number;

/** A marked base class, for a `super.probeMethod()` call (used by the tests only). */
declare class MarkedBase {
	readonly __probeMarker: true;
	probeMethod(value: number): number;
}

/** Compiles its argument, then declines: an internal error when the argument's own claim hoisted (tests only). */
/** @probe argdecline */
declare function probeArgDecline(a: unknown): unknown;

/** `void probeVoidHoist` is declined by the expression fallback after a hoist: an internal error (tests only). */
declare const probeVoidHoist: number;

/** Decorators lowered with `compile_expr(call.node)`. */
/** @probe decoself */
declare function probeDecoSelf(label?: string): (ctor: object) => void;
/** @probe decoself */
declare function probeDecoSelfPlain(ctor: object): void;

/** Declare a local named `_hoisted` / `impl` when the compiler approves the name, then compile the argument. */
/** @probe approve */
declare function probeApprove<T>(v: T): T;
/** @probe approveimpl */
declare function probeApproveImpl<T>(v: T): T;

/** Compiles its receiver inside a capture, drops the statements and declines (tests only). */
interface CapChain {
	/** @probe caprecv */
	cr(a: number): number;
}
declare const capMap: Record<string, CapChain>;

/** `void void probeVoidSelf`: the outer hook compiles its own node and declines (tests only). */
declare const probeVoidSelf: number;

/** The claimed node through compile_in_order (a `new`, an eager and a lazy chain call, an expression hook). */
/** @probe inorderself */
declare class ProbeInOrder {
	constructor(a: number);
}
/** @probe inorderself */
declare function probeInOrderFn(a: number): number;
interface InOrderChain {
	/** @probe inorderself */
	io(a: number): number;
}
declare const maybeInOrder: InOrderChain | undefined;
declare function getInOrder(): InOrderChain;
declare const probeVoidInOrder: number;

/** A late string or mark_import_used, then declined: each an internal error (tests only). */
/** @probe latedecline */
declare function probeLateDecline(): string;
/** @probe markdecline */
declare function probeMarkDecline(a: unknown): unknown;

/** An expression-kind hook (probe-exprcall) wraps these calls; the probe's call hook still claims them inside. */
/** @probe self */
declare function probeExprSelf(a: number): number;
/** @probe inorderself */
declare function probeExprInOrder(a: number): number;
/** @probe new */
declare class ProbeExprWidget {
	constructor(size: number);
}

/** The claimed call compiled twice (tests only). */
/** @probe twice */
declare function probeTwice(a: number): number;

/** `probeStmtSelf;`: a statement hook compiles its own statement (tests only). */
declare const probeStmtSelf: number;
