// compile_object_literal with FieldOrder::Emitted: the literal as a TS transformer rebuilt it, members reversed and
// `w = 4` appended first, evaluated in that order (a table built inline until a member needs statements)
declare function f(n: number): number;
let i = 0;
export const plain = probeObjEmitted({ a: 1, b: "x", c: true });
export const effects = probeObjEmitted({ a: f(1), b: f(2), ...{ s: f(3) }, m() { return f(4); } });
export const statements = probeObjEmitted({ a: f(1), b: i++ > 0 ? f(2) : f(3), c: f(4), [f(5)]: 6 });
