// Generates the parity tables under crates/flamework_rs/tests/data from the node libraries and the
// built TS transformer (packages/transformer/out) of the Flamework monorepo. Run once, check the
// output in:
//
//   node crates/flamework_rs/tests/data/gen/generate.cjs <monorepo>
//
// <monorepo> defaults to E:/Projects/TS/Flamework/ExperimentalV2Monorepo. Nothing in it is written;
// throwaway files go to the OS temp directory and are removed at the end.
"use strict";
const fs = require("fs");
const os = require("os");
const path = require("path");
const crypto = require("crypto");
const { createRequire } = require("module");

const MONOREPO = path.resolve(process.argv[2] ?? "E:/Projects/TS/Flamework/ExperimentalV2Monorepo");
const TRANSFORMER = path.join(MONOREPO, "packages", "transformer");
const OUT = path.join(TRANSFORMER, "out");
const DATA = path.resolve(__dirname, "..");
const req = createRequire(path.join(TRANSFORMER, "package.json"));

const Hashids = req("hashids");
const uuid = req("uuid");
const glob = req("glob");
const { readProjectConfig, loadProjectConfig, describeMovedKeys, getRuntimeConfig, insertSchemaReference, getSchemaReference, assertTransformerEntry } =
	require(path.join(OUT, "util", "projectConfig.js"));
const { parseEnvFile, loadEnv } = require(path.join(OUT, "util", "env.js"));
const { BuildInfo } = require(path.join(OUT, "classes", "buildInfo.js"));
const { TransformState } = require(path.join(OUT, "classes", "transformState.js"));
const { layoutHash } = require(path.join(OUT, "util", "functions", "buildSerializerFromType.js"));
const isPathDescendantOf_1 = require(path.join(OUT, "util", "functions", "isPathDescendantOf.js"));

const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "fwrs-gen-"));
const written = [];
function save(name, value, compact = false) {
	const file = path.join(DATA, name);
	fs.writeFileSync(file, (compact ? JSON.stringify(value) : JSON.stringify(value, undefined, "\t")) + "\n");
	written.push(`${name} (${fs.statSync(file).size} bytes)`);
}

/** Deterministic pseudo-random bytes, so that a rerun gives the same tables. */
function bytes(label, length) {
	let out = Buffer.alloc(0);
	for (let i = 0; out.length < length; i++) {
		out = Buffer.concat([out, crypto.createHash("sha512").update(`${label}:${i}`).digest()]);
	}
	return out.subarray(0, length);
}
const hex = (label, length) => bytes(label, Math.ceil(length / 2)).toString("hex").slice(0, length);
const v4 = (label) => uuid.v4({ random: bytes(label, 16) });

function message(fn) {
	try {
		fn();
	} catch (error) {
		return String(error instanceof Error ? error.message : error);
	}
	return undefined;
}

// ---------------------------------------------------------------- hashids
{
	const series = [];
	for (let i = 0; i < 30; i++) {
		const salt = hex(`salt${i}`, 128);
		const h = new Hashids(salt, 2);
		const hashes = [];
		for (let n = 1; n <= 3000; n++) hashes.push(h.encode(n));
		series.push({ salt, minLength: 2, start: 1, hashes });
	}
	const edgeSalts = [
		"",
		"a",
		"aaaaaaaaaaaaaaaa",
		"abcabcabc",
		"0".repeat(128),
		"f".repeat(128),
		"salt with spaces",
		"ñandú€",
		"😀🎉x𝄞",
		hex("long", 2000),
		"cfhistuCFHISTU",
	];
	for (const salt of edgeSalts) {
		const h = new Hashids(salt, 2);
		const hashes = [];
		for (let n = 0; n <= 500; n++) hashes.push(h.encode(n));
		series.push({ salt, minLength: 2, start: 0, hashes });
	}
	const extra = [];
	const numberSets = [[0], [1], [99], [100], [2 ** 31], [2 ** 32], [2 ** 53 - 1], [1, 2, 3], [0, 0], [5000, 1, 77]];
	for (const salt of ["", hex("extra", 128), "😀🎉x"]) {
		for (const minLength of [0, 1, 2, 5, 12, 30]) {
			const h = new Hashids(salt, minLength);
			for (const numbers of numberSets) extra.push({ salt, minLength, numbers, hash: h.encode(numbers) });
		}
	}
	save("hashids.json", { series, extra }, true);
}

// ---------------------------------------------------------------- uuid v5
{
	const CALLSITE = "6f4c1d2e-8b3a-4e5f-9c7d-2a1b0e9f8d7c";
	const namespaces = [CALLSITE, uuid.v5.DNS, uuid.v5.URL, "00000000-0000-0000-0000-000000000000", CALLSITE.toUpperCase()];
	for (let i = 0; i < 5; i++) namespaces.push(v4(`seed${i}`));
	const names = [
		"",
		"a",
		"@:text",
		"addPaths:src/glob/**/*.ts",
		"transformer-fixture:src/networking.ts@Remotes+0",
		"transformer-fixture:src/networking.ts@+1234",
		"@flamework-experimental/core:src/flamework.ts@Flamework.createModule+987",
		"ünïcödé:src/ä/ö.ts@Ü+12",
		"emoji 😀🎉 and 𝄞",
		"line\nbreak\r\ttab",
		"x".repeat(5000),
		"€".repeat(1000),
	];
	const rows = [];
	for (const namespace of namespaces) for (const name of names) rows.push({ name, namespace, uuid: uuid.v5(name, namespace) });
	const invalid = [];
	for (const namespace of ["", "not-a-uuid", "6f4c1d2e-8b3a-6e5f-9c7d-2a1b0e9f8d7c", "6f4c1d2e-8b3a-4e5f-7c7d-2a1b0e9f8d7c", "6f4c1d2e8b3a4e5f9c7d2a1b0e9f8d7c", ` ${CALLSITE}`]) {
		invalid.push({ namespace, error: message(() => uuid.v5("a", namespace)) });
	}
	save("uuid.json", { rows, invalid });
}

// ---------------------------------------------------------------- sha1 / sha256 / layoutHash
{
	const inputs = ["", "a", "abc", "flamework", "ünïcödé 😀", "x".repeat(55), "x".repeat(56), "x".repeat(63), "x".repeat(64), "x".repeat(65), "y".repeat(1000), hex("big", 200000)];
	const rows = inputs.map((input) => ({
		input,
		sha1: crypto.createHash("sha1").update(input, "utf8").digest("hex"),
		sha256: crypto.createHash("sha256").update(input, "utf8").digest("hex"),
	}));
	const layouts = ["", "u8", "struct{a:u8,b:string32}", "union[0:literal(\"a\"),1:literal(\"b\")]", "ü😀", hex("layout", 5000)].map((description) => ({
		description,
		hash: layoutHash(description),
	}));
	save("hashes.json", { rows, layouts });
}

