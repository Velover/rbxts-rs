/** @jsx createElement */
interface RenderedElement {
	tag: string;
}
function createElement(tag: unknown, props: unknown, ...children: Array<unknown>): RenderedElement {
	return { tag: "x" };
}
declare namespace createElement {
	namespace JSX {
		type Element = RenderedElement;
		interface IntrinsicElements {
			text: { value?: number };
		}
	}
}
function Comp(props: { a: number }): RenderedElement;
function Comp(props: { b: string }): RenderedElement;
function Comp(props: object): RenderedElement {
	return { tag: "c" };
}
export const e = <Comp a="x" />;
interface TwoWays {
	(props: { n: number }): RenderedElement;
	(props: { s: string }): RenderedElement;
}
declare const Both: TwoWays;
export const f = <Both n="x" />;
