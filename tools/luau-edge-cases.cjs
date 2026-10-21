// Hand-written Luau trees for the renderer's edge cases, rendered by roblox-ts's own `@roblox-ts/luau-ast`.
//
// Usage: node tools/luau-edge-cases.cjs <roblox-ts checkout> <out.jsonl> [--print]
//
// Writes the same JSONL as dump-luau-ast.cjs, for `rbxts-bench luau-parity`; the committed copy is
// crates/rbxts_bench/fixtures/luau-edge-cases.jsonl. `--print` shows each case's text (the expectations in
// rbxts_luau's unit tests were taken from it).
"use strict";
const fs = require("fs");
const path = require("path");
const { loadLuauAst, createSerializer } = require("./luau-ast-json.cjs");

const [rtArg, outArg, flag] = process.argv.slice(2);
if (!rtArg || !outArg) {
	console.error("usage: node luau-edge-cases.cjs <roblox-ts dir> <out.jsonl> [--print]");
	process.exit(2);
}
const luauAst = loadLuauAst(path.resolve(rtArg));
const { luau } = luauAst;
const serializeFile = createSerializer(luauAst);
const K = luau.SyntaxKind;
const L = (...nodes) => luau.list.make(...nodes);

// shorthands
const id = luau.id;
const str = luau.string;
const num = luau.number;
const numText = value => luau.create(K.NumberLiteral, { value });
const paren = expression => luau.create(K.ParenthesizedExpression, { expression });
const callStmt = expression => luau.create(K.CallStatement, { expression });
const call = (f, args = []) => luau.call(f, args);
const method = (expression, name, args = []) => luau.create(K.MethodCallExpression, { expression, name, args: L(...args) });
const local = (left, right) => luau.create(K.VariableDeclaration, { left, right });
const assign = (left, operator, right) => luau.create(K.Assignment, { left, operator, right });
const ret = expression => luau.create(K.ReturnStatement, { expression });
const index = (expression, i) => luau.create(K.ComputedIndexExpression, { expression, index: i });
const ifExpr = (condition, expression, alternative) => luau.create(K.IfExpression, { condition, expression, alternative });
const fn = (parameters, statements, hasDotDotDot = false) =>
	luau.create(K.FunctionExpression, { parameters: L(...parameters), hasDotDotDot, statements: L(...statements) });
const print = (...args) => callStmt(call(id("print"), args));