// ---------------------------------------------------------------- .env parser
{
	const texts = [
		[
			"# a comment",
			"",
			"PLAIN=value # trailing comment",
			'DOUBLE="a \\"quoted\\" line\\nnext"',
			"SINGLE='kept \\n as written'",
			"export EXPORTED=yes",
			"not a line",
			"SPACED =  padded  ",
		].join("\n"),
		"A=1\r\nB=2\r\n",
		"A=1\rB=2",
		"A=b#c\nB=b #c\nC=b\t#c\nD=#c\nE= #c",
		'A="x # y" # z\nB=\'p # q\'\nC="unclosed\nD=\'unclosed',
		'A="esc \\\\ \\t \\r \\x \\"q\\""\nB="trailing\\\\"\nC="ends with \\',
		"export A=1\nexportB=2\nexport =3\nexport\tC=4\n  export   D = 5  ",
		"\uFEFFBOM=1\nX=\uFEFFv\uFEFF",
		"DUP=1\nDUP=2\n_U=u\nlower=l\nA1=a1\n1A=bad\n-A=bad",
		"EMPTY=\nQEMPTY=\"\"\nSQEMPTY=''\n=x\nA",
		"UNI=ünïcödé 😀\nNBSP=\u00A0a\u00A0\nLS=a\u2028b\nNEL=\u0085a\u0085",
		"__proto__=p\nconstructor=c\ntoString=t",
		"A = \"x\"tail\nB='y'tail\nC=\"\"\"",
		"   \n\t\n#only\n   # indented comment\nK=v",
		"A=a=b=c\nB==x\nC=\"a\"=",
	];
	const rows = texts.map((text) => ({ text, entries: Object.entries(parseEnvFile(text)) }));

	// loadEnv precedence: files under TMP/env, the process environment given.
	const dir = path.join(TMP, "env");
	fs.mkdirSync(dir, { recursive: true });
	fs.writeFileSync(path.join(dir, ".env"), "FW_TEST_PREFIX=$a\nFW_TEST_SALT=from-env\nFW_TEST_SCOPES=a\nONLY_ENV=1\n");
	fs.writeFileSync(path.join(dir, ".env.local"), "FW_TEST_SALT=from-local\nFW_TEST_SCOPES=b\nONLY_LOCAL=2\n");
	const files = { ".env": fs.readFileSync(path.join(dir, ".env"), "utf8"), ".env.local": fs.readFileSync(path.join(dir, ".env.local"), "utf8") };
	const loads = [{}, { FW_TEST_SCOPES: "c" }, { FW_TEST_SCOPES: "c", NEW: "n", ONLY_ENV: "p" }].map((processEnv) => ({
		processEnv: Object.entries(processEnv),
		entries: Object.entries(loadEnv(dir, processEnv)),
	}));
	save("env.json", { rows, loadEnv: { files, loads } });
}

