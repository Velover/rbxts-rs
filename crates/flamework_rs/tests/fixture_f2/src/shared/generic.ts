import { Dependency, Flamework, Modding, Provider } from "@flamework-experimental/core";
import { conciseOf, dependencyOf, either, idOf, inlinedText, Mark, Registry, second, Tagged, textOf, withOptions } from "./markers";

@Provider()
export class Economy {}

export interface Shop {
	open(): void;
}

class LoggerImpl {
	log(message: string) {
		print(message);
	}
}

/** A dependency with metadata: what the provider gets is told where it is injected. */
export type Logger = Modding.Injectable<{ type: LoggerImpl; metadata: [Modding.Caller.Line, "logger", Modding.Target.Id<Shop>] }>;

@Provider()
export class Bank {
	constructor(
		readonly economy: Economy,
		readonly logger: Logger,
	) {}
}

// Generic markers.
export const economyId = idOf<Economy>();
export const shopText = textOf<Shop | undefined>();
export const economyDependency = dependencyOf<Economy>();
export const economyConcise = conciseOf<Economy>();
export const loggerConcise = conciseOf<Logger>();
export const loggerDependency = dependencyOf<Logger>();

// Core's macros: an inlined id, a rewritten call, dependencies.
export const inlined = Flamework.id<Bank>();
export const inlinedOwn = inlinedText<[Economy, string]>();
export const passedThrough = Flamework.id<Bank>("explicit" as never);
export function isShop(value: unknown) {
	return Flamework.implements<Shop>(value);
}
export function resolve() {
	return [Dependency<Economy>(), Dependency<Logger>(), Dependency<Economy>(undefined, idOf<Economy>())];
}

// Unions, padding, explicit arguments.
export const generated = either<Economy>();
export const explicit = either<Economy>("mine");
export const padded = second<Economy>();
export const paddedUndefined = second<Economy>(undefined);
export const options = withOptions<Shop>({ name: "shop", size: 3 });

// A method, a constructor and a decorator.
export const registry = new Registry().add<Economy>(1).add<Shop>(2);
export const tagged = new Tagged<Bank>();

@Mark<Economy>()
export class Marked {}
