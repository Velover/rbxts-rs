// computed keys: offered to property_key with the key of a single string literal type, else `None` (declined)
const ka = "a";
const km = "m";
export const c1 = keyed[ka];
export const c2 = keyed[keyName() as "b"];
keyed[ka] = 1;
keyed[keyName() as "b"] += 2;
export const c3 = keyed[km](1);
export const c4 = keyed[keyName() as "m"](2);
export const c5 = keyed[km]?.(3);
export const c6 = maybeKeyed?.[ka];
export const c7 = keyed[anyKey as "a" | "b"];
export const c8 = keyedList[0];
delete keyed[keyName() as "opt"];
export const c9 = probeLinks();

interface CompAttrs {
	count: number;
	label: string;
}

// assignments with a computed key: the hook compiles `key_node`
export class CompKeys {
	/** @probeattrs */
	attrs: CompAttrs = { count: 0, label: "" };

	run(k: "count" | "label") {
		const kc = "count";
		this.attrs[kc] = 2;
		this.attrs[k] = 1 as never;
	}
}
