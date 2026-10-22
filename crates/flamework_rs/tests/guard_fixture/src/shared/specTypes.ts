import { Flamework, Serialization } from "@flamework-experimental/core";

// The types of the specs' type checks (packages/specs/src/typeChecks/codecs.ts), as guards.
export interface Point {
	x: number;
	y: number;
}

export interface Owner {
	label: string;
}

export interface Entity {
	id: number;
	name: string;
	tags: string[];
	pos: Point;
	spot?: Vector3;
	flag: boolean;
	owner: Owner;
}

export type Shape = { kind: "circle"; r: number } | { kind: "rect"; w: number; h: number; label: string };

export const u8 = Flamework.createGuard<Serialization.Implicit.u8>();
export const varint = Flamework.createGuard<Serialization.varint>();
export const string8 = Flamework.createGuard<Serialization.string8>();
export const bytes = Flamework.createGuard<buffer>();
export const literals = Flamework.createGuard<"a" | "b" | "c">();
export const circle = Flamework.createGuard<{ kind: "circle"; r: number }>();
export const frame = Flamework.createGuard<CFrame>();
export const list = Flamework.createGuard<number[]>();
export const set = Flamework.createGuard<Set<string>>();
export const map = Flamework.createGuard<Map<string, number>>();
export const maybe = Flamework.createGuard<number | undefined>();
export const either = Flamework.createGuard<number | string>();
export const pair = Flamework.createGuard<[number, string]>();
export const points = Flamework.createGuard<Point[]>();
export const entity = Flamework.createGuard<Entity>();
export const shape = Flamework.createGuard<Shape>();
export const plastic = Flamework.createGuard<Enum.Material.Plastic>();
export const plasticOrWood = Flamework.createGuard<Enum.Material.Plastic | Enum.Material.Wood>();
export const plasticOrNumber = Flamework.createGuard<Enum.Material.Plastic | Enum.Material.Wood | number>();
export const moveArgs = Flamework.createGuard<[id: number, entity: Entity, flag: boolean]>();
export const manyArgs = Flamework.createGuard<[label: string, ...values: number[]]>();
