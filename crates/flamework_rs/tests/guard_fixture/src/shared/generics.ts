import { Flamework } from "@flamework-experimental/core";

// Generic types, resolved through the checker; type parameters through their constraints.
interface Wrapper<T> {
	value: T;
	list: T[];
}

type Maybe<T> = T | undefined;
type Pair<A, B> = [A, B];
type Keyed<K extends string, V> = { [P in K]: V };

export const wrapped = Flamework.createGuard<Wrapper<number>>();
export const maybe = Flamework.createGuard<Maybe<string>>();
export const pairOf = Flamework.createGuard<Pair<string, Wrapper<boolean>>>();
export const keyed = Flamework.createGuard<Keyed<"left" | "right", Vector2>>();

export function constrained<T extends string>() {
	return Flamework.createGuard<T>();
}

export function genericObject<T extends { id: number }>() {
	return Flamework.createGuard<{ item: T; items: T[] }>();
}
