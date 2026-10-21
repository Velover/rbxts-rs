// Generates crates/rbxts_project/tests/fixtures/rojo_cases.json: what the JavaScript @roblox-ts/rojo-resolver says
// about the hand-made fixture trees in crates/rbxts_project/tests/fixtures/rojo. Windows answers (win32 paths).
// Absolute paths are written with placeholders: {root} (the fixtures directory as path.resolve spells it),
// {root_fwd} (the same with forward slashes) and {root_lower} (lowercased).
//
//   node tools/gen-rojo-cases.cjs

"use strict";
const path = require("path");
const fs = require("fs");

const RT = process.env.RBXTS_DIR || "E:/Projects/TS/Flamework/roblox-ts-test/roblox-ts";
const { RojoResolver, NetworkType, RbxType, FileRelation, RbxPathParent } = require(require.resolve("@roblox-ts/rojo-resolver", { paths: [RT] }));

const fixtures = path.resolve(__dirname, "..", "crates", "rbxts_project", "tests", "fixtures", "rojo");
const placeholder = s =>
	typeof s === "string"
		? s.split(fixtures).join("{root}").split(fixtures.replace(/\\/g, "/")).join("{root_fwd}").split(fixtures.toLowerCase()).join("{root_lower}")
		: s;
const expand = s => s.replace("{root}", fixtures).replace("{root_fwd}", fixtures.replace(/\\/g, "/")).replace("{root_lower}", fixtures.toLowerCase());

function resolverJson(r) {
	return {
		isGame: r.isGame,
		warnings: r.getWarnings().map(placeholder),
		partitions: r.getPartitions().map(p => ({ fsPath: placeholder(p.fsPath), rbxPath: [...p.rbxPath] })),
		fileMounts: [...r.filePathToRbxPathMap].map(([k, v]) => [placeholder(k), [...v]]).sort((a, b) => (a[0] < b[0] ? -1 : 1)),
	};
}

function queries(r, list) {
	return list.map(q => {
		const rbxPath = r.getRbxPathFromFilePath(expand(q));
		return {
			query: q,
			rbxPath: rbxPath ? [...rbxPath] : null,
			rbxType: RbxType[r.getRbxTypeFromFilePath(expand(q))],
			network: rbxPath ? NetworkType[r.getNetworkType(rbxPath)] : null,
			isolated: rbxPath ? r.isIsolated(rbxPath) : null,
		};
	});
}

