import { Flamework } from "@flamework-experimental/core";

// Arrays, tuples, maps, sets, records and index signatures.
export const numbers = Flamework.createGuard<number[]>();
export const readonlyNumbers = Flamework.createGuard<ReadonlyArray<number>>();
export const nested = Flamework.createGuard<string[][]>();
export const pair = Flamework.createGuard<[number, string]>();
export const optionalPair = Flamework.createGuard<[number, string?]>();
export const labeled = Flamework.createGuard<[id: number, name?: string]>();
export const rest = Flamework.createGuard<[number, ...string[]]>();
export const restOnly = Flamework.createGuard<[...string[]]>();
export const middleRest = Flamework.createGuard<[number, ...string[], boolean]>();
export const twoTrailing = Flamework.createGuard<[number, ...string[], boolean, number]>();
export const optionalRest = Flamework.createGuard<[number, string?, ...boolean[]]>();
export const readonlyTuple = Flamework.createGuard<readonly [number, number]>();
export const emptyTuple = Flamework.createGuard<[]>();
export const map = Flamework.createGuard<Map<string, number>>();
export const readonlyMap = Flamework.createGuard<ReadonlyMap<number, string[]>>();
export const set = Flamework.createGuard<Set<string>>();
export const readonlySet = Flamework.createGuard<ReadonlySet<Vector3>>();
export const record = Flamework.createGuard<Record<string, number>>();
export const keyedRecord = Flamework.createGuard<Record<"a" | "b", number>>();
export const numericRecord = Flamework.createGuard<Record<10 | 2, string>>();
export const indexString = Flamework.createGuard<{ [key: string]: boolean }>();
export const indexNumber = Flamework.createGuard<{ [index: number]: string }>();
export const indexAndProps = Flamework.createGuard<{ name: string; [key: string]: string }>();
export const partialRecord = Flamework.createGuard<Partial<Record<"x" | "y", number>>>();