// ---------------------------------------------------------------- flamework.config.json
{
	const dir = path.join(TMP, "config");
	fs.mkdirSync(dir, { recursive: true });
	const file = path.join(dir, "flamework.config.json");
	const cases = [];
	const run = (name, text, env = {}) => {
		fs.writeFileSync(file, text);
		let result;
		const error = message(() => {
			result = JSON.stringify(readProjectConfig(file, env));
		});
		cases.push({ name, text, env: Object.entries(env), ...(error !== undefined ? { error: error.split(file).join("<FILE>") } : { result }) });
	};

	// projectConfig.test.ts, in order.
	run("comments and trailing commas", `{\n\t// like tsconfig\n\t"transformer": { "hashPrefix": "$c", "optimizations": { "guardGenerationDedupLimit": 4, }, },\n}`);
	run("drops $schema", `{ "$schema": "./x.json", "transformer": { "obfuscation": true } }`);
	run("unknown top-level key", `{ "hashPrefix": "$c" }`);
	run("unknown section key", `{ "transformer": { "hashPrefx": "$c" } }`);
	run("bad idGenerationMode", `{ "transformer": { "idGenerationMode": "medium" } }`);
	run("bad streamingMode", `{ "components": { "streamingMode": "Sometimes" } }`);
	run("serialization not a boolean", `{ "networking": { "serialization": "yes" } }`);
	run("watchRenames true", `{ "components": { "watchRenames": true } }`);
	run("watchRenames env false", `{ "components": { "watchRenames": "\${WATCH}" } }`, { WATCH: "false" });
	run("watchRenames env sometimes", `{ "components": { "watchRenames": "\${WATCH}" } }`, { WATCH: "sometimes" });
	run("watchRenames 1", `{ "components": { "watchRenames": 1 } }`);
	run("checks all", `{ "serialization": { "checks": { "category": "all", "mode": "warn", "side": "server" } } }`);
	run("checks empty", `{ "serialization": { "checks": {} } }`);
	run("checks mode env", `{ "serialization": { "checks": { "mode": "\${MODE:-assert}" } } }`, { MODE: "warn" });
	run("checks mode fallback", `{ "serialization": { "checks": { "mode": "\${MODE:-assert}" } } }`);
	run("checks categry", `{ "serialization": { "checks": { "categry": "all" } } }`);
	run("serialization check", `{ "serialization": { "check": {} } }`);
	run("category strict", `{ "serialization": { "checks": { "category": "strict" } } }`);
	run("mode raise", `{ "serialization": { "checks": { "mode": "raise" } } }`);
	run("side Server", `{ "serialization": { "checks": { "side": "Server" } } }`);
	run("side true", `{ "serialization": { "checks": { "side": true } } }`);
	run("types true", `{ "serialization": { "checks": { "types": true, "mode": "warn" } } }`);
	for (const TYPES of [undefined, "true", "1", "sometimes"]) {
		run(`types env ${TYPES}`, `{ "serialization": { "checks": { "types": "\${TYPES:-false}" } } }`, TYPES === undefined ? {} : { TYPES });
	}
	run("types yes", `{ "serialization": { "checks": { "types": "yes" } } }`);
	run("types 1", `{ "serialization": { "checks": { "types": 1 } } }`);
	run("checks type", `{ "serialization": { "checks": { "type": true } } }`);
	run("concurrency 2", `{ "testing": { "concurrency": 2 } }`);
	for (const CONCURRENCY of [undefined, "1", "many", " 7 ", "0x10", "1e1", "Infinity", "", "  ", "1_0", ".5", "5.", "-3"]) {
		run(`concurrency env ${JSON.stringify(CONCURRENCY)}`, `{ "testing": { "concurrency": "\${CONCURRENCY:-4}" } }`, CONCURRENCY === undefined ? {} : { CONCURRENCY });
	}
	for (const bad of ["0", "1.5", "-2"]) run(`concurrency ${bad}`, `{ "testing": { "concurrency": ${bad} } }`);
	run("parallel 2", `{ "flameworkTest": { "parallel": 2 } }`);
	for (const PARALLEL of [undefined, "4", "lots"]) run(`parallel env ${PARALLEL}`, `{ "flameworkTest": { "parallel": "\${PARALLEL:-1}" } }`, PARALLEL === undefined ? {} : { PARALLEL });
	for (const bad of ["0", "1.5", "-2"]) run(`parallel ${bad}`, `{ "flameworkTest": { "parallel": ${bad} } }`);
	run("showWindows true", `{ "flameworkTest": { "showWindows": true } }`);
	for (const SHOW of [undefined, "1", "sometimes", "TRUE", " false "]) run(`showWindows env ${SHOW}`, `{ "flameworkTest": { "showWindows": "\${SHOW:-false}" } }`, SHOW === undefined ? {} : { SHOW });
	run("showWindows yes", `{ "flameworkTest": { "showWindows": "yes" } }`);
	run("showWindows 1", `{ "flameworkTest": { "showWindows": 1 } }`);
	run("moved testing keys", JSON.stringify({ testing: { timeout: 5, failOnSkip: true, keepAwake: true, lockTimeout: 60, lockHold: 30, parallel: 2, showWindows: true } }));
	run("moved cloud keys", JSON.stringify({ cloud: { testingUniverseId: "1", testingPlaceId: "2", apiKey: "${MISSING_KEY_FOR_THE_REFUSAL}", originalPlace: "original.rbxl" } }));
	run("cloud empty", `{ "cloud": {} }`);
	run("cloud null", `{ "cloud": null }`);
	run("cloud array", `{ "cloud": [1] }`);
	run("cloud other keys", `{ "cloud": { "other": 1, "apiKey": "k" } }`);
	run("testing moved and cloud", `{ "testing": { "parallel": 2 }, "cloud": { "apiKey": "k" } }`);
	run("testing null with cloud", `{ "testing": null, "cloud": "x" }`);
	run("flameworkTest every key", JSON.stringify({ flameworkTest: { failOnSkip: true, keepAwake: false, lockTimeout: 60, lockHold: 30, parallel: 2, showWindows: true, universeId: "1", placeId: "2", apiKey: "key", originalPlace: "original.rbxl" } }));
	run("flameworkTest testingPlaceId", `{ "flameworkTest": { "testingPlaceId": "2" } }`);
	run("parse error", `{ "transformer": `);
	run("hashPrefix 1", `{ "transformer": { "hashPrefix": 1 } }`);
	run("unset flag", `{ "transformer": { "obfuscation": "\${FW_UNSET_FLAG}" } }`);
	run("obfuscation maybe", `{ "transformer": { "obfuscation": "maybe" } }`);
	run("env prefix and salt", `{ "transformer": { "hashPrefix": "\${PREFIX}", "salt": "\${SALT:-fixed}" } }`, { PREFIX: "$e" });
	run("env prefix and salt given", `{ "transformer": { "hashPrefix": "\${PREFIX}", "salt": "\${SALT:-fixed}" } }`, { PREFIX: "$e", SALT: "given" });
	const conv = `{\n\t"transformer": { "obfuscation": "\${OBFUSCATE:-false}", "optimizations": { "guardGenerationDedupLimit": "\${DEDUP}" } },\n\t"scopes": { "active": "\${SCOPES:-}" }\n}`;
	run("convert all", conv, { OBFUSCATE: "TRUE", DEDUP: "4", SCOPES: "components, providers" });
	run("convert empty list", conv, { DEDUP: "2", SCOPES: "" });
	run("convert star", conv, { DEDUP: "2", SCOPES: "*" });
	run("convert list edges", conv, { DEDUP: "2", SCOPES: " a ,, b , ,c," });
	run("convert dedup 0", conv, { DEDUP: "0" });
	run("convert dedup 2.5", conv, { DEDUP: "2.5" });
	run("obfuscate maybe", `{ "transformer": { "obfuscation": "\${OBFUSCATE}" } }`, { OBFUSCATE: "maybe" });
	run("timeout soon", `{ "components": { "warningTimeout": "\${TIMEOUT}" } }`, { TIMEOUT: "soon" });
	run("prefix unset", `{ "transformer": { "hashPrefix": "\${PREFIX}" } }`);
	run("literal dollar", `{ "transformer": { "hashPrefix": "$$g", "salt": "plain" } }`);

	// More: placeholders.
	run("placeholder forms", `{ "flameworkTest": { "universeId": "$\${A}", "placeId": "\${A}\${B:-b}$$\${C:-}", "apiKey": "\${1A} \${A \${A:-x}y} $", "originalPlace": "\${B:-a}b}" } }`, { A: "1" });
	run("placeholder in list", `{ "scopes": { "active": ["\${A}", "\${B:-x,y}", "z"] } }`, { A: "a,b" });
	run("placeholder in key", `{ "\${A}": 1 }`, { A: "transformer" });
	run("placeholder first missing in order", `{ "testing": { "entry": "\${MISSING_B}" }, "core": { "profiling": "\${MISSING_A}" } }`);
	run("placeholder empty value", `{ "transformer": { "salt": "\${EMPTY}" } }`, { EMPTY: "" });
	run("placeholder unicode", `{ "transformer": { "salt": "\${U}ü" } }`, { U: "😀" });

	// More: validation order and messages.
	run("two unknown top keys", `{ "zeta": 1, "alpha": 2 }`);
	run("unknown and bad", `{ "core": { "profiling": 1 }, "bogus": 1 }`);
	run("bad in two sections, schema order", `{ "core": { "profiling": 1 }, "transformer": { "salt": 1 } }`);
	run("bad in one section, schema order", `{ "transformer": { "obfuscation": "x1", "salt": 1 } }`);
	run("bad after unknown inside", `{ "transformer": { "salt": 1, "nope": 2 } }`);
	run("root array", `[]`);
	run("root string", `"x"`);
	run("root null", `null`);
	run("root number", `12`);
	run("section not object", `{ "transformer": [] }`);
	run("section null", `{ "transformer": null }`);
	run("section string", `{ "core": "x" }`);
	run("$schema not string", `{ "$schema": 1 }`);
	run("plugins ok", `{ "transformer": { "plugins": ["./a.cjs", { "path": "b", "options": { "x": 1 } }] } }`);
	run("plugins number", `{ "transformer": { "plugins": [1] } }`);
	run("plugins object without path", `{ "transformer": { "plugins": [{ "options": {} }] } }`);
	run("plugins object extra", `{ "transformer": { "plugins": [{ "path": "a", "extra": 1 }] } }`);
	run("plugins options not object", `{ "transformer": { "plugins": [{ "path": "a", "options": [] }] } }`);
	run("plugins not array", `{ "transformer": { "plugins": "a,b" } }`);
	run("plugins second bad", `{ "transformer": { "plugins": ["a", 2, 3] } }`);
	run("dedup 0", `{ "transformer": { "optimizations": { "guardGenerationDedupLimit": 0 } } }`);
	run("dedup string", `{ "transformer": { "optimizations": { "guardGenerationDedupLimit": "x" } } }`);
	run("dedup float", `{ "transformer": { "optimizations": { "guardGenerationDedupLimit": 2.5 } } }`);
	run("optimizations unknown", `{ "transformer": { "optimizations": { "other": 1 } } }`);
	run("warningTimeout -1", `{ "components": { "warningTimeout": -1 } }`);
	run("warningTimeout string", `{ "components": { "warningTimeout": "x1" } }`);
	run("warningTimeout bool", `{ "components": { "warningTimeout": true } }`);
	run("attributeWarningTimeout", `{ "components": { "attributeWarningTimeout": 3 } }`);
	run("lockHold 0.5", `{ "flameworkTest": { "lockHold": 0.5 } }`);
	run("active not list", `{ "scopes": { "active": 1 } }`);
	run("active item number", `{ "scopes": { "active": ["a", 1] } }`);
	run("active string split", `{ "scopes": { "active": "a,b" } }`);
	run("activeIn string", `{ "testing": { "activeIn": "x, y", "inactiveIn": "" } }`);
	run("entry ok", `{ "testing": { "entry": "src/server/main", "enabled": "\${E:-0}" } }`);
	run("everything", JSON.stringify({ transformer: { plugins: [], noSemanticDiagnostics: false, salt: "s", hashPrefix: "h", obfuscation: false, idGenerationMode: "tiny", optimizations: {} }, serialization: { checks: { category: "none", mode: "assert", side: "both", types: false } }, core: { profiling: true }, networking: { serialization: false }, components: { warningTimeout: 0, attributeWarningTimeout: 1.5, streamingMode: "Watching", watchRenames: false }, scopes: { active: ["*"] }, testing: { enabled: true, activeIn: [], inactiveIn: ["x"], autoRun: true, timeout: 0, concurrency: 1, entry: "e" } }));
	run("key order kept", `{ "testing": { "timeout": 1, "autoRun": true }, "core": {}, "transformer": { "salt": "s", "hashPrefix": "h" } }`);
	run("pointer escaping", `{ "a/b": 1 }`);
	run("pointer escaping 2", `{ "core": { "a~b/c": 1 } }`);
	run("integer-like keys", `{ "core": {}, "1": 1, "0": 0 }`);
	run("duplicate keys", `{ "transformer": { "salt": "a" }, "core": {}, "transformer": { "salt": "b" } }`);
	run("numbers", `{ "testing": { "timeout": 1e1, "concurrency": 3.0 }, "components": { "warningTimeout": -0, "attributeWarningTimeout": 0.000001 } }`);
	run("number forms", `{ "testing": { "timeout": .5 } }`);
	run("number hex", `{ "testing": { "timeout": 0x10 } }`);
	run("number huge", `{ "testing": { "timeout": 1e400 } }`);
	run("negative minimum", `{ "testing": { "timeout": -5 } }`);

	// More: parsing.
	run("empty file", ``);
	run("whitespace only", ` \n\t `);
	run("comment only", `// nothing\n/* here */`);
	run("bom", `\uFEFF{ "core": {} }`);
	run("block comments", `{ /* a */ "core" /* b */ : /* c */ { } /* d */ }`);
	run("single quotes", `{ 'core': { 'profiling': true } }`);
	run("unquoted key", `{ core: {} }`);
	run("missing comma", `{ "core": {} "networking": {} }`);
	run("missing colon", `{ "core" {} }`);
	run("missing value", `{ "core": }`);
	run("unterminated string", `{ "core": { "profiling": "tr`);
	run("unterminated object", `{ "core": { "profiling": true }`);
	run("unterminated comment", `{ "core": {} /* never`);
	run("trailing garbage", `{ "core": {} } x`);
	run("two objects", `{ "core": {} } { }`);
	run("double comma", `{ "core": {},, "networking": {} }`);
	run("leading comma", `{ , "core": {} }`);
	run("array trailing comma", `{ "scopes": { "active": ["a", "b",] } }`);
	run("array double comma", `{ "scopes": { "active": ["a",, "b"] } }`);
	run("unclosed array", `{ "scopes": { "active": ["a" } }`);
	run("escapes", `{ "flameworkTest": { "apiKey": "a\\u00e9\\n\\t\\"\\\\\\/" } }`);
	run("bad escape", `{ "flameworkTest": { "apiKey": "a\\qb" } }`);
	run("raw newline in string", `{ "flameworkTest": { "apiKey": "a\nb" } }`);
	run("true as key", `{ true: 1 }`);
	run("expression value", `{ "testing": { "timeout": 1 + 2 } }`);
	run("identifier value", `{ "testing": { "timeout": abc } }`);
	run("negative via minus", `{ "testing": { "timeout": - 5 } }`);
	run("crlf", `{\r\n\t"core": {\r\n\t\t"profiling": false\r\n\t}\r\n}\r\n`);
	run("unicode values", `{ "flameworkTest": { "apiKey": "ü😀\u2028" } }`);
	save("config.json", { cases });
}

