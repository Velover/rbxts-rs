// Dumps the Luau ASTs roblox-ts renders, with the text it renders them to, for the `rbxts-bench luau-parity` check.
//
// Usage (from the project directory, as transformer plugins find the tsconfig through the working directory):
//   node <rbxts-rs>/tools/dump-luau-ast.cjs <roblox-ts checkout with out/ built> <tsconfig> <out.jsonl>
//
// Runs roblox-ts's build in-process (TypeScript 5, as `rbxtsc build --allowCommentDirectives --optimizedLoops`) and
// wraps `renderAST` at run time, so nothing in the roblox-ts checkout changes (its export is a plain property of
// LuauRenderer/render.js that the package index reads through getters). Each output line is one rendered file:
// { file, text, ast, stats } (see luau-ast-json.cjs). The build writes its outputs as a normal build does.
"use strict";
const fs = require("fs");
const path = require("path");
const { loadLuauAst, createSerializer } = require("./luau-ast-json.cjs");

const [rtArg, configArg, outArg] = process.argv.slice(2);
if (!rtArg || !configArg || !outArg) {
	console.error("usage: node dump-luau-ast.cjs <roblox-ts dir> <tsconfig> <out.jsonl>");
	process.exit(2);
}
const RT = path.resolve(rtArg);
const CONFIG = path.resolve(configArg);
const OUT = path.resolve(outArg);
const ROOT = path.dirname(CONFIG);
const req = p => require(path.join(RT, "out", p));

const luauAst = loadLuauAst(RT);
const serializeFile = createSerializer(luauAst);

// remember which file is being transformed; renderAST only gets the AST
let currentFile;
const transformSourceFileModule = req("TSTransformer/nodes/transformSourceFile");
const transformSourceFile = transformSourceFileModule.transformSourceFile;
transformSourceFileModule.transformSourceFile = (state, sourceFile) => {
	currentFile = sourceFile.fileName;
	return transformSourceFile(state, sourceFile);
};

const out = fs.openSync(OUT, "w");
let files = 0;
let renderMs = 0;
const totals = { nodes: 0, parentMissing: {}, parentStale: {}, shared: {}, decisions: {} };
const renderAST = luauAst.renderModule.renderAST;
luauAst.renderModule.renderAST = ast => {
	const start = performance.now();
	const text = renderAST(ast);
	renderMs += performance.now() - start;
	const { json, stats } = serializeFile(ast);
	const file = path.relative(ROOT, currentFile ?? "?").replace(/\\/g, "/");
	fs.writeSync(out, JSON.stringify({ file, text, ast: json, stats }) + "\n");
	files++;
	totals.nodes += stats.nodes;
	for (const key of ["parentMissing", "parentStale", "shared", "decisions"]) {
		for (const [k, n] of Object.entries(stats[key])) totals[key][k] = (totals[key][k] ?? 0) + n;
	}
	return text;
};

const { ProjectBuild } = req("Project/classes/ProjectBuild");
const { formatDiagnostics } = req("Shared/util/formatDiagnostics");
const build = new ProjectBuild(CONFIG, { allowCommentDirectives: true, optimizedLoops: true });
for (const project of build.graph.projects.values()) {
	// without the build info every file is compiled (and rendered) again
	if (project.pathTranslator?.buildInfoOutputPath) fs.rmSync(project.pathTranslator.buildInfoOutputPath, { force: true });
}
const t0 = performance.now();
const result = build.build();
build.close?.();
fs.closeSync(out);

console.log(
	JSON.stringify(
		{ files, buildMs: Math.round(performance.now() - t0), renderMs: Math.round(renderMs), diagnostics: result.diagnostics.length, ...totals },
		null,
		1,
	),
);
if (result.diagnostics.length) console.log(formatDiagnostics(result.diagnostics.slice(0, 10)));
process.exitCode = result.diagnostics.length ? 1 : 0;
