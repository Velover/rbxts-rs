import { Injectable, Provider } from "@flamework-experimental/core";

/** `index` files give `init` ids. */
@Provider()
export class Hub {}

/** Constructed through a module, not registered. */
@Injectable()
export class Helper {
	constructor(readonly hub: Hub) {}
}
