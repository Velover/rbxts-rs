import { Provider } from "@flamework-experimental/core";

export namespace Outer {
	export namespace Inner {
		/** A provider in a namespace: its id names the namespaces, and it is still module-level. */
		@Provider()
		export class Deep {}
	}
}

/** A provider made by a function call is not module-level: no `flamework:module`. */
export function makeProvider() {
	@Provider()
	class Local {}

	return Local;
}
