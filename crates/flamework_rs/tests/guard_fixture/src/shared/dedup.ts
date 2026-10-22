import { Flamework } from "@flamework-experimental/core";

// The dedup heuristic (`guardGenerationDedupLimit`, 3 here): types met three times are declared once.
interface Vec2 {
	x: number;
	y: number;
}

export const points = Flamework.createGuard<{ a: Vec2; b: Vec2; c: Vec2 }>();
export const nestedPoints = Flamework.createGuard<{
	a: { p: Vec2; q: Vec2 };
	b: { p: Vec2; q: Vec2 };
	c: { p: Vec2; q: Vec2 };
}>();
export const unions = Flamework.createGuard<{ a: string | number; b: string | number; c: string | number }>();
export const twice = Flamework.createGuard<{ a: Vec2; b: Vec2 }>();

export function inFunction() {
	return Flamework.createGuard<{ a: Vec2; b: Vec2; c: Vec2 }>();
}
