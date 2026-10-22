import { Flamework } from "@flamework-experimental/core";

// Instance types, with children from intersections and interfaces.
interface Character extends Model {
	Head: Part;
	HumanoidRootPart: BasePart;
	Animate: LocalScript & { idle: Animation };
}

export const part = Flamework.createGuard<Part>();
export const instance = Flamework.createGuard<Instance>();
export const basePart = Flamework.createGuard<BasePart>();
export const modelWithHead = Flamework.createGuard<Model & { Head: Part; Humanoid: Humanoid }>();
export const character = Flamework.createGuard<Character>();
export const partOrModel = Flamework.createGuard<Part | Model>();
export const optionalPart = Flamework.createGuard<Part | undefined>();
export const parts = Flamework.createGuard<Part[]>();
