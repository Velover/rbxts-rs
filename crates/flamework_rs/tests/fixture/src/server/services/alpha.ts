import { OnInit, OnStart, Provider } from "@flamework-experimental/core";
import { Beta } from "./beta";
import { Hub } from "./hub";

/** A provider with dependencies and lifecycle interfaces. */
@Provider()
export class Alpha implements OnStart, OnInit {
	constructor(
		private readonly beta: Beta,
		private readonly hub: Hub,
	) {}

	onInit() {}

	onStart() {
		print(this.beta, this.hub);
	}
}
