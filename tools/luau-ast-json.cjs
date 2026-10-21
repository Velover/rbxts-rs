// Serializes `@roblox-ts/luau-ast` trees to the JSON that `rbxts-bench luau-parity` reads (shared by
// dump-luau-ast.cjs and luau-edge-cases.cjs).
"use strict";
const path = require("path");

// SyntaxKind names by value (the enum's reverse map is overwritten by its First*/Last* aliases)
const KIND_NAMES = [
	"Identifier", "TemporaryIdentifier", "ComputedIndexExpression", "PropertyAccessExpression", "CallExpression",
	"MethodCallExpression", "ParenthesizedExpression", "None", "NilLiteral", "FalseLiteral", "TrueLiteral",
	"NumberLiteral", "StringLiteral", "VarArgsLiteral", "FunctionExpression", "BinaryExpression", "UnaryExpression",
	"IfExpression", "InterpolatedString", "Array", "Map", "Set", "MixedTable", "Assignment", "BreakStatement",
	"CallStatement", "ContinueStatement", "DoStatement", "WhileStatement", "RepeatStatement", "IfStatement",
	"NumericForStatement", "ForStatement", "FunctionDeclaration", "MethodDeclaration", "VariableDeclaration",
	"ReturnStatement", "Comment", "MapField", "InterpolatedStringPart",
];

/** Loads `@roblox-ts/luau-ast` as the roblox-ts checkout at `rt` resolves it. */
function loadLuauAst(rt) {
	const dir = path.dirname(require.resolve("@roblox-ts/luau-ast/package.json", { paths: [rt] }));
	const luau = require(dir).default;
	for (const [i, name] of KIND_NAMES.entries()) {
		if (luau.SyntaxKind[name] !== i) throw new Error(`SyntaxKind.${name} is ${luau.SyntaxKind[name]}, expected ${i}`);
	}
	return {
		luau,
		renderModule: require(path.join(dir, "out/LuauRenderer/render.js")),
		needsParentheses: require(path.join(dir, "out/LuauRenderer/util/needsParentheses.js")).needsParentheses,
	};
}

function bump(map, key) {
	map[key] = (map[key] ?? 0) + 1;
}

/**
 * Returns `serializeFile(ast) -> { json, stats }`.
 * - `json`: a node is { k: <SyntaxKind name>, ...fields } and a list is an array. Temporary identifiers get a per-file
 *   id in order of appearance: the TS ids are process-wide, and the renderer only compares them for equality.
 * - `stats` counts what a structural (parent-free) renderer could get wrong: nodes whose `parent` is missing or is
 *   not the node that holds them, nodes shared between two places, and the parent-dependent render decisions
 *   (parentheses, scope edges, long-bracket spacing) that differ between the real and the structural parent.
 */
function createSerializer({ luau, needsParentheses }) {
	function scopeEdge(node, parent, edge) {
		if (!parent) return false;
		if (luau.hasStatements(parent) && node === parent.statements[edge]?.value) return true;
		return luau.isIfStatement(parent) && luau.list.isList(parent.elseBody) && node === parent.elseBody[edge]?.value;
	}
	function bracketSpacing(node, parent) {
		if (!parent) return false;
		return (
			(luau.isMapField(parent) && node === parent.index) ||
			(luau.isComputedIndexExpression(parent) && node === parent.index) ||
			luau.isSet(parent)
		);
	}
	function withParent(node, parent, fn) {
		const saved = node.parent;
		node.parent = parent;
		try {
			return fn();
		} finally {
			node.parent = saved;
		}
	}

	return function serializeFile(ast) {
		const tempIds = new Map();
		const seen = new Set();
		const stats = { nodes: 0, parentMissing: {}, parentStale: {}, shared: {}, decisions: {} };

		function check(node, parent) {
			stats.nodes++;
			const kind = KIND_NAMES[node.kind];
			if (seen.has(node)) bump(stats.shared, kind);
			seen.add(node);
			if (node.parent === parent) return;
			bump(parent && !node.parent ? stats.parentMissing : stats.parentStale, kind);
			if (luau.isExpressionWithPrecedence(node)) {
				const real = needsParentheses(node);
				const structural = withParent(node, parent, () => needsParentheses(node));
				if (real !== structural) bump(stats.decisions, "parentheses");
			}
			if (luau.isStatement(node)) {
				for (const edge of ["head", "tail"]) {
					if (scopeEdge(node, node.parent, edge) !== scopeEdge(node, parent, edge)) {
						bump(stats.decisions, `scope ${edge}`);
					}
				}
			}
			if (luau.isStringLiteral(node) && node.quote === undefined) {
				const long = node.value.includes("\n") || (node.value.includes('"') && node.value.includes("'"));
				if (long && bracketSpacing(node, node.parent) !== bracketSpacing(node, parent)) {
					bump(stats.decisions, "bracket spacing");
				}
			}
		}

		function value(v, parent) {
			if (luau.isNode(v)) return node(v, parent);
			if (luau.list.isList(v)) return luau.list.mapToArray(v, item => node(item, parent));
			return v;
		}

		function node(n, parent) {
			check(n, parent);
			const out = { k: KIND_NAMES[n.kind] };
			for (const [key, v] of Object.entries(n)) {
				if (key === "kind" || key === "parent" || v === undefined) continue;
				if (key === "id" && luau.isTemporaryIdentifier(n)) {
					let id = tempIds.get(v);
					if (id === undefined) tempIds.set(v, (id = tempIds.size));
					out.id = id;
					continue;
				}
				out[key] = value(v, n);
			}
			return out;
		}

		// top-level statements have no parent node
		const json = luau.list.mapToArray(ast, item => node(item, undefined));
		return { json, stats };
	};
}

module.exports = { KIND_NAMES, loadLuauAst, createSerializer };
