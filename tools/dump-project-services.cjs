// Dumps what roblox-ts's project setup computes for a project, with the JavaScript packages roblox-ts itself uses,
// for `rbxts-bench project-parity` to compare against rbxts_project.
//
//   node tools/dump-project-services.cjs <tsconfig.json | project dir> <out.json> [--rojo <path>] [--type <t>]
//        [--includePath <path>] [--timing-runs N]
//
// RBXTS_DIR (default E:/Projects/TS/Flamework/roblox-ts-test/roblox-ts) is a roblox-ts checkout with `out/` built;
// `@roblox-ts/rojo-resolver` and `@roblox-ts/path-translator` come from its node_modules.
//
// For every project of the build graph (the root and its project references): the project data, the path translator,
// the Rojo resolver (partitions, file mounts, warnings), the package resolvers and path mapping, the project type and
// runtime library path, compileFiles's diagnostics, and per program source file the output, declaration, transformed,
// input and import paths with their Rojo paths, types, network types and isolation. Also the resolver build time,
// with fs.realpathSync as shipped and with realpathSync.native.

"use strict";
const path = require("path");
const fs = require("fs");

const RT = process.env.RBXTS_DIR || "E:/Projects/TS/Flamework/roblox-ts-test/roblox-ts";
const rtRequire = mod => require(require.resolve(mod, { paths: [RT] }));

const kleur = rtRequire("kleur");
kleur.enabled = false;
const ts = rtRequire("typescript");
const { RojoResolver, NetworkType, RbxType, RbxPathParent } = rtRequire("@roblox-ts/rojo-resolver");
const { ProjectGraph } = require(path.join(RT, "out/Project/classes/ProjectGraph"));
const { checkRojoConfig } = require(path.join(RT, "out/Project/functions/checkRojoConfig"));
const { checkFileName } = require(path.join(RT, "out/Project/functions/checkFileName"));
const { createNodeModulesPathMapping } = require(path.join(RT, "out/Project/functions/createNodeModulesPathMapping"));
const { getRootDirs } = require(path.join(RT, "out/Shared/util/getRootDirs"));
const { getCanonicalFileName } = require(path.join(RT, "out/Shared/util/getCanonicalFileName"));

// `compileFiles` is not exported in pieces, so its setup part (the Rojo resolver and its warnings, the input file
// list, the project type and the runtime library checks) is copied below. Refuse to dump when the checkout's
// `compileFiles.js` no longer holds what was copied, so a roblox-ts change cannot go unnoticed.
(function checkCopiedSetup() {
	const source = fs.readFileSync(path.join(RT, "out/Project/functions/compileFiles.js"), "utf8");
	const copied = [
		"rojo_resolver_1.RojoResolver.fromPath(data.rojoConfigPath) : rojo_resolver_1.RojoResolver.synthetic(outDir)",
		"for (const warning of rojoResolver.getWarnings()) {",
		".filter(sourceFile => !sourceFile.isDeclarationFile && !program.isSourceFileFromExternalLibrary(sourceFile))",
		"(0, checkRojoConfig_1.checkRojoConfig)(data, rojoResolver, (0, getRootDirs_1.getRootDirs)(compilerOptions), inputFileNames, pathTranslator);",
		"if (!path_1.default.normalize(sourceFile.fileName).startsWith(data.nodeModulesPath)) {",
		"const pkgRojoResolvers = compilerOptions.typeRoots.map(rojo_resolver_1.RojoResolver.synthetic);",
		"(0, createNodeModulesPathMapping_1.createNodeModulesPathMapping)(compilerOptions.typeRoots);",
		"data.projectOptions.type) !== null && _b !== void 0 ? _b : inferProjectType(data, rojoResolver);",
		"if (data.isPackage) {\n        return constants_1.ProjectType.Package;\n    }\n    else if (rojoResolver.isGame) {",
		"if (projectType !== constants_1.ProjectType.Package && data.rojoConfigPath === undefined) {\n        return emitResultFailure(\"Non-package projects must have a Rojo project file!\");",
		"runtimeLibRbxPath = rojoResolver.getRbxPathFromFilePath(path_1.default.join(data.projectOptions.includePath, \"RuntimeLib.lua\"));",
		"return emitResultFailure(\"Rojo project contained no data for include folder!\");",
		"else if (rojoResolver.getNetworkType(runtimeLibRbxPath) !== rojo_resolver_1.NetworkType.Unknown) {\n            return emitResultFailure(\"Runtime library cannot be in a server-only or client-only container!\");",
		"else if (rojoResolver.isIsolated(runtimeLibRbxPath)) {\n            return emitResultFailure(\"Runtime library cannot be in an isolated container!\");",
	];
	const missing = copied.filter(text => !source.replace(/\r\n/g, "\n").includes(text));
	if (missing.length > 0) {
		console.error("roblox-ts's compileFiles.js changed; update the setup this tool copies from it. Missing:");
		for (const text of missing) console.error(`  ${text}`);
		process.exit(2);
	}
})();

