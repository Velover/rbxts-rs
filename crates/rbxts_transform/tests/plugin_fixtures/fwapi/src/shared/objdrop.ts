// ObjectEdits::drop: a written member left out of the literal a TS transformer rebuilt, an accessor included (no
// "Getters and Setters are not supported!"), and `attributes` appended in its stead with a deferred value
declare function h(n: number): number;
let k = 0;
export const getter = probeObjDrop({ tag: "x", get attributes() { return { name: 1 }; } }, { name: h(1) });
export const setter = probeObjDrop({ set attributes(v: number) {}, tag: h(1) }, k++ > 0 ? h(2) : h(3));
export const field = probeObjDrop({ a: h(1), attributes: h(2), b: h(3) }, h(4));
export const method = probeObjDrop({ attributes() { return 1; }, a: h(1) }, { x: k++ > 0 ? h(2) : h(3) });
export const spread = probeObjDrop({ ...{ s: h(1) }, attributes: 0, b: h(2) }, h(3));
