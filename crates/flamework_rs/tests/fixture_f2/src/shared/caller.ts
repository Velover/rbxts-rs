import { callsite, where } from "./markers";

// Caller metadata: line, character, width, uuid and the source text, in UTF-16.
export const atRoot = where();

export function inFunction() {
	return where(  );
}

export class Holder {
	readonly field = callsite();

	method() {
		return [callsite(), callsite()];
	}
}

export const multiline = where(
	undefined,
	undefined,
);

export const ids = [callsite(), callsite()];
