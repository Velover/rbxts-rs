import { Flamework, requireModules } from "@flamework-experimental/core";

const loaded = requireModules("src/server/services");

Flamework.createModule()
	.registerProviders("src/server/services")
	.registerProvidersGlob("src/server/services/**/*.ts")
	.registerProvidersGlob("./services/hub/*.ts");

// F2: a generic id, declined in F1 and compiled as a plain call.
print(loaded, Flamework.id<string>());

// A glob that matches nothing, and a path with no module in it, are warned about.
Flamework.createModule().registerProvidersGlob("src/server/nothing/*.ts");