const cases = {
	strings: () => {
		const values = ["plain", 'say "hi"', "it's", `both " and '`, "two\nlines", "a]]b\nc", "ends]", "x]=]y\nz"];
		return L(
			...values.map(v => print(str(v))),
			print(luau.string("already \\'escaped\\'", "'"), luau.string('dq \\"', '"')),
			print(luau.map([[str("a\nb"), num(1)], [str("ok"), num(2)], [str("not ok"), num(3)]])),
			print(index(id("t"), str("x\ny")), index(id("t"), str("name")), index(id("t"), str("end"))),
			print(luau.set([str("k\nl"), str("key"), num(5)])),
			print(luau.create(K.PropertyAccessExpression, { expression: id("t"), name: "my-prop" })),
		);
	},
	numbers: () =>
		L(
			print(numText("1_000"), numText("0x_FF"), numText("0b1010"), numText(".5"), numText("1e10"), numText("5.")),
			print(numText("0o17"), numText("Infinity"), numText("1_0n"), numText("0O7_7")),
			print(num(-5), num(1e21), num(0.1 + 0.2), num(-0), num(1e-7), num(123456789.125)),
			print(luau.unary("-", num(-5)), luau.unary("-", luau.unary("-", id("a")))),
		),
	precedence: () => {
		const b = luau.binary;
		const [a, c, d] = [id("a"), id("c"), id("d")];
		return L(
			print(b(b(a, "+", id("b")), "*", c), b(a, "-", b(id("b"), "-", c)), b(b(a, "-", id("b")), "-", c)),
			print(b(a, "^", b(id("b"), "^", c)), b(luau.unary("-", a), "^", id("b")), b(a, "^", luau.unary("-", id("b")))),
			print(luau.unary("not", b(a, "==", id("b"))), luau.unary("-", b(a, "+", id("b"))), luau.unary("#", index(a, num(1)))),
			print(b(b(a, "and", id("b")), "or", c), b(b(a, "or", id("b")), "and", c), b(a, "or", b(id("b"), "and", c))),
			print(b(a, "..", b(id("b"), "..", c)), b(b(a, "..", id("b")), "..", c), b(b(a, "//", id("b")), "%", c)),
			print(b(ifExpr(a, id("b"), c), "+", d), b(d, "or", ifExpr(a, id("b"), c)), b(ifExpr(a, id("b"), c), "or", d)),
			print(luau.unary("not", luau.unary("not", a)), luau.unary("-", ifExpr(a, id("b"), c))),
			print(paren(paren(a)), paren(b(a, "+", id("b"))), b(paren(b(a, "+", id("b"))), "*", c)),
		);
	},
	semicolons: () =>
		L(
			local(id("a"), id("b")),
			callStmt(call(paren(fn([], [])))),
			assign(id("x"), "=", num(1)),
			callStmt(call(paren(id("f")))),
			callStmt(call(id("g"))),
			luau.comment(" a comment in between"),
			assign(index(paren(id("t")), num(1)), "=", num(2)),
			local(id("c"), undefined),
			callStmt(method(paren(id("obj")), "m")),
			local(id("d"), luau.binary(id("x"), "+", num(1))),
			callStmt(call(paren(id("h")))),
			assign(L(id("p"), id("q")), "=", L(id("q"), luau.unary("-", id("p")))),
			callStmt(call(paren(id("i")))),
			local(id("e"), ifExpr(id("x"), num(1), id("y"))),
			assign(L(luau.property(paren(id("u")), "v"), id("w")), "=", L(num(1), num(2))),
		),
	tempIds: () => {
		const exp = luau.tempId("exp");
		const exp2 = luau.tempId("exp");
		const anon = luau.tempId();
		const anon2 = luau.tempId();
		const x1 = luau.tempId("x");
		const x2 = luau.tempId("x");
		const xOne = luau.tempId("x_1");
		const inner = luau.tempId("exp");
		const param = luau.tempId("param");
		const forOnly = luau.tempId("value");
		const forOnly2 = luau.tempId("other");
		return L(
			local(id("_exp"), num(0)),
			local(exp, num(1)),
			local(exp2, num(2)),
			local(L(anon, anon2), L(num(3), num(4))),
			local(x1, num(5)),
			local(x2, num(6)),
			local(xOne, num(7)),
			luau.create(K.FunctionDeclaration, {
				localize: true,
				name: id("f"),
				parameters: L(param, id("_exp_2")),
				hasDotDotDot: true,
				statements: L(local(inner, param), ret(L(exp, inner, luau.create(K.VarArgsLiteral, {})))),
			}),
			luau.create(K.ForStatement, {
				ids: L(id("_"), id("v")),
				expression: call(id("pairs"), [luau.map([[forOnly2, num(1)], [str("k"), forOnly]])]),
				statements: L(print(id("v"))),
			}),
			luau.create(K.DoStatement, { statements: L(local(luau.tempId("exp"), num(8)), print(exp)) }),
		);
	},
	tempIdScopes: () => {
		// a function declaration first in its block, and a temporary named after a local declared later in the block
		const t = luau.tempId("t");
		return L(
			luau.create(K.DoStatement, {
				statements: L(
					luau.create(K.FunctionDeclaration, {
						localize: true,
						name: id("g"),
						parameters: L(luau.tempId("t")),
						hasDotDotDot: false,
						statements: L(),
					}),
					local(t, num(1)),
					local(id("_t"), t),
				),
			}),
		);
	},
	blocks: () =>
		L(
			luau.create(K.DoStatement, { statements: L() }),
			luau.create(K.WhileStatement, { condition: luau.bool(true), statements: L(luau.create(K.BreakStatement, {})) }),
			luau.create(K.RepeatStatement, {
				condition: id("done"),
				statements: L(luau.create(K.ContinueStatement, {}), luau.comment("after continue")),
			}),
			luau.create(K.IfStatement, {
				condition: id("a"),
				statements: L(),
				elseBody: luau.create(K.IfStatement, {
					condition: id("b"),
					statements: L(print(num(1))),
					elseBody: luau.create(K.IfStatement, { condition: id("c"), statements: L(), elseBody: L(print(num(2))) }),
				}),
			}),
			luau.create(K.IfStatement, { condition: id("d"), statements: L(print(num(3))), elseBody: L() }),
			luau.create(K.NumericForStatement, { id: id("i"), start: num(1), end: id("n"), step: numText("1.0"), statements: L() }),
			luau.create(K.NumericForStatement, { id: id("j"), start: num(10), end: num(1), step: num(-1), statements: L(print(id("j"))) }),
			luau.create(K.ForStatement, { ids: L(), expression: call(id("iter")), statements: L() }),
			local(id("empty"), fn([], [])),
			local(id("tables"), luau.array([luau.array(), luau.map(), luau.set(), luau.mixedTable()])),
			luau.create(K.MethodDeclaration, {
				expression: id("Class"),
				name: "method",
				parameters: L(id("self")),
				hasDotDotDot: false,
				statements: L(ret(luau.mixedTable([num(1), [str("key"), fn([id("x")], [ret(id("x"))])], [num(2), num(3)]]))),
			}),
			luau.create(K.FunctionDeclaration, {
				localize: false,
				name: luau.property(id("Class"), "static"),
				parameters: L(),
				hasDotDotDot: true,
				statements: L(ret(L())),
			}),
		),
	interpolation: () =>
		L(
			print(
				luau.create(K.InterpolatedString, {
					parts: L(
						luau.create(K.InterpolatedStringPart, { text: "a {b} \\{c\\} \\u{1F600} \\\\u{41}\nline\r\nend\\" }),
						luau.array([num(1)]),
						luau.create(K.InterpolatedStringPart, { text: " and " }),
						luau.binary(id("x"), "+", num(1)),
						luau.map([[str("k"), num(1)]]),
					),
				}),
			),
		),
	ifExpressions: () =>
		L(
			local(id("v"), ifExpr(id("a"), num(1), ifExpr(id("b"), num(2), ifExpr(id("c"), num(3), num(4))))),
			local(id("w"), ifExpr(ifExpr(id("a"), id("b"), id("c")), id("d"), id("e"))),
		),
	assignments: () =>
		L(
			...["=", "+=", "-=", "*=", "/=", "//=", "%=", "^=", "..="].map(op => assign(id("a"), op, num(1))),
			assign(index(id("t"), str("k")), "..=", str("x")),
		),
	literalCalls: () =>
		L(
			print(method(paren(str("abc")), "upper"), method(paren(num(5)), "x"), method(paren(luau.array()), "y")),
			print(method(paren(luau.binary(id("a"), "..", id("b"))), "len"), call(paren(fn([], [ret(num(1))])))),
			print(call(luau.property(paren(luau.map([[str("a"), num(1)]])), "a"))),
		),
	comments: () =>
		L(
			luau.comment("single"),
			luau.comment("multi\n\n  \nline ]] and ]=]"),
			luau.create(K.DoStatement, { statements: L(luau.comment("nested\nblock"), luau.comment("")) }),
		),
};

const fd = fs.openSync(path.resolve(outArg), "w");
for (const [name, make] of Object.entries(cases)) {
	const ast = make();
	const text = luauAst.renderModule.renderAST(ast);
	const { json } = serializeFile(ast);
	fs.writeSync(fd, JSON.stringify({ file: `edge/${name}`, text, ast: json }) + "\n");
	if (flag === "--print") console.log(`=== ${name}\n${text}`);
}
fs.closeSync(fd);