function parseArgs() {
	const args = process.argv.slice(2);
	const positional = [];
	const overrides = {};
	let timingRuns = 3;
	for (let i = 0; i < args.length; i++) {
		const a = args[i];
		if (a === "--rojo") overrides.rojo = path.resolve(args[++i]);
		else if (a === "--type") overrides.type = args[++i];
		else if (a === "--includePath") overrides.includePath = path.resolve(args[++i]);
		else if (a === "--timing-runs") timingRuns = Number(args[++i]);
		else positional.push(a);
	}
	if (positional.length !== 2) {
		console.error("usage: node tools/dump-project-services.cjs <tsconfig.json | dir> <out.json> [--rojo p] [--type t] [--includePath p]");
		process.exit(2);
	}
	let tsconfig = path.resolve(positional[0]);
	if (fs.statSync(tsconfig).isDirectory()) tsconfig = path.join(tsconfig, "tsconfig.json");
	return { tsconfig, out: path.resolve(positional[1]), overrides, timingRuns };
}

// LogService exists under two module paths (roblox-ts's `out` mixes `Shared` and `shared`), so capture its output.
const warnings = [];
function captureWarnings(fn) {
	const write = process.stdout.write;
	let buffer = "";
	process.stdout.write = chunk => {
		buffer += String(chunk);
		return true;
	};
	try {
		return fn();
	} finally {
		process.stdout.write = write;
		for (const line of buffer.split("\n")) {
			if (line.startsWith("Compiler Warning: ")) warnings.push(line.slice("Compiler Warning: ".length));
			else if (line.length > 0) warnings.push(`(stdout) ${line}`);
		}
	}
}

function flushDiagnostics() {
	const out = [];
	for (const [key, mod] of Object.entries(require.cache)) {
		if (/classes[\\/]DiagnosticService\.js$/i.test(key) && mod.exports.DiagnosticService) {
			for (const d of mod.exports.DiagnosticService.flush()) out.push(diagnosticJson(d));
		}
	}
	return out;
}

function diagnosticJson(d) {
	return {
		severity: d.category === ts.DiagnosticCategory.Error ? "error" : d.category === ts.DiagnosticCategory.Warning ? "warning" : "other",
		message: ts.flattenDiagnosticMessageText(d.messageText, "\n"),
	};
}

const rbxPathJson = p => (p === undefined ? null : [...p]);
const relativeJson = p => p.map(v => (v === RbxPathParent ? ".." : v));
const networkName = n => NetworkType[n];
const rbxTypeName = t => RbxType[t];

function resolverJson(r) {
	return {
		isGame: r.isGame,
		warnings: [...r.getWarnings()],
		partitions: r.getPartitions().map(p => ({ fsPath: p.fsPath, rbxPath: [...p.rbxPath] })),
		fileMounts: [...r.filePathToRbxPathMap].map(([k, v]) => [k, [...v]]).sort((a, b) => (a[0] < b[0] ? -1 : 1)),
	};
}

// The installed resolver may be the locally patched one (realpathSync.native); the shipped file is kept beside it as
// RojoResolver.js.orig. Each variant is timed as installed, and with fs.realpathSync swapped for the other one.
function timeResolver(rojoConfigPath, runs) {
	if (!rojoConfigPath || !fs.existsSync(rojoConfigPath)) return null;
	const installed = require.resolve("@roblox-ts/rojo-resolver", { paths: [RT] });
	const fsx = require(require.resolve("fs-extra", { paths: [path.dirname(installed)] }));
	const measure = Resolver => {
		const times = [];
		for (let i = 0; i < runs; i++) {
			const start = process.hrtime.bigint();
			Resolver.fromPath(rojoConfigPath);
			times.push(Number(process.hrtime.bigint() - start) / 1e6);
		}
		times.sort((a, b) => a - b);
		return { minMs: times[0], medianMs: times[Math.floor(times.length / 2)], runs };
	};
	const source = fs.readFileSync(installed, "utf8");
	const result = { installedUsesNative: source.includes("realpathSync.native") };
	const orig = installed + ".orig";
	const shipped = fs.existsSync(orig) ? require(orig).RojoResolver : result.installedUsesNative ? null : RojoResolver;
	if (shipped) {
		// the shipped code calls fsx.realpathSync: time it as shipped, then with the native implementation
		result.shipped = measure(shipped);
		const original = fsx.realpathSync;
		fsx.realpathSync = original.native;
		try {
			result.shippedWithNative = measure(shipped);
		} finally {
			fsx.realpathSync = original;
		}
	}
	if (result.installedUsesNative) result.installedPatched = measure(RojoResolver);
	return result;
}

