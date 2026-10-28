// import_named_at: a member's import asked for at its class, so before the decorator's (TypeScript transforms a
// class's members before its decorators), though the decorator is written first
@probeDecoImportFrom("./decos", "dA")
export class C {
	m() {
		return probeImportAtClass("./decos", "dB");
	}
}
