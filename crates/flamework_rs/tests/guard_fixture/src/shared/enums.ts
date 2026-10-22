import { Flamework } from "@flamework-experimental/core";

// TypeScript enums and Roblox enums.
enum Color {
	Red,
	Green,
	Blue,
}

enum Mode {
	Fast = "fast",
	Slow = "slow",
}

const enum Direction {
	Up = 1,
	Down = 2,
}

export const color = Flamework.createGuard<Color>();
export const mode = Flamework.createGuard<Mode>();
export const direction = Flamework.createGuard<Direction>();
export const red = Flamework.createGuard<Color.Red>();
export const someColors = Flamework.createGuard<Color.Red | Color.Blue>();
export const optionalMode = Flamework.createGuard<Mode | undefined>();
export const material = Flamework.createGuard<Enum.Material>();
export const plastic = Flamework.createGuard<Enum.Material.Plastic>();
export const plasticOrWood = Flamework.createGuard<Enum.Material.Plastic | Enum.Material.Wood>();
export const plasticOrNumber = Flamework.createGuard<Enum.Material.Plastic | Enum.Material.Wood | number>();
export const sortOrder = Flamework.createGuard<Enum.SortOrder.Name | Enum.SortOrder.Custom | Enum.SortOrder.LayoutOrder>();
export const keyCode = Flamework.createGuard<Enum.KeyCode>();
export const optionalMaterial = Flamework.createGuard<Enum.Material | undefined>();
