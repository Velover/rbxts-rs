import { Flamework } from "@flamework-experimental/core";

// Types the guard builder refuses: each statement reports its own error.
class Thing {
	x = 1;
}

interface Box<T> {
	inner?: Box<Box<T>>;
}

interface Holder {
	thing: Thing;
}

declare const marker: unique symbol;

export const classGuard = Flamework.createGuard<Thing>();
export const nestedClass = Flamework.createGuard<Holder>();
export const twoIndexes = Flamework.createGuard<{ [key: string]: number; [index: number]: number }>();
export const twoIndexIntersection = Flamework.createGuard<{ [key: string]: number } & { [index: number]: number }>();
export const bigintGuard = Flamework.createGuard<bigint>();
export const objectGuard = Flamework.createGuard<object>();
export const neverGuard = Flamework.createGuard<never>();
export const symbolGuard = Flamework.createGuard<typeof marker>();
export const nominal = Flamework.createGuard<Part & Model>();
export const box = Flamework.createGuard<Box<number>>();

/** @metadata reflect flamework:guard */
function Guarded() {
	return (target: object, key: string) => {
		print(target, key);
	};
}

export class Metadata {
	@Guarded()
	thing?: Thing;
}

export function conditional<T>() {
	return Flamework.createGuard<T extends string ? number : boolean>();
}

/** No constraint to check against: refused, as the TS refuses it. */
export function unconstrained<T>() {
	return Flamework.createGuard<T>();
}

export function indexed<T extends { name: string }, K extends keyof T>() {
	return Flamework.createGuard<T[K]>();
}
