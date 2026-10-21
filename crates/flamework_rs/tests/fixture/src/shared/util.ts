/** No Flamework at all: compiled exactly as without the plugin. */
export function add(a: number, b: number) {
	return a + b;
}

export class Plain {
	value = add(1, 2);
}
