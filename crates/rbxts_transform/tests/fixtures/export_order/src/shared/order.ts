// Export order (tests/fixtures.rs, `export_order`): TypeScript 5 binds a block's function declarations first, so the
// export table lists the exported functions first, then the other exports in source order; TypeScript 7's API sorts
// them in source order.
export const first = 1;
export class Klass {}
export function fn1() {
	return first;
}
export { first as aliased };
export enum Kind {
	A,
}
export const [d1, d2] = [1, 2];
export default function fallback() {
	return 0;
}
export function fn2() {
	return 2;
}
export namespace Space {
	export const inner = 1;
	export class InnerClass {}
	export function innerFn() {
		return inner;
	}
}
function hidden() {
	return 3;
}
export { hidden };
