// TS2769 with three overloads tried, and a fourth ruled out by its arity
export function three(x: string): void;
export function three(x: number): void;
export function three(x: boolean, y?: string): void;
export function three(x: string, y: string, z: string): void;
export function three(x: unknown, y?: unknown, z?: unknown) {}
three(undefined as unknown as object);
declare const holder: {
	m(cb: (x: number) => number): void;
	m(cb: (x: string) => string): void;
};
holder.m((x: boolean) => x);
