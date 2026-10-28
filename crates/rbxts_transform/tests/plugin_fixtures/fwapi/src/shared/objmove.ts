// compile_object_literal_with: the literal a TS transformer rebuilt as `{ ...others, w: extra, moved: value }`, the
// generated values built at their places in the emitted order (`replace_deferred`, `append_deferred`), so their
// statements run after the fields before them: `local _left = "w"`, the statements, `_object[_left] = value`
declare function g(n: number): number;
let j = 0;
export const plain = probeObjMove({ moved: 0, a: 1 }, 2, 3);
export const valueStatements = probeObjMove({ a: g(1), moved: g(2), b: g(3) }, j++ > 0 ? g(4) : g(5), g(6));
export const both = probeObjMove({ moved: 1, a: g(1), [g(2)]: 3 }, j++ > 0 ? g(4) : g(5), j++ > 1 ? g(7) : g(8));
export const nested = probeObjMove({ moved: 1, a: g(1) }, { x: j++ > 0 ? g(4) : g(5), y: g(6) }, g(7));
export const extraOnly = probeObjMove({ a: g(1), moved: 1 }, g(2), j++ > 0 ? g(3) : g(4));
