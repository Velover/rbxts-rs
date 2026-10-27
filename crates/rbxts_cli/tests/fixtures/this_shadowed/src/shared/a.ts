// TS2683 with its related information "An outer value of 'this' is shadowed by this container." on the function
// expression: TypeScript 5 spans its first token, TypeScript 7 the name it is assigned to
export const c1 = function () {
	return this;
};
export let c2: unknown;
c2 = function () {
	return this;
};
export const c3 = async function () {
	return this;
};
export const c4 = {
	inner: (function () {
		return this;
	})(),
};
export const c5 = function named() {
	return this;
};