// ---------------------------------------------------------------- locating and entries
{
	const root = path.join(TMP, "entry");
	const place = path.join(root, "places", "a");
	const reset = () => {
		fs.rmSync(root, { recursive: true, force: true });
		fs.mkdirSync(place, { recursive: true });
		fs.writeFileSync(path.join(root, "package.json"), JSON.stringify({ name: "config-test" }));
	};
	const TRANSFORM = "@flamework-experimental/transformer";
	const cases = [];
	const run = (name, files, projectDir, entry, env = {}) => {
		reset();
		for (const [relative, text] of Object.entries(files)) {
			const file = path.join(root, relative);
			fs.mkdirSync(path.dirname(file), { recursive: true });
			fs.writeFileSync(file, text);
		}
		let result;
		const error = message(() => {
			const loaded = loadProjectConfig(projectDir === "root" ? root : place, root, entry, env);
			result = {
				configPath: loaded.configPath === undefined ? null : loaded.configPath.split(root).join("<ROOT>"),
				config: JSON.stringify(loaded.config),
				project: JSON.stringify(loaded.project),
				env: Object.entries(loaded.env),
			};
		});
		cases.push({ name, files: Object.entries(files), projectDir, entry: JSON.stringify(entry), env: Object.entries(env), ...(error !== undefined ? { error: error.split(root).join("<ROOT>") } : { result }) });
	};
	run("in the tsconfig directory", { "places/a/flamework.config.json": "{}" }, "place", {});
	run("walks up", { "flamework.config.json": `{ "core": {} }` }, "place", {});
	run("nearest wins", { "flamework.config.json": `{ "core": {} }`, "places/flamework.config.json": `{ "networking": {} }` }, "place", {});
	run("none", {}, "place", {});
	run("not above the root", { "../flamework.config.json": "{}" }, "place", {});
	run("explicit configFile", { "places/a/config/fw.json": `{ "transformer": { "hashPrefix": "$x" } }` }, "place", { configFile: "config/fw.json" });
	run("explicit missing", {}, "place", { configFile: "missing.json" });
	run("explicit outside", { "other.json": `{ "core": {} }` }, "place", { configFile: "../../other.json" });
	run("configFile not a path", {}, "place", { transform: TRANSFORM, configFile: true });
	run("configFile null", {}, "place", { transform: TRANSFORM, configFile: null });
	for (const [key, value] of [["plugins", ["./plugin.cjs"]], ["noSemanticDiagnostics", true], ["salt", "s"], ["hashPrefix", "$g"], ["obfuscation", true], ["idGenerationMode", "short"], ["optimizations", { guardGenerationDedupLimit: 3 }]]) {
		run(`entry option ${key}`, { "places/a/flamework.config.json": `{ "transformer": { "obfuscation": false } }` }, "place", { transform: TRANSFORM, [key]: value });
	}
	run("entry option, new file", {}, "place", { transform: TRANSFORM, obfuscation: true });
	run("entry option, configFile", { "places/a/config/fw.json": "{}" }, "place", { transform: TRANSFORM, configFile: "config/fw.json", hashPrefix: "$g" });
	run("entry unknown keys", {}, "place", { transform: TRANSFORM, $rbxpackmode$: true, preloadIds: true, hashPrefx: "$g" });
	run("entry both", {}, "place", { transform: TRANSFORM, obfuscation: true, hashPrefix: "$g", $rbxpackmode$: true });
	run("entry undefined-valued key ignored", {}, "place", { transform: TRANSFORM, salt: undefined });
	run("entry null-valued key", {}, "place", { transform: TRANSFORM, salt: null });
	run("loader keys", { "places/a/flamework.config.json": `{ "transformer": { "hashPrefix": "$file" } }` }, "place", {
		transform: TRANSFORM, import: "default", type: "program", after: false, afterDeclarations: false, name: "flamework", transformProgram: false, isEsm: false, tsConfig: "./tsconfig.json", resolvePathAliases: false,
	});
	run("configFile with every option", { "places/a/config/fw.json": `{ "transformer": { "hashPrefix": "$x", "obfuscation": true, "optimizations": { "guardGenerationDedupLimit": 3 } } }` }, "place", { transform: TRANSFORM, configFile: "config/fw.json" });
	run("env files next to the config", { "flamework.config.json": `{ "transformer": { "hashPrefix": "\${FW_TEST_PREFIX}", "salt": "\${FW_TEST_SALT}" }, "scopes": { "active": "\${FW_TEST_SCOPES}" } }`, ".env": "FW_TEST_PREFIX=$a\nFW_TEST_SALT=from-env\nFW_TEST_SCOPES=a\n", ".env.local": "FW_TEST_SALT=from-local\nFW_TEST_SCOPES=b\n" }, "root", {}, { FW_TEST_SCOPES: "c" });
	run("env next to the tsconfig without a config", { "places/a/.env": "FW_FP_LONE=yes\n", ".env": "ROOT=1\n" }, "place", {});
	run("env next to the config, not the tsconfig", { "flamework.config.json": "{}", "places/a/.env": "PLACE=1\n", ".env": "ROOT=1\n" }, "place", {});
	save("entry.json", { cases });
}

