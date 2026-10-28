// Object rest types (`const { a, ...rest } = source`): TypeScript 5's `getRestType` lists the source's properties in
// its order, the destructured ones left out; TypeScript 7 sorts them by declaration.

// The spread's operand is declared after it, so declaration order differs from the spread's.
const spreadSource = { a: 1, ...late, c: 3 };
declare const late: Late;
interface Late {
	b: number;
}

/** A rest of a spread (right side first). TS5: b, a. */
const restOfSpread = () => {
	const { c: _c, ...rest } = spreadSource;
	return rest;
};
export type RestOfSpread = ReturnType<typeof restOfSpread>;
export declare const restOfSpreadValue: RestOfSpread;

interface RestA {
	x: number;
	y: number;
}
interface RestB extends RestA {
	y: number;
	z: number;
}
declare const restB: RestB;

/** A rest of an interface redeclaring its base's member (own members first). TS5: y, x. */
const restOfInterface = () => {
	const { z: _z, ...rest } = restB;
	return rest;
};
export type RestOfInterface = ReturnType<typeof restOfInterface>;
export declare const restOfInterfaceValue: RestOfInterface;

/** A rest of a parameter. TS5: y, x. */
const restOfParameter = ({ z: _z, ...rest }: RestB) => rest;
export type RestOfParameter = ReturnType<typeof restOfParameter>;
export declare const restOfParameterValue: RestOfParameter;

interface Holder {
	inner: RestB;
	other: number;
}
declare const holder: Holder;

/** A rest in a nested pattern: of `inner`'s type. TS5: z, x. */
const restOfNested = () => {
	const {
		inner: { y: _y, ...rest },
	} = holder;
	return rest;
};
export type RestOfNested = ReturnType<typeof restOfNested>;
export declare const restOfNestedValue: RestOfNested;

/** A rest of an optional source (`undefined` left out). TS5: y, x. */
const restOfOptional = (source?: RestB) => {
	const { z: _z, ...rest } = source ?? restB;
	return rest;
};
export type RestOfOptional = ReturnType<typeof restOfOptional>;
export declare const restOfOptionalValue: RestOfOptional;

class RestBase {
	k1 = 1;
	k2 = 2;
	method() {}
}
class RestDerived extends RestBase {
	k3 = 3;
	k1 = 4;
}
declare const derived: RestDerived;

/** A rest of a class instance: no methods, own members first. TS5: k3, k1, k2. */
const restOfClass = () => {
	const { ...rest } = derived;
	return rest;
};
export type RestOfClass = ReturnType<typeof restOfClass>;
export declare const restOfClassValue: RestOfClass;

/** A rest of an instantiated generic interface. TS5: y, w, x. */
interface GenericB<T> extends RestA {
	y: number;
	w: T;
	v: T;
}
declare const genericB: GenericB<string>;
const restOfGeneric = () => {
	const { v: _v, ...rest } = genericB;
	return rest;
};
export type RestOfGeneric = ReturnType<typeof restOfGeneric>;
export declare const restOfGenericValue: RestOfGeneric;

interface UnionP extends RestA {
	y: number;
	p: number;
}
interface UnionQ extends RestA {
	x: number;
	q: number;
}
declare const unionSource: UnionP | UnionQ;

/** A rest of a union: one rest per member. TS5 (`UnionP`'s): p, x. */
const restOfUnion = () => {
	const { y: _y, ...rest } = unionSource;
	return rest;
};
export type RestOfUnionP = Extract<ReturnType<typeof restOfUnion>, { p: number }>;
export declare const restOfUnionPValue: RestOfUnionP;
export type RestOfUnionQ = Extract<ReturnType<typeof restOfUnion>, { q: number }>;
export declare const restOfUnionQValue: RestOfUnionQ;