function dumpProject(node, graph, isRoot, timingRuns) {
	const { data, config, pathTranslator } = node;
	const options = config.options;
	const out = {
		tsconfig: data.tsConfigPath,
		isRoot,
		// what a single-project build must be told to reproduce this project's data
		overrides: isRoot
			? null
			: { rojo: data.rojoConfigPath ?? null, includePath: data.projectOptions.includePath, type: data.projectOptions.type ?? null, luau: data.projectOptions.luau },
		data: {
			projectPath: data.projectPath,
			isPackage: data.isPackage,
			nodeModulesPath: data.nodeModulesPath,
			includePath: data.projectOptions.includePath,
			rojoConfigPath: data.rojoConfigPath ?? null,
			type: data.projectOptions.type ?? null,
			luau: data.projectOptions.luau,
		},
		pathTranslator: {
			rootDir: pathTranslator.rootDir,
			outDir: pathTranslator.outDir,
			buildInfoOutputPath: pathTranslator.buildInfoOutputPath ?? null,
			declaration: pathTranslator.declaration,
			useLuauExtension: pathTranslator.useLuauExtension,
		},
	};

	// compileFiles, up to the transform
	const program = ts.createProgram({ rootNames: config.fileNames, options, projectReferences: config.projectReferences });
	const sourceFiles = program.getSourceFiles();
	flushDiagnostics();
	let rojoResolver;
	const setupWarningStart = warnings.length;
	captureWarnings(() => {
		rojoResolver = data.rojoResolver ?? (data.rojoConfigPath ? RojoResolver.fromPath(data.rojoConfigPath) : RojoResolver.synthetic(options.outDir));
		for (const w of rojoResolver.getWarnings()) require(path.join(RT, "out/Shared/classes/LogService")).LogService.warn(w);
	});
	const inputFileNames = sourceFiles
		.filter(sf => !sf.isDeclarationFile && !program.isSourceFileFromExternalLibrary(sf))
		.map(sf => sf.fileName);
	checkRojoConfig(data, rojoResolver, getRootDirs(options), inputFileNames, pathTranslator);
	for (const sf of sourceFiles) {
		if (!path.normalize(sf.fileName).startsWith(data.nodeModulesPath)) checkFileName(sf.fileName);
	}
	const diagnostics = flushDiagnostics();
	const pkgRojoResolvers = options.typeRoots.map(RojoResolver.synthetic);
	const mapping = createNodeModulesPathMapping(options.typeRoots);
	const projectType = data.projectOptions.type ?? (data.isPackage ? "package" : rojoResolver.isGame ? "game" : "model");
	let fatal = null;
	let runtimeLibRbxPath;
	if (projectType !== "package" && data.rojoConfigPath === undefined) {
		fatal = "Non-package projects must have a Rojo project file!";
	} else if (projectType !== "package") {
		runtimeLibRbxPath = rojoResolver.getRbxPathFromFilePath(path.join(data.projectOptions.includePath, "RuntimeLib.lua"));
		if (!runtimeLibRbxPath) fatal = "Rojo project contained no data for include folder!";
		else if (rojoResolver.getNetworkType(runtimeLibRbxPath) !== NetworkType.Unknown)
			fatal = "Runtime library cannot be in a server-only or client-only container!";
		else if (rojoResolver.isIsolated(runtimeLibRbxPath)) fatal = "Runtime library cannot be in an isolated container!";
	}
	if (fatal) runtimeLibRbxPath = undefined;

	out.setup = {
		warnings: warnings.slice(setupWarningStart),
		diagnostics,
		fatal,
		projectType,
		runtimeLibRbxPath: rbxPathJson(runtimeLibRbxPath),
		rojo: resolverJson(rojoResolver),
		pkgRojo: pkgRojoResolvers.map(resolverJson),
		nodeModulesPathMapping: [...mapping].sort((a, b) => (a[0] < b[0] ? -1 : 1)),
	};

	const lookup = p => {
		const rbxPath = rojoResolver.getRbxPathFromFilePath(p);
		return {
			rbxPath: rbxPathJson(rbxPath),
			network: rbxPath ? networkName(rojoResolver.getNetworkType(rbxPath)) : null,
			isolated: rbxPath ? rojoResolver.isIsolated(rbxPath) : null,
		};
	};
	out.files = sourceFiles.map(sf => {
		const fileName = sf.fileName;
		const outputPath = pathTranslator.getOutputPath(fileName);
		const declarationPath = pathTranslator.getOutputDeclarationPath(fileName);
		const importPath = pathTranslator.getImportPath(fileName);
		const nodeModule = ts.isInsideNodeModules(fileName);
		const mapped = mapping.get(getCanonicalFileName(path.normalize(fileName)));
		const nodeImportPath = pathTranslator.getImportPath(mapped ?? fileName, true);
		const output = lookup(outputPath);
		const f = {
			fileName,
			isDeclaration: sf.isDeclarationFile,
			isExternalLibrary: program.isSourceFileFromExternalLibrary(sf),
			outputPath,
			outputDeclarationPath: declarationPath,
			outputTransformedPath: pathTranslator.getOutputTransformedPath(fileName),
			inputPathsOfOutput: pathTranslator.getInputPaths(outputPath),
			inputPathsOfDeclaration: pathTranslator.getInputPaths(declarationPath),
			importPath,
			importPathNodeModule: nodeImportPath,
			output,
			rbxType: rbxTypeName(rojoResolver.getRbxTypeFromFilePath(outputPath)),
			import: lookup(importPath),
			nodeModuleImport: nodeModule ? lookup(nodeImportPath) : null,
			packageRelative: nodeModule ? rbxPathJson(pkgRojoResolvers.map(r => r.getRbxPathFromFilePath(nodeImportPath)).find(p => p)) : null,
		};
		if (output.rbxPath && runtimeLibRbxPath) {
			f.toRuntimeLib = relativeJson(RojoResolver.relative(output.rbxPath, runtimeLibRbxPath));
			f.runtimeLibRelation = ["OutToOut", "OutToIn", "InToOut", "InToIn"][rojoResolver.getFileRelation(output.rbxPath, runtimeLibRbxPath)];
		}
		return f;
	});
	// resolve_module probes: every package in the type roots, imported from the first project source file
	const from = sourceFiles.find(sf => !sf.isDeclarationFile && !program.isSourceFileFromExternalLibrary(sf));
	out.moduleProbes = [];
	if (from) {
		for (const root of options.typeRoots) {
			if (!fs.existsSync(root)) continue;
			for (const pkg of fs.readdirSync(root)) {
				const specifier = `${path.basename(root)}/${pkg}`;
				const r = ts.resolveModuleName(specifier, from.fileName, options, ts.sys).resolvedModule;
				out.moduleProbes.push({
					from: from.fileName,
					specifier,
					resolved: r ? { fileName: r.resolvedFileName, isExternalLibraryImport: !!r.isExternalLibraryImport, packageName: r.packageId?.name ?? null } : null,
				});
			}
		}
		for (const specifier of ["./does-not-exist", "@rbxts/not-a-package"]) {
			const r = ts.resolveModuleName(specifier, from.fileName, options, ts.sys).resolvedModule;
			out.moduleProbes.push({ from: from.fileName, specifier, resolved: r ? { fileName: r.resolvedFileName, isExternalLibraryImport: !!r.isExternalLibraryImport, packageName: r.packageId?.name ?? null } : null });
		}
	}
	out.timing = isRoot && timingRuns > 0 ? timeResolver(data.rojoConfigPath, timingRuns) : null;
	return out;
}

