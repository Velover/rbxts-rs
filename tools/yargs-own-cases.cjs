// Writes crates/rbxts_cli/tests/fixtures/yargs_own_cases.jsonl: argument vectors with rbxts-rs's own build options
// (`--timings`, `--events`, `--checkers`) and what roblox-ts's CLI setup (its yargs, `src/CLI/cli.ts` and
// `commands/build.ts`) makes of each once those options are declared as rbxts-rs declares them (`timings` boolean,
// `events` string, `checkers` untyped). The stock vectors (`yargs_cases.jsonl`) cannot hold them: stock's strict mode
// refuses unknown options.
//
//   node tools/yargs-own-cases.cjs   (RBXTS_DIR: a roblox-ts checkout with `out/` built)

"use strict";
const fs = require("fs");
const path = require("path");

const RT = process.env.RBXTS_DIR || "E:/Projects/TS/Flamework/roblox-ts-test/roblox-ts";
const yargs = require("module").createRequire(`${RT}/out/CLI/cli.js`)("yargs/yargs");
const build = require(`${RT}/out/CLI/commands/build.js`);
const KEYS = [
	"project", "watch", "usePolling", "verbose", "noInclude", "logTruthyChanges", "writeOnlyChanged",
	"writeTransformedFiles", "optimizedLoops", "type", "includePath", "rojo", "allowCommentDirectives", "luau",
	"timings", "events", "checkers",
];
const command = {
	...build,
	builder: parser =>
		build.builder(parser).option("timings", { boolean: true }).option("events", { string: true }).option("checkers", {}),
};

const vectors = [
	["build", "--timings"],
	["--timings"],
	["build", "--timings=false"],
	["build", "--timings", "false"],
	["build", "--no-timings"],
	["build", "--timings", "--timings"],
	["build", "--timings=yes"],
	["build", "--events", "json"],
	["build", "--events=json"],
	["build", "-w", "--events", "json"],
	["build", "--events"],
	["build", "--events", "json", "--events", "text"],
	["build", "--events", "--timings"],
	["build", "--no-events"],
	["build", "--events", "1"],
	["build", "--events", "-w"],
	["-w", "--events", "json", "--timings", "-p", "."],
	["build", "--checkers", "4"],
	["build", "--checkers=4"],
	["build", "--checkers", "abc"],
	["build", "--checkers"],
	["build", "--checkers", "0x10"],
	["build", "--checkers", "1e1"],
	["build", "--checkers", "-1"],
	["build", "--checkers", "2", "--checkers", "3"],
	["build", "--no-checkers"],
	["build", "--checkers", "4", "--timings", "--events=json", "--verbose"],
	["build", "--timing"],
	["build", "--event", "json"],
];

const print = console.log;
const lines = [];
(async () => {
	for (const args of vectors) {
		let result;
		let logged = "";
		console.log = console.error = (...a) => {
			logged += a.join(" ");
		};
		await yargs(args)
			.usage("roblox-ts - A TypeScript-to-Luau Compiler for Roblox")
			.help("help").alias("h", "help").describe("help", "show help information")
			.version("3.0.0").alias("v", "version").describe("version", "show version information")
			.command({
				...command,
				handler: argv => {
					result = result ?? { ok: Object.fromEntries(KEYS.filter(k => argv[k] !== undefined).map(k => [k, argv[k]])) };
				},
			})
			.recommendCommands()
			.strict()
			.exitProcess(false)
			.wrap(80)
			.fail((msg, err) => {
				if (!result || !result.fail) result = { fail: msg ?? String(err) };
			})
			.parseAsync()
			.catch(e => {
				result = { fail: String(e) };
			});
		console.log = print;
		if (!result) result = { other: logged.trim() === "3.0.0" ? "version" : "help" };
		lines.push(JSON.stringify({ args, ...result }));
	}
	const target = path.join(__dirname, "..", "crates", "rbxts_cli", "tests", "fixtures", "yargs_own_cases.jsonl");
	fs.writeFileSync(target, lines.join("\n") + "\n");
	print(`wrote ${lines.length} vectors to ${target}`);
})();