// ---------------------------------------------------------------- $schema line, moved keys, runtime sections
{
	const S = "./s.json";
	const inserts = [
		`{\n\t"core": { "profiling": true }\n}\n`,
		`{\r\n  "core": {},\r\n  "networking": {}\r\n}\r\n`,
		`{\n\t// the transformer\n\t"transformer": { "obfuscation": true, },\n}`,
		`{ // settings\n\t"core": {}\n}`,
		`{\n\n    "core": {}\n}`,
		`\uFEFF{\n\t"core": {}\n}`,
		`{ "core": {} }`,
		`{}`,
		`{ }\n`,
		`{\r\n}\r\n`,
		`{ "$schema": "https://example.com/other.json" }`,
		`{\n\t"core": {},\n\t"$schema": "../x.json"\n}`,
		`{ "core": `,
		`[]`,
		`{ /* never\nclosed "core": {} }`,
		`  {\n\t\t"core": {}\n}`,
		`{\n  \n\t\n  "a": 1\n}`,
		`{\n}`,
		`{\n\n}`,
		`// lead\n{ "a": 1 }`,
		`{"a":1}`,
		`{\t"a": 1,\n\t"b": 2 }`,
		`{ 'a': 1 }`,
		`{ "a": 1 } // tail`,
		`""`,
		``,
	].map((text) => ({ text, result: insertSchemaReference(text, S) ?? null }));
	const refs = [];
	for (const [config, schema] of [
		["C:\\p\\flamework.config.json", "C:\\p\\node_modules\\@flamework-experimental\\transformer\\flamework.config.schema.json"],
		["C:\\p\\places\\a\\config\\fw.json", "C:\\p\\node_modules\\@flamework-experimental\\transformer\\flamework.config.schema.json"],
		["C:\\p\\flamework.config.json", "D:\\x\\flamework.config.schema.json"],
		["C:\\p\\flamework.config.json", "c:\\P\\s.json"],
		["C:\\p\\flamework.config.json", "C:\\p\\s.json"],
		["C:\\p\\a\\flamework.config.json", "C:\\p\\a b\\ü s.json"],
		["C:\\p\\flamework.config.json", "C:\\p\\..x\\s.json"],
		["C:\\p\\flamework.config.json", "D:\\x y\\ü#?%.json"],
	]) refs.push({ config, schema, result: getSchemaReference(config, schema) });
	const moved = [
		{},
		{ testing: { timeout: 1, parallel: 2, failOnSkip: 1 } },
		{ cloud: { testingPlaceId: "1", other: 1 } },
		{ cloud: null },
		{ cloud: [] },
		{ cloud: "x", testing: [] },
		{ testing: { showWindows: 1, keepAwake: 1, lockTimeout: 1, lockHold: 1, parallel: 1 } },
		[],
		null,
	].map((config) => ({ config: JSON.stringify(config), lines: describeMovedKeys(config) }));
	const runtime = [
		{ transformer: { hashPrefix: "$x" }, serialization: { checks: { category: "all" } }, core: { profiling: false }, networking: { serialization: true }, scopes: { active: ["a"] } },
		{ transformer: { obfuscation: true } },
		{ testing: { timeout: 5, concurrency: 2 }, flameworkTest: { keepAwake: true } },
		{ scopes: { active: [] }, components: { warningTimeout: 2 }, core: {} },
		{},
	].map((project) => ({ project: JSON.stringify(project), runtime: JSON.stringify(getRuntimeConfig(project) ?? null) }));
	save("schema-ref.json", { inserts, refs, moved, runtime });
}

// ---------------------------------------------------------------- flamework.build
{
	const file = path.join(TMP, "broken.build");
	const valid = JSON.stringify({ version: 1, flameworkVersion: "2.0.0", identifiers: {} }, undefined, "\t");
	const problems = [];
	const refusal = (text) => {
		fs.writeFileSync(file, text);
		problems.push({ text, error: message(() => BuildInfo.fromPath(file, "broken.build", ["Do this."])) ?? null });
	};
	for (const text of [
		valid.slice(0, 30),
		`<<<<<<< HEAD\n${valid}\n=======\n${valid}\n>>>>>>> other\n`,
		"",
		"  \n\t",
		`{ "version": "1", "identifiers": [] }`,
		"[1, 2]",
		`{ "version": 1, "flameworkVersion": "2.0.0", "identifiers": { "a": 1 } }`,
		valid,
		"{",
		"{}",
		"null",
		"1",
		'"x"',
		`{ "version": 1, "flameworkVersion": 2, "identifiers": {} }`,
		`{ "version": 1, "flameworkVersion": "2", "identifiers": {}, "salt": 5 }`,
		`{ "version": 1, "flameworkVersion": "2", "identifiers": {}, "extra": 5, "metadata": { "globs": { "paths": { "a": [1] } } } }`,
		`{ "version": 1, "flameworkVersion": "2", "identifiers": {}, "metadata": { "globs": { "uses": { "f": [{ "glob": "g", "text": "t", "macro": "m", "line": 1 }] } } } }`,
		`{ "version": 1, "flameworkVersion": "2", "identifiers": {}, "metadata": { "paths": { "uses": { "f": [{ "path": "p", "macro": "m", "line": 1, "column": 2, "raises": "yes" }] } } } }`,
		`{ "version": 1, "flameworkVersion": "2", "identifiers": {}, "metadata": [] }`,
		`{ "version": 1, "flameworkVersion": "2", "identifiers": {}, "stringHashes": { "a": null } }`,
		`{ "version": 1e400, "flameworkVersion": "2", "identifiers": {} }`,
		`{ "version": 1, "flameworkVersion": "2", "identifiers": {}, }`,
		`{ "version": 1 "flameworkVersion": "2" }`,
		`{ 'version': 1 }`,
		`{ "a": tru }`,
		`{ "a": "\u0001" }`,
		`{ "a": "\\x" }`,
		`{ "a": "\\u12" }`,
		`{ "a": -x }`,
		`{ "a": 1.e5 }`,
		`{ "a": 01 }`,
		`{ "a": 1e }`,
		`{ "a": "unterminated`,
		`[1,]`,
		`{"a" 1}`,
		`{} x`,
		"\uFEFF{}",
		`{ "version": 1, "flameworkVersion": "2", "identifiers": {}, "version": "x" }`,
	]) refusal(text);

	// Field order and bytes for a sequence of operations on a fresh build info.
	const sequences = [];
	const seq = (name, ops) => {
		const info = new BuildInfo(path.join(TMP, "seq.build"));
		const results = [];
		for (const [op, ...args] of ops) {
			const value = info[op](...args);
			results.push(value === undefined ? null : value);
		}
		info.save();
		sequences.push({ name, ops, results, text: fs.readFileSync(path.join(TMP, "seq.build"), "utf8") });
	};
	seq("fresh", []);
	seq("game, full", [["setIdGenerationMode", "full"], ["setIdentifierPrefix", undefined], ["addIdentifier", "g:out/a@A", "out/a@A"], ["getLatestId"]]);
	seq("package", [["setIdGenerationMode", "full"], ["setIdentifierPrefix", "@s/p"], ["addIdentifier", "@s/p:out/a@A", "@s/p:a@A"], ["addPathUse", "src/x.ts", { path: "src/a", macro: "registerProviders", line: 3, column: 9 }]]);
	seq("globs", [
		["setIdGenerationMode", "short"],
		["setIdentifierPrefix", undefined],
		["addGlob", "src/a/**", "src/x.ts", { glob: "src/a/**", text: "./a/**", macro: "registerProvidersGlob", line: 1, column: 2 }],
		["addGlob", "src/b/*.ts", "src/x.ts"],
		["addGlob", "src/a/**", "src/y.ts", { glob: "src/a/**", text: "src/a/**", macro: "fromGlob", line: 10, column: 20 }],
		["addPathUse", "src/y.ts", { path: "src/c", macro: "requireModules", line: 4, column: 1, raises: true }],
		["addIdentifier", "g:out/a@A", "a@A{x}"],
		["setMetadata", "paths", { uses: {} }],
		["getLatestId"],
	]);
	seq("mode change drops", [["setIdGenerationMode", "full"], ["addIdentifier", "a", "b"], ["setIdGenerationMode", "tiny"], ["getIdentifierFromInternal", "a"], ["getLatestId"]]);
	seq("unicode and escapes", [["setIdentifierPrefix", "ü\"\\\n\u0001\u007f\u2028😀"], ["addIdentifier", "k\t", "v\u001f"]]);
	seq("integer-like keys", [["addIdentifier", "b", "1"], ["addIdentifier", "10", "2"], ["addIdentifier", "2", "3"], ["addIdentifier", "01", "4"], ["addIdentifier", "4294967295", "5"], ["addIdentifier", "4294967294", "6"]]);
	seq("invalidate", [
		["addGlob", "g1", "f1"], ["addGlob", "g2", "f1"], ["addGlob", "g2", "f2"], ["addGlob", "g3", "f3", { glob: "g3", text: "g3", macro: "m", line: 1, column: 1 }],
		["invalidateGlobs", "f1"], ["invalidateGlobs", "f3"], ["addPathUse", "f1", { path: "p", macro: "m", line: 1, column: 1 }], ["invalidatePathUses", "f1"],
	]);

	// The repository's own flamework.build files, which must read and write back to the same bytes.
	const realDir = path.join(DATA, "buildinfo");
	fs.mkdirSync(realDir, { recursive: true });
	const real = [];
	for (const relative of [
		"packages/core/flamework.build",
		"packages/components/flamework.build",
		"packages/networking/flamework.build",
		"packages/testing/flamework.build",
		"packages/specs/flamework.build",
		"packages/specs/variants/types/flamework.build",
		"packages/transformer/tests/fixture/flamework.build",
		"tests/place/flamework.build",
	]) {
		const source = path.join(MONOREPO, relative);
		if (!fs.existsSync(source)) continue;
		const name = relative.replace(/\/flamework\.build$/, "").replace(/[\/]/g, "_") + ".build";
		fs.copyFileSync(source, path.join(realDir, name));
		const text = fs.readFileSync(source, "utf8");
		real.push({ name, rewrites: JSON.stringify(JSON.parse(text), undefined, "\t") === text });
	}
	// refuseMixedVersions is private to transformState.js: evaluated from its compiled text with the real constants.
	const stateSource = fs.readFileSync(path.join(OUT, "classes", "transformState.js"), "utf8");
	const refuseSource = /function refuseMixedVersions\([\s\S]*?\n}\n/.exec(stateSource)[0];
	const refuseMixedVersions = new Function(
		"__spreadArray",
		"__read",
		"diagnostics_1",
		"packages_1",
		"constants_1",
		`${refuseSource}\nreturn refuseMixedVersions;`,
	)(
		(to, from) => to.concat(from),
		(o) => [...o],
		require(path.join(OUT, "classes", "diagnostics.js")),
		require(path.join(OUT, "util", "packages.js")),
		require(path.join(OUT, "util", "constants.js")),
	);
	const mixed = [
		[],
		[{ name: "@flamework-experimental/core", version: "2.0.0-alpha.7", file: "node_modules/@flamework-experimental/core/flamework.build" }],
		[
			{ name: "@flamework-experimental/core", version: "2.0.0-alpha.7", file: "a/flamework.build" },
			{ name: "@flamework-experimental/networking", version: "2.0.0-alpha.7", file: "b/flamework.build" },
		],
		[
			{ name: "@flamework-experimental/core", version: "2.0.0-alpha.6", file: "a/flamework.build" },
			{ name: "@flamework-experimental/components", version: "2.0.0-alpha.7", file: "c/flamework.build" },
		],
	].map((packages) => ({ packages, error: message(() => refuseMixedVersions(packages)) ?? null }));
	save("buildinfo.json", { problems, sequences, real, mixed });
}