/** The dump of one build graph; also used by tools/gen-setup-fixtures.cjs. */
function dump(tsconfig, overrides = {}, timingRuns = 0) {
	const result = { tool: "dump-project-services", rbxtsDir: RT, cwd: process.cwd(), tsconfig, overrides, projects: [], error: null };
	warnings.length = 0;
	let graph;
	try {
		captureWarnings(() => {
			graph = new ProjectGraph(tsconfig, overrides);
		});
	} catch (e) {
		result.error = e.diagnostics ? e.diagnostics.map(d => diagnosticJson(d).message) : [String(e)];
	}
	result.graphWarnings = warnings.splice(0);
	if (graph) {
		for (const node of graph.projects.values()) {
			if (!node.pathTranslator) continue;
			result.projects.push(dumpProject(node, graph, node === graph.root, timingRuns));
		}
	}
	return result;
}

function main() {
	const { tsconfig, out, overrides, timingRuns } = parseArgs();
	const result = dump(tsconfig, overrides, timingRuns);
	fs.mkdirSync(path.dirname(out), { recursive: true });
	fs.writeFileSync(out, JSON.stringify(result, null, 1));
	const files = result.projects.reduce((n, p) => n + p.files.length, 0);
	console.log(`wrote ${out}: ${result.projects.length} project(s), ${files} files${result.error ? ", error: " + result.error.join(" | ") : ""}`);
}

if (require.main === module) main();
module.exports = { dump };
