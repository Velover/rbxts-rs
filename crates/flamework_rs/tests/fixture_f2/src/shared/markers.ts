import { Modding } from "@flamework-experimental/core";

// One user macro per marker kind; the call sites are in the other files.

/** @metadata macro */
export function idOf<T>(id?: Modding.Target.Id<T>): string {
	return id!;
}

/** @metadata macro */
export function textOf<T>(text?: Modding.Target.Text<T>): string {
	return text!;
}

/** @metadata macro */
export function dependencyOf<T>(info?: Modding.Target.Dependency<T>): Modding.DependencyInfo {
	return info!;
}

/** @metadata macro */
export function conciseOf<T>(info?: Modding.Target.DependencyConcise<T>): unknown {
	return info!;
}

/** @metadata macro */
export function where(
	line?: Modding.Caller.Line,
	character?: Modding.Caller.Character,
	width?: Modding.Caller.Width,
	uuid?: Modding.Caller.Uuid,
	text?: Modding.Caller.Text,
) {
	return [line, character, width, uuid, text];
}

/** @metadata macro */
export function callsite(uuid?: Modding.Caller.Uuid): string {
	return uuid!;
}

/** @metadata macro */
export function obfuscated(name?: Modding.Target.Obfuscate<"playerJoined", "remotes">): string {
	return name!;
}

/** @metadata macro */
export function labelsOf<T extends readonly unknown[]>(labels?: Modding.Target.Labels<T>): string[] | undefined {
	return labels;
}

/** @metadata macro */
export function emit<T>(value?: Modding.Emit<T>): T {
	return value!;
}

/** @metadata macro */
export function constant<T>(value?: Modding.Caller.Constant<Modding.Emit<T>>): T {
	return value!;
}

/** @metadata macro */
export function pathOf<T extends string>(_path: T, path?: Modding.Intrinsic<"path", [T], string[]>): string[] {
	return path!;
}

/** @metadata macro */
export function globOf<T extends string>(_glob: T, glob?: Modding.Intrinsic<"pathglob", [T], string>): string {
	return glob!;
}

/** @metadata macro */
export function keysOf<T>(value?: Modding.Intrinsic<"obfuscate-obj", [T, "remotes"], Record<string, unknown>>) {
	return value!;
}

/** @metadata macro */
export function shuffled<T>(value?: Modding.Intrinsic<"shuffle-array", [T], unknown[]>) {
	return value!;
}

/** @metadata macro */
export function fieldInfo<T>(meta?: Modding.Intrinsic<"plugin", ["fieldInfo", T], unknown[]>): unknown[] {
	return meta!;
}

/** @metadata macro */
export function describe<T>(meta?: Modding.Intrinsic<"plugin", ["describe", T], string>): string {
	return meta!;
}

/** @metadata macro */
export function memberCount<T>(meta?: Modding.Intrinsic<"plugin", ["mustBeUnion", T], number>): number {
	return meta!;
}

/** @metadata macro */
export function transformerVersion<T>(meta?: Modding.Intrinsic<"plugin", ["transformerVersion", T], string>): string {
	return meta!;
}

/** A parameter that takes metadata or a value: the first marked member of the union generates. */
/** @metadata macro */
export function either<T>(id?: string | Modding.Target.Id<T>): string {
	return id!;
}

/** Padding: the generated parameter is the second one. */
/** @metadata macro */
export function second<T>(first?: string, id?: Modding.Target.Id<T>): string {
	return `${first}/${id!}`;
}

/** Inlined: the call is replaced by its generated value. */
/** @metadata macro {@link text intrinsic-inline} */
export declare function inlinedText<T>(text?: Modding.Target.Text<T>): string;

/** A literal argument is required. */
/** @metadata macro {@link options intrinsic-const} */
export function withOptions<T>(options: { name: string; size: number }, id?: Modding.Target.Id<T>) {
	return [options, id!] as const;
}

/** A macro method and a macro class. */
export class Registry {
	readonly entries = new Map<string, unknown>();

	/** @metadata macro */
	add<T>(value: unknown, id?: Modding.Target.Id<T>) {
		this.entries.set(id!, value);
		return this;
	}
}

/** @metadata macro */
export class Tagged<T> {
	constructor(public readonly id?: Modding.Target.Id<T>) {}
}

/** A decorator macro. */
/** @metadata macro */
export function Mark<T>(id?: Modding.Target.Id<T>) {
	return (target: object) => {
		print(target, id);
	};
}
