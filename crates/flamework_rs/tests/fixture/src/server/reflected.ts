import { Reflect } from "@flamework-experimental/core";

interface Shape {
	size: number;
}

/**
 * A user decorator that asks for more metadata than the core decorators do.
 *
 * @metadata reflect identifier flamework:parameters flamework:parameter_names flamework:implements
 */
export function Tracked() {
	return (ctor: object) => {
		Reflect.defineMetadata(ctor, "tracked", true);
	};
}

/** Requested per member. @metadata reflect flamework:type flamework:return_type flamework:parameters */
export function Field() {
	return (target: object, key: string) => {
		print(target, key);
	};
}

@Tracked()
export class Measured<T> implements Shape {
	size = 0;

	@Field()
	shape: Shape = { size: 1 };

	constructor(
		readonly count: number,
		readonly generic: T,
		readonly anything: defined,
		{ size }: Shape,
	) {
		print(size);
	}

	@Field()
	measure(by: Measured<string>): Shape {
		return { size: by.size };
	}
}