// ---------------------------------------------------------------- ids
{
	const uidSource = fs.readFileSync(path.join(OUT, "util", "uid.js"), "utf8");
	const extract = (name) => {
		const match = new RegExp(`function ${name}\\(([\\s\\S]*?)\\n}\\n`).exec(uidSource);
		if (!match) throw new Error(`no ${name} in uid.js`);
		return match[0];
	};
	const __read = (o, n) => [...o].slice(0, n);
	const fns = new Function(
		"__read",
		"path_1",
		"getDeclarationName_1",
		"getPackageJson_1",
		"isPathDescendantOf_1",
		`${extract("formatInternalid")}\n${extract("getShortId")}\n${extract("getInternalId")}\nreturn { formatInternalid, getShortId, getInternalId };`,
	)(__read, { default: path }, { getDeclarationName: (node) => node.fullName }, {
		getPackageJson: (directory) => fakePackages(directory),
	}, isPathDescendantOf_1);
	let fakePackages;

	const formats = [];
	for (const internalId of [
		"pkg:out/a@A",
		"pkg:out/dir/sub/a@A.B",
		"pkg:a@A",
		"pkg:@A",
		"pkg:out/a@",
		"pkg:out/a@@",
		"pkg:out/a@b@C",
		"@scope/pkg:out/a@A",
		"a:b:c/d@E",
		"pkg:out\\win\\a@A",
		"no-colon@A",
		"pkg:no-at",
		"",
		"pkg:out/ü/😀@Ä",
		"pkg:out/a@A\nB",
		"pkg:out/a\n@A",
		"pk\ng:out/a@A",
		"pkg:out/a@A:B",
		"pkg:out/a@$p:error",
		":@x",
		"pkg:/lead@A",
	]) {
		for (const prefix of [undefined, "", "fw", "@s/p", "$c"]) {
			formats.push({ internalId, prefix: prefix ?? null, result: fns.formatInternalid({ config: {} }, internalId, prefix) });
		}
	}

	// The id each declaration gets, in request order, in each mode: getDeclarationUid's logic, with
	// the transformer's own formatInternalid, getShortId, TransformState.hash and BuildInfo.
	const scenarios = [];
	const declarations = [
		{ internalId: "game:out/server/a@A", fullName: "A", fileName: "C:\\p\\src\\server\\a.ts" },
		{ internalId: "game:out/server/a@NS.B", fullName: "NS.B", fileName: "C:\\p\\src\\server\\a.ts" },
		{ internalId: "game:out/shared/index@C", fullName: "C", fileName: "C:\\p\\src\\shared\\index.ts" },
		{ internalId: "game:out/server/a@A", fullName: "A", fileName: "C:\\p\\src\\server\\a.ts" },
		{ internalId: "game:out/types@D", fullName: "D", fileName: "C:\\p\\src\\types.d.ts" },
		{ internalId: "@s/lib:out/x@P", fullName: "P", fileName: "C:\\p\\node_modules\\@s\\lib\\out\\x.d.ts", isPackage: true },
		{ internalId: "@s/other:out/y@Q", fullName: "Q", fileName: "C:\\p\\node_modules\\@s\\other\\out\\y.d.ts", isPackage: true, packagePrefix: "@s/other" },
		{ internalId: "game:out/ü@Ü", fullName: "Ü", fileName: "C:\\p\\src\\ü.ts" },
	];
	for (const mode of ["full", "short", "tiny", "obfuscated"]) {
		for (const [isGame, packageName, hashPrefix] of [[true, "game", undefined], [true, "game", "g"], [false, "@s/me", "@s/me"], [false, "@s/me", ""]]) {
			for (const salt of [hex("idsalt", 128), "", "😀"]) {
				const buildInfo = new BuildInfo("C:\\p\\flamework.build");
				buildInfo.setIdGenerationMode(mode);
				buildInfo.setIdentifierPrefix(hashPrefix);
				const state = { config: { idGenerationMode: mode, hashPrefix, salt }, buildInfo, isGame, packageName };
				state.hash = (id, noPrefix) => TransformState.prototype.hash.call(state, id, noPrefix);
				const ids = [];
				for (const d of declarations) {
					const node = { fullName: d.fullName, getSourceFile: () => ({ fileName: d.fileName }) };
					let id = buildInfo.getIdentifierFromInternal(d.internalId);
					if (!id) {
						if (d.isPackage) {
							id = d.packagePrefix ? fns.formatInternalid(state, d.internalId, d.packagePrefix) : d.internalId;
						} else {
							if (mode === "obfuscated") id = state.hash(buildInfo.getLatestId());
							else if (mode === "short" || mode === "tiny") id = fns.getShortId(state, node);
							else id = fns.formatInternalid(state, d.internalId);
							buildInfo.addIdentifier(d.internalId, id);
						}
					}
					ids.push(id);
				}
				buildInfo.save = function () {};
				scenarios.push({ mode, isGame, packageName, hashPrefix: hashPrefix ?? null, salt, ids, identifiers: Object.entries(JSON.parse(JSON.stringify(buildInfo.buildInfo)).identifiers) });
			}
		}
	}

	// Internal ids from paths: getInternalId with a fake program around the real path functions.
	const internals = [];
	const internal = (c) => {
		fakePackages = () => ({ directory: c.packageDir, result: { name: c.packageName } });
		const state = {
			getSourceFile: () => ({ fileName: c.filePath }),
			rootDirs: c.rootDirs,
			currentDirectory: c.currentDirectory,
			pathTranslator: { getOutputPath: () => c.outputPath },
		};
		internals.push({ ...c, result: fns.getInternalId(state, { fullName: c.fullName }) });
	};
	const P = "C:\\p";
	internal({ packageName: "game", packageDir: P, filePath: `${P}\\src\\server\\a.ts`, rootDirs: [`${P}\\src`], currentDirectory: P, outputPath: `${P}\\out\\server\\a.lua`, fullName: "A" });
	internal({ packageName: "game", packageDir: P, filePath: `${P}\\src\\index.ts`, rootDirs: [`${P}\\src`], currentDirectory: P, outputPath: `${P}\\out\\init.lua`, fullName: "A.B" });
	internal({ packageName: "game", packageDir: P, filePath: `${P}\\src\\a.ts`, rootDirs: [`${P}\\src`], currentDirectory: P, outputPath: `${P}\\out\\a.luau`, fullName: "A" });
	internal({ packageName: "game", packageDir: P, filePath: `${P}\\src\\a.ts`, rootDirs: [`${P}\\src`], currentDirectory: `${P}\\places\\x`, outputPath: `${P}\\out\\a.lua`, fullName: "A" });
	internal({ packageName: "game", packageDir: P, filePath: `${P}\\shared\\a.ts`, rootDirs: [`${P}\\src`, `${P}\\shared`], currentDirectory: P, outputPath: `${P}\\out\\shared\\a.lua`, fullName: "A" });
	internal({ packageName: "game", packageDir: P, filePath: `c:\\P\\SRC\\a.ts`, rootDirs: [`${P}\\src`], currentDirectory: P, outputPath: `${P}\\out\\a.lua`, fullName: "A" });
	internal({ packageName: "game", packageDir: P, filePath: `${P}\\src\\a.ts`, rootDirs: [`${P}\\src`], currentDirectory: P, outputPath: `D:\\out\\a.lua`, fullName: "A" });
	const lib = `${P}\\node_modules\\@s\\lib`;
	for (const file of ["out\\x.d.ts", "out\\index.d.ts", "out\\x.ts", "out\\sub\\index.ts", "out\\myindex.d.ts", "out\\x.mts", "out\\foots", "out\\ü.d.ts", "src\\x.tsx", "out\\x.d.d.ts"]) {
		internal({ packageName: "@s/lib", packageDir: lib, filePath: `${lib}\\${file}`, rootDirs: [`${P}\\src`], currentDirectory: P, outputPath: "", fullName: "X" });
	}
	internal({ packageName: "@s/lib", packageDir: `C:\\real\\lib`, filePath: `C:\\real\\lib\\out\\x.d.ts`, rootDirs: [`${P}\\src`], currentDirectory: P, outputPath: "", fullName: "X" });
	internal({ packageName: "@s/lib", packageDir: `C:\\real\\lib`, filePath: `C:\\elsewhere\\x.d.ts`, rootDirs: [`${P}\\src`], currentDirectory: P, outputPath: "", fullName: "X" });
	internal({ packageName: "game", packageDir: P, filePath: `${P}\\srcx\\a.ts`, rootDirs: [`${P}\\src`], currentDirectory: P, outputPath: "", fullName: "A" });
	internal({ packageName: "game", packageDir: P, filePath: `${P}\\..src\\a.ts`, rootDirs: [P], currentDirectory: P, outputPath: `${P}\\..src\\a.lua`, fullName: "A" });

	save("ids.json", { formats, declarations, scenarios, internals });
}

