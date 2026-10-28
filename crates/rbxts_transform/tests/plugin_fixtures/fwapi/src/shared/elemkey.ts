// a property key answer with the element access spelling (`KeyAnswer::as_element_access`): a method call through it
// is roblox-ts's element call, `obj.key(obj, …)`, as for the `obj["key"](…)` a TS transformer writes
declare const ek: ElemKeyed;
declare function getEk(): ElemKeyed;
export const a = ek.elem(1);
export const b = ek.elemsame(2);
export const c = getEk().elem(3);
export function d(o: ElemKeyed) {
	return o.elem(o.elemsame(4));
}
