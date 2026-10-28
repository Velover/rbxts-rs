// compile_call_with: a value the plugin builds in the second slot (here the third written argument, compiled there),
// so its temporaries are named between the first and the last argument's, and the earlier values are bound to locals
// as for a written argument with prereqs (`ensureTransformOrder`)
declare function f(n: number): number;
declare const x: number | undefined, y: number | undefined, z: number | undefined;
export const d = probeDeferred(x ?? f(1), y ?? f(2), z ?? f(3));
export const e = probeDeferred(f(4), f(5), z ?? f(6));
