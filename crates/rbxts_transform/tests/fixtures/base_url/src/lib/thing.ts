import { Thing } from "./types";

// functions before variables: TypeScript 5 lists exported functions first, TypeScript 7 in source order
export function getThing(): Thing {
	return { n: 1 };
}
export const t = 2;
