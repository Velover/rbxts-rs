// TS1238 / TS1241 over TS2769: a decorator no overload takes, each overload's error listed
function dec(x: number): void;
function dec(x: string): void;
function dec(x: unknown) {}
@dec
class K {}
declare function method(a: number, b: string, c: TypedPropertyDescriptor<() => void>): void;
declare function method(a: string, b: string, c: TypedPropertyDescriptor<() => void>): void;
class M {
	@method
	m() {}
}
export { K, M };