const gameFiles = [
	"out/shared/init.luau",
	"out/shared/module.luau",
	"out/shared/script.server.luau",
	"out/shared/local.client.luau",
	"out/shared/plug.plugin.luau",
	"out/shared/legacy.server.lua",
	"out/shared/data.json",
	"out/shared/thing.model.json",
	"out/shared/thing.meta.json",
	"out/shared/settings.toml",
	"out/shared/readme.txt",
	"out/shared/sub/init.server.luau",
	"out/shared/sub/init.meta.json",
	"out/shared/sub/x.luau",
	"out/shared/deep/a/b.luau",
	"out/shared/nestedproj/lib/z.luau",
	"out/shared/nestedproj/extra.luau",
	"out/shared/withdef/src/w.luau",
	"out/shared/withdef/ignored.luau",
	"out/shared/..dots/d.luau",
	"out/shared",
	"out/single.lua",
	"out/single.luau",
	"out/optional/o.luau",
	"out/server/main.server.luau",
	"out/client/main.client.luau",
	"out/character/c.luau",
	"out/gui/g.luau",
	"out/store/s.luau",
	"out/first/f.luau",
	"out/numbered/n.luau",
	"out/two/t.luau",
	"assets/data.model.json",
	"nested/src/x.luau",
	"nested/child.luau",
	"withdefault/lib/y.luau",
	"include/RuntimeLib.lua",
	"include/Promise.lua",
	"out/notmounted.luau",
	"does/not/exist/q.luau",
	"src/not/synced.ts",
];
const spell = (rel, how) => {
	const win = rel.replace(/\//g, "\\");
	switch (how) {
		case "native":
			return `{root}\\game\\${win}`;
		case "forward":
			return `{root_fwd}/game/${rel}`;
		case "lower-root":
			return `{root_lower}\\game\\${win}`;
		case "upper-dirs": {
			const parts = win.split("\\");
			return `{root}\\GAME\\${parts.map((p, i) => (i < parts.length - 1 ? p.toUpperCase() : p)).join("\\")}`;
		}
	}
};
const gameQueries = [];
for (const how of ["native", "forward", "lower-root", "upper-dirs"]) for (const rel of gameFiles) gameQueries.push(spell(rel, how));
gameQueries.push("{root}\\game\\out\\shared\\..\\shared\\module.luau", "{root}/game//out/shared/./sub/x.luau", "{root}\\game\\out\\shared\\MODULE.LUAU");

const result = { generator: "tools/gen-rojo-cases.cjs", cases: [] };

const game = RojoResolver.fromPath(path.join(fixtures, "game", "default.project.json"));
result.cases.push({ name: "game", kind: "fromPath", config: "{root}\\game\\default.project.json", resolver: resolverJson(game), queries: queries(game, gameQueries) });

const rbx = s => s.split(".").filter(x => x.length > 0);
const pairs = [
	["ReplicatedStorage.Shared", "ReplicatedStorage.Shared.module"],
	["StarterGui.Gui.g", "StarterGui.Gui.x"],
	["StarterGui.Gui.g", "StarterPack.x"],
	["StarterGui.Gui", "ReplicatedStorage.x"],
	["ReplicatedStorage.x", "StarterPlayer.StarterPlayerScripts.Client.main"],
	["StarterPlayer", "StarterPlayer.StarterCharacterScripts.y"],
	["StarterPlayer.StarterPlayerScripts.a", "StarterPlayer.StarterCharacterScripts.b"],
	["", "StarterGui.x"],
	["PluginDebugService.p", "PluginDebugService.q"],
];
result.relations = pairs.map(([a, b]) => ({
	from: rbx(a),
	to: rbx(b),
	relation: FileRelation[game.getFileRelation(rbx(a), rbx(b))],
	relative: RojoResolver.relative(rbx(a), rbx(b)).map(v => (v === RbxPathParent ? ".." : v)),
	isolatedFrom: game.isIsolated(rbx(a)),
	networkFrom: NetworkType[game.getNetworkType(rbx(a))],
}));

const synth = RojoResolver.synthetic(path.join(fixtures, "game", "out"));
result.cases.push({ name: "synthetic-out", kind: "synthetic", base: "{root}\\game\\out", resolver: resolverJson(synth), queries: queries(synth, gameQueries.filter(q => q.includes("out"))) });

const model = RojoResolver.fromPath(path.join(fixtures, "model", "default.project.json"));
result.cases.push({
	name: "model",
	kind: "fromPath",
	config: "{root}\\model\\default.project.json",
	resolver: resolverJson(model),
	queries: queries(model, ["{root}\\model\\out\\init.luau", "{root}\\model\\out\\a\\b.client.luau", "{root}\\model\\lib\\c.luau", "{root}\\model\\other.luau"]),
});

for (const file of fs.readdirSync(path.join(fixtures, "invalid"))) {
	const r = RojoResolver.fromPath(path.join(fixtures, "invalid", file));
	result.cases.push({ name: `invalid/${file}`, kind: "fromPath", config: `{root}\\invalid\\${file}`, resolver: resolverJson(r), queries: [] });
}
const missing = RojoResolver.fromPath(path.join(fixtures, "nope.project.json"));
result.cases.push({ name: "missing", kind: "fromPath", config: "{root}\\nope.project.json", resolver: resolverJson(missing), queries: [] });

result.find = ["game", "multi", "model", "invalid", "game/out"].map(dir => {
	const found = RojoResolver.findRojoConfigFilePath(path.join(fixtures, dir));
	return { dir: `{root}\\${dir.replace(/\//g, "\\")}`, path: found.path ? placeholder(found.path) : null, warnings: found.warnings.map(placeholder) };
});

const target = path.join(fixtures, "..", "rojo_cases.json");
fs.writeFileSync(target, JSON.stringify(result, null, 1) + "\n");
console.log(`wrote ${target}: ${result.cases.length} cases, ${result.cases.reduce((n, c) => n + c.queries.length, 0)} queries`);