// ---------------------------------------------------------------- glob
{
	const root = path.join(TMP, "glob");
	const tree = [
		"src/glob/a.ts",
		"src/glob/b.ts",
		"src/glob/c.d.ts",
		"src/glob/readme.md",
		"src/glob/nested/deep.ts",
		"src/glob/nested/more/deeper.ts",
		"src/glob/.hidden.ts",
		"src/glob/.dotdir/x.ts",
		"src/glob/{a}.ts",
		"src/glob/a b.ts",
		"src/glob/x1.ts",
		"src/glob/x2.ts",
		"src/glob/x10.ts",
		"src/Mixed/File.TS",
		"src/Mixed/lower.ts",
		"src/server/Tests/one.ts",
		"src/server/Tests/sub/two.ts",
		"src/client/Tests/three.ts",
		"src/client/NotTests/four.ts",
		"src/server/Discovery/hidden.ts",
		"src/server/Discovery/hidA.ts",
		"src/server/Discovery/shown.ts",
		"src/server/Discovery/nested/deep.ts",
		"src/ünï/ä.ts",
		"src/ünï/😀.ts",
		"src/.config/c.ts",
		".root/r.ts",
		"top.ts",
		"src/x.ts",
		"src/typesOnly/t.d.ts",
	];
	const emptyDirs = ["src/empty", "src/glob/emptyDir", "src/server/Tests/emptyTests"];
	fs.rmSync(root, { recursive: true, force: true });
	for (const file of tree) {
		fs.mkdirSync(path.dirname(path.join(root, file)), { recursive: true });
		fs.writeFileSync(path.join(root, file), "");
	}
	for (const dir of emptyDirs) fs.mkdirSync(path.join(root, dir), { recursive: true });
	const patterns = [
		"src/glob/**/*.ts",
		"src/missing/**/*.ts",
		"src/*/Tests/**",
		"src/server/Discovery/hid*.ts",
		"src/server/Discovery/**/*.ts",
		"src/server/Discovery/*.ts",
		"src/server/NoSuchTests/**",
		"src/glob/*.ts",
		"src/glob/**",
		"src/glob/**/",
		"src/glob/",
		"src/glob",
		"src/**",
		"**",
		"**/*.ts",
		"*",
		"*.ts",
		"src/*",
		"src/**/*.d.ts",
		"src/**/.*",
		"src/glob/.*",
		"src/glob/.hidden.ts",
		"src/glob/.dotdir/*",
		"src/**/.dotdir/**",
		".root/**",
		"**/r.ts",
		"src/{glob,server}/**/*.ts",
		"src/glob/{a,b}.ts",
		"src/glob/{a,b,zz}.ts",
		"src/glob/x{1..3}.ts",
		"src/glob/x{1..10}.ts",
		"src/glob/{a}.ts",
		"src/glob/\\{a\\}.ts",
		"src/glob/[ab].ts",
		"src/glob/[!a].ts",
		"src/glob/[^a].ts",
		"src/glob/[a-c].ts",
		"src/glob/?.ts",
		"src/glob/??.ts",
		"src/glob/a b.ts",
		"src/glob/a*",
		"src/glob/*b*",
		"src/GLOB/**/*.TS",
		"src/mixed/*",
		"src/Mixed/file.ts",
		"SRC/MIXED/FILE.TS",
		"src/**/nested",
		"src/**/nested/**",
		"src/ünï/*.ts",
		"src/ÜNÏ/*.ts",
		"src/ünï/😀.ts",
		"./src/glob/*.ts",
		"src/../src/glob/*.ts",
		"src/glob/./a.ts",
		"/src/glob/*.ts",
		"src/**/**/deep.ts",
		"src/*/*/*.ts",
		"src/**/empty*",
		"src/empty/**",
		"src/glob/emptyDir/**",
		"top.ts",
		"nope.ts",
		"src/x.ts/**",
		"src/glob/a.ts/",
		"src//glob//a.ts",
		"src/typesOnly/**/*.ts",
		"src/*/Tests/*",
		"src/{server,client}/*Tests/**/*.ts",
		"src/glob/x[0-9].ts",
		"src/glob/x*[0-9].ts",
		"",
		"SRC/Mixed/File.TS",
		"SRC/MIXED",
		"SRC",
		"SRC/MIXED/",
		"src/MIXED/*",
		"SRC/**/FILE.TS",
		"TOP.TS",
		"src/GLOB/A.TS",
		"src/glob/NESTED/deep.ts",
		"SRC/GLOB/NESTED",
		"src/glob/nest*/DEEP.TS",
		"src/glob/*/DEEP.TS",
		"src/[g]LOB/A.TS",
		"src/GLOB/a.ts/../A.TS",
		"src/./GLOB/A.TS",
		"./TOP.TS",
		"src/glob/../GLOB/A.TS",
		"src/nope/../glob/a.ts",
		"src/glob/nested/..",
		"src/glob/..",
		"src/*/..",
		"src/glob/a.ts/.",
		"src/**/../a.ts",
		"src/*/NESTED/DEEP.TS",
		"src/GLOB/**/DEEP.TS",
		"src/glob/@(a|b).ts",
		"src/glob/+(x|1|0).ts",
		"src/glob/x*(1|0).ts",
		"src/glob/?(x)1.ts",
		"src/glob/!(a).ts",
		"src/glob/[[:alpha:]].ts",
		"src/glob/[[:digit:]x]*.ts",
		"src/glob/[A-C].TS",
		"#comment",
		"!src/glob/a.ts",
		".",
		"./",
		"./*",
		"src/**/*.{ts,md}",
		"src/{,glob/}a.ts",
		"src/glob/{a,{b,c}}.ts",
		"src/glob/x{2..1}.ts",
		"src/glob/x{01..2}.ts",
		"src/glob/{a..c}.ts",
	];
	const rows = patterns.map((pattern) => {
		let results;
		const error = message(() => {
			results = glob
				.sync(pattern, { root, cwd: root, nocase: true })
				.map((p) => p.split(root).join("<ROOT>").replace(/\\/g, "/"))
				.sort();
		});
		return error !== undefined ? { pattern, error } : { pattern, results };
	});
	save("glob.json", { tree, emptyDirs, rows });
}

