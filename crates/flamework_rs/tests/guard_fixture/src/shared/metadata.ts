import { Reflect } from "@flamework-experimental/core";
import type { Tree } from "./recursive";

// Class metadata guards: `flamework:guard`, `flamework:parameter_guards` and `flamework:return_guard`.

/** @metadata reflect flamework:guard flamework:parameter_guards flamework:return_guard */
export function Guarded() {
	return (target: object, key: string) => {
		print(target, key);
	};
}

/** @metadata reflect flamework:parameter_guards */
export function Checked() {
	return (ctor: object) => {
		Reflect.defineMetadata(ctor, "checked", true);
	};
}

interface Settings {
	volume: number;
	muted?: boolean;
	tags: string[];
}

@Checked()
export class Service {
	@Guarded()
	field = new Map<string, Vector3>();

	@Guarded()
	optionalField?: [number, ...string[]];

	@Guarded()
	tree?: Tree;

	constructor(
		readonly name: string,
		readonly settings: Settings,
		readonly count?: number,
	) {}

	@Guarded()
	method(a: number, b: Settings | undefined, ...rest: string[]): Settings {
		return b ?? { volume: a, tags: rest };
	}

	@Guarded()
	walk(tree: Tree): Tree[] {
		return tree.kids;
	}

	@Guarded()
	async promised(): Promise<number> {
		return 1;
	}

	@Guarded()
	nothing() {}
}

@Checked()
export class Empty {}

export class Holder<T extends string> {
	@Guarded()
	value!: T;

	@Guarded()
	run(item: T): T {
		return item;
	}
}
