import { callsite, where } from "./markers";

// Positions are UTF-16: "é" is one unit, "😀" two, as TypeScript counts them.
export const greeting = "héllo 😀"; export const here = where();

export const names = { "日本": callsite(), "😀": where(undefined, undefined, undefined, undefined) };

export function nested() {
	const value = "😀😀"; /* ünïcödé */ const at = where();
	return [value, callsite(), at];
}