// ---------------------------------------------------------------- brace expansion (minimatch's brace-expansion)
{
	const globReq = createRequire(req.resolve("glob"));
	const mmReq = createRequire(globReq.resolve("minimatch"));
	const expand = mmReq("brace-expansion");
	const inputs = [
		"a{b,c}d",
		"x{1..3}",
		"{a}",
		"a{01..3}",
		"{a,{b,c}}",
		"a{,b}",
		"{c..a}",
		"{a..e..2}",
		"{A..z..10}",
		"{z..a..-3}",
		"{-05..5..3}",
		"a{b,c}{d,e}",
		"{,}",
		"{}a",
		"a{}b",
		"{a,b}{c,d}{1..2}",
		"${a,b}",
		"a{b,{c,d}e}f",
		"x{9223372036854775806..9223372036854775807}",
		"x{1..3..-9223372036854775808}",
		"x{-9223372036854775808..-9223372036854775806}",
		"x{99999999999999999999..100000000000000000002}",
		"x{9007199254740990..9007199254740995}",
		"x{1..9007199254740993..9007199254740992}",
		"x{1..100000000000}",
		"x{0001..3}",
		"x{-1..-3}",
		"x{10..1..4}",
		"x{1..10..0}",
	];
	const rows = inputs.map((input) => {
		const out = expand(input);
		return { input, count: out.length, head: out.slice(0, 20), tail: out.slice(-5) };
	});
	save("braces.json", { rows });
}

// ---------------------------------------------------------------- glob over links (junctions and symbolic links)
// The validator's tree (fw0/validate, validator-gen.cjs): a folder linked into src has to load, so glob 9's rules for
// `**` and links are tested on a tree that has them. The Rust test builds the same tree with the same links.
{
	const root = path.join(TMP, "globlinks");
	fs.rmSync(root, { recursive: true, force: true });
	const files = [
		"src/Server/Services/DataService.ts",
		"src/Server/Services/nested/Inner.ts",
		"src/Server/Components/PlayerComponent.ts",
		"src/Server/Components/sub/Deep.ts",
		"src/Server/components.ts",
		"src/Server/main.server.ts",
		"src/Server/.hidden/Secret.ts",
		"src/Shared/Components/SharedComp.ts",
		"src/Shared/components2/x.ts",
		"src/Shared/Network.ts",
		"src/Client/Controllers/InputController.ts",
		"src/Client/Components/UI/Button.tsx",
		"src/Client/COMPONENTS.d.ts",
		"src/ReplicatedStorage/Components/A.d.ts",
		"src/Σigma/ς.ts",
		"src/Straße/x.ts",
		"src/ÄÖÜ/äöü.ts",
		"external/Ext.ts",
		"external/sub/DeepExt.ts",
		"external/Components/LinkedComp.ts",
		"externalTwo/Two.ts",
		"externalTwo/inner/TwoDeep.ts",
	];
	for (const f of files) {
		fs.mkdirSync(path.dirname(path.join(root, f)), { recursive: true });
		fs.writeFileSync(path.join(root, f), "");
	}
	const links = [
		{ target: "external", at: "src/Linked", type: "junction" },
		{ target: "externalTwo", at: "external/LinkToTwo", type: "junction" },
		{ target: "external", at: "src/Server/Services/ServicesLink", type: "junction" },
		{ target: "external/Ext.ts", at: "src/Shared/FileLink.ts", type: "file" },
		{ target: "external", at: "src/Shared/DirSymlink", type: "dir" },
	];
	for (const l of links) fs.symlinkSync(path.join(root, l.target), path.join(root, l.at), l.type);
	const patterns = [
		"src/server/**/*",
		"src/server/**/*.ts",
		"src/**/components",
		"src/**/Components",
		"src/**/components/",
		"src/**/components/**",
		"src/**/components/*.ts",
		"src/**/components/**/*.ts",
		"src/*/components/*",
		"src/**/*",
		"src/**",
		"src/**/*.tsx",
		"src/**/*.d.ts",
		"SRC/SERVER/**/*",
		"SRC/SERVER/SERVICES/*",
		"src/server/services/DATASERVICE.TS",
		"src/server/services/dataservice.ts",
		"src/SERVER/Services/Nested/inner.TS",
		"src/server/components",
		"src/server/COMPONENTS.TS",
		"src/client/components*",
		"src/client/components.d.ts",
		"src/**/component*",
		"src/linked/**",
		"src/linked/*",
		"src/linked/**/*.ts",
		"src/Linked/sub/*",
		"src/**/ext.ts",
		"src/**/deepext.ts",
		"src/**/two.ts",
		"src/**/twodeep.ts",
		"src/**/linkedcomp.ts",
		"src/server/services/serviceslink/**",
		"src/shared/*",
		"src/shared/**",
		"src/shared/filelink.ts",
		"src/shared/dirsymlink/**",
		"./src/server/**/*",
		"src/server/**/*.server.ts",
		"src/σigma/*",
		"src/ΣIGMA/Σ.ts",
		"src/σigma/σ.ts",
		"src/STRASSE/*",
		"src/straße/X.TS",
		"src/äöü/*",
		"src/ÄÖÜ/ÄÖÜ.TS",
		"src/*/",
		"**/components",
		"src/{server,shared}/**/*",
		"src/{Server,client}/components/**/*.{ts,tsx}",
		"src/*/**",
		"src/*/**/*.ts",
		"src/Server/Services/ServicesLink/**/*.ts",
		"src/**/LinkToTwo/*",
		"**/*.ts",
		"src/**/",
		"src/shared/*/",
		"src/linked/linktotwo/**",
		"src/**/sub/*.ts",
	];
	const rows = patterns.map((pattern) => {
		let results;
		const error = message(() => {
			results = glob
				.sync(pattern, { root, cwd: root, nocase: true })
				.map((p) => p.split(root).join("<ROOT>").replace(/\\/g, "/"))
				.sort();
		});
		return error !== undefined ? { pattern, error } : { pattern, results };
	});
	save("glob-links.json", { files, links, rows });
}

fs.rmSync(TMP, { recursive: true, force: true });
console.log(`wrote ${written.length} tables:\n  ${written.join("\n  ")}`);
