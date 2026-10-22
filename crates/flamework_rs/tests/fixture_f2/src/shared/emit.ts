import { Modding } from "@flamework-experimental/core";
import { constant, emit, keysOf, labelsOf, obfuscated, shuffled } from "./markers";

// `Modding.Emit` / `Many`: tuples, arrays of unions, objects (number keys included), literals.
export const tuple = emit<["a", 1, true, false, undefined]>();
export const single = emit<"only"[]>();
export const empty = emit<never[]>();
export const object = emit<{ name: "x"; 10: "ten"; nested: { flag: true; skipped: undefined }; "two words": 2 }>();
export const record = emit<Record<1 | 2, "n">>();
export const inspected = Modding.inspect<{ kind: "inspected"; list: ["a", "b"] }>();

function sample(first: string, second: number, ...rest: boolean[]) {
	return [first, second, rest];
}

// Labels: tuple element names, and parameter names.
export const labels = labelsOf<[name: string, age: number]>();
export const parameterNames = labelsOf<Parameters<typeof sample>>();
export const unlabelled = labelsOf<[string, number]>();

// Constant: hoisted to the file root inside a function, inline at the root.
export const atRoot = constant<{ shared: true }>();

export function invoke() {
	return [constant<{ shared: true; deep: { list: [1, 2] } }>(), constant<"plain">(), constant<{ again: 1 }>()];
}

// Obfuscation intrinsics: with obfuscation off, the keys and order stay.
export const keys = keysOf<{ playerJoined: 1; playerLeft: 2; 3: "number key" }>();
export const order = shuffled<["a", "b", "c"]>();
export const name = obfuscated();
