import { Flamework } from "@flamework-experimental/core";
import { describe, fieldInfo, globOf, memberCount, pathOf, transformerVersion } from "./markers";

// Paths and globs.
export const commands = pathOf("src/server/commands");
export const commandGlob = globOf("src/server/commands/*.ts");
export const relativeGlob = globOf("../server/commands/*.ts");

// Flamework.env: set in the fixture's .env, a fallback, and nothing.
export const channel = Flamework.env("F2_FIXTURE_CHANNEL");
export const region = Flamework.env("F2_FIXTURE_REGION", "eu");
export const missing = Flamework.env("F2_FIXTURE_MISSING");

// Macro types of the fixture plugin.
interface Save {
	coins: number;
	readonly userId: number;
	nickname?: string;
	tags: string[];
	pair: [string, number];
}

export const fields = fieldInfo<Save>();
export const fieldsAgain = fieldInfo<Save>();
export const literal = describe<"hello">();
export const numberLiteral = describe<42>();
export const booleanLiteral = describe<true>();
export const kinds = describe<boolean[]>();
export const members = memberCount<"a" | "b" | "c">();
export const version = transformerVersion<void>();

export function later() {
	// A second use in another statement reuses the hoisted value.
	return fieldInfo<Save>();
}
