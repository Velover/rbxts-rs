import { Flamework } from "@flamework-experimental/core";

// Recursive types (the specs' recursiveGuards types), checked through functions that call themselves.
export interface Tree {
	value: number;
	kids: Tree[];
}

export interface Chain {
	value: number;
	next?: Chain;
}

export interface Menu {
	label: string;
	items: Map<string, Menu>;
}

export type Json = string | number | boolean | Json[] | { [key: string]: Json };

export interface Directory {
	name: string;
	files?: Entry[];
}

export interface Entry {
	name: string;
	parent: Directory;
}

/** A union whose members both go into the same children. */
export type Node = { kind: "folder"; children?: Node[] } | { kind: "group"; children?: Node[] };

/** The same without a discriminant. */
export type Either = { x: number; kids: Either[] } | { y: string; kids: Either[] };

export type AliasTree = { value: number; kids: AliasTree[] };

export const tree = Flamework.createGuard<Tree>();
export const chain = Flamework.createGuard<Chain>();
export const menu = Flamework.createGuard<Menu>();
export const json = Flamework.createGuard<Json>();
export const directory = Flamework.createGuard<Directory>();
export const node = Flamework.createGuard<Node>();
export const either = Flamework.createGuard<Either>();
export const aliasTree = Flamework.createGuard<AliasTree>();
export const forest = Flamework.createGuard<Tree[]>();
export const optionalTree = Flamework.createGuard<Tree | undefined>();
export const treeTuple = Flamework.createGuard<[Tree, ...Chain[]]>();

/** Past the dedup limit of 3: `Tree`'s guard is declared once, outside the guard that uses it. */
export const orchard = Flamework.createGuard<{ a: Tree; b: Tree; c: Tree; chain: Chain }>();
