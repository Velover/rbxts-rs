import { Provider } from "@flamework-experimental/core";

@Provider({ loadOrder: 1 })
export class Beta {
	static readonly label = "beta";

	value = 1;
}
