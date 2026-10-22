import { Flamework } from "@flamework-experimental/core";

// Interfaces, type literals and their keys.
interface Point {
	x: number;
	y: number;
}

interface Named {
	name: string;
	tags?: string[];
}

interface Labeled extends Named {
	label: string;
	point: Point;
}

export const point = Flamework.createGuard<Point>();
export const literalObject = Flamework.createGuard<{ a: number; b?: string; c: Point }>();
export const nestedObject = Flamework.createGuard<{ inner: { deep: { value: number } } }>();
export const numericKeys = Flamework.createGuard<{ 10: string; 1.5: number; "two words": boolean; [-1]: Point }>();
export const ignoredFields = Flamework.createGuard<{ a: number; b: unknown; c: never }>();
export const emptyObject = Flamework.createGuard<{}>();
export const withMethod = Flamework.createGuard<{ run(): void; value: number }>();
export const intersection = Flamework.createGuard<Point & Named>();
export const unionOfObjects = Flamework.createGuard<{ kind: "a"; a: number } | { kind: "b"; b: string }>();
export const picked = Flamework.createGuard<Pick<Point, "x">>();
export const extended = Flamework.createGuard<Labeled>();
export const readonlyObject = Flamework.createGuard<Readonly<Point>>();
export const optionalObject = Flamework.createGuard<Point | undefined>();

// Inherited properties: TS5 lists the interface's own properties, then the inherited ones it does not redeclare.
namespace Shop {
	export interface Display {
		readonly DisplayName: string;
		readonly Description: string;
		readonly Icon: string;
		readonly Rarity: string;
	}
}

interface Potion extends Shop.Display {
	Type: string;
	Duration: string;
	Boosts?: { Xp?: number };
}

interface Requirement {
	readonly Type: string;
	readonly Amount: number;
}

interface BuyRequirement extends Requirement {
	readonly Amount: number;
	readonly ShovelName: string;
}

interface Display {
	readonly DisplayName: string;
	readonly Description: string;
	readonly Icon: string;
	readonly Rarity: string;
}

interface LocalPotion extends Display {
	Type: string;
	Duration: string;
}

export const potions = Flamework.createGuard<Map<string, Potion>>();
export const buyRequirement = Flamework.createGuard<BuyRequirement>();
export const localPotion = Flamework.createGuard<LocalPotion>();

// Inherited properties read before the guard is built: the order is still the declared one.
interface Card {
	readonly Title: string;
	readonly Body: string;
	readonly Icon: string;
	readonly Rarity: string;
}

interface PotionCard extends Card {
	Kind: string;
	Length: string;
}

export function describe(card: PotionCard) {
	return card.Rarity + card.Icon;
}

export const potionCard = Flamework.createGuard<PotionCard>();
