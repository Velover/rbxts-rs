// Cx::compile_expr_named: a chain compiled as the initializer of `const <name> = <chain>` names its temporary after
// the variable, as roblox-ts does (`local _target = …`), numbered where the name is taken
interface Box {
	b?: { c: number };
	f?: () => number;
	m(): number;
}
declare function get(tag: string): Box | undefined;
export function simple() {
	const r = probeExprNamed(get("a")?.b, "target");
	return r;
}
export function taken() {
	const target = get("a")?.b;
	{
		const r = probeExprNamed(get("b")?.b?.c, "target");
		print(r);
	}
	return target;
}
export function several() {
	const r1 = probeExprNamed(get("a")?.m(), "target");
	const r2 = probeExprNamed(get("b")?.f?.(), "target_1");
	const r3 = probeExprNamed(get("c")?.["b"], "other");
	return [r1, r2, r3];
}
export function siblings() {
	{
		const r = probeExprNamed(get("a")?.b, "target");
		print(r);
	}
	{
		const r = probeExprNamed(get("b")?.b, "target");
		print(r);
	}
}
export function notChain() {
	const r = probeExprNamed(get("x"), "target");
	return r;
}
export function wrapped() {
	const r1 = probeExprNamed((get("a")?.b), "target");
	const r2 = probeExprNamed(get("b")?.b!, "other");
	return [r1, r2];
}
export const top = probeExprNamed(get("t")?.b, "target");
