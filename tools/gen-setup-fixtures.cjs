// Generates expected.json in every fixture project under crates/rbxts_bench/tests/fixtures/setup: roblox-ts's own
// project setup (dump-project-services.cjs) on that project, with the fixtures directory written as {root} (and
// {root_fwd} with forward slashes) so that the expectations do not depend on where the repository is.
//
//   node tools/gen-setup-fixtures.cjs

"use strict";
const fs = require("fs");
const path = require("path");
const { dump } = require("./dump-project-services.cjs");

const fixtures = path.resolve(__dirname, "..", "crates", "rbxts_bench", "tests", "fixtures", "setup");
const fwd = fixtures.replace(/\\/g, "/");

function placeholders(value) {
	if (typeof value === "string") return value.split(fixtures).join("{root}").split(fwd).join("{root_fwd}");
	if (Array.isArray(value)) return value.map(placeholders);
	if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [placeholders(k), placeholders(v)]));
	return value;
}

for (const name of fs.readdirSync(fixtures)) {
	const dir = path.join(fixtures, name);
	if (!fs.statSync(dir).isDirectory()) continue;
	const result = dump(path.join(dir, "tsconfig.json"), {}, 0);
	delete result.rbxtsDir;
	result.cwd = fixtures;
	fs.writeFileSync(path.join(dir, "expected.json"), JSON.stringify(placeholders(result), null, 1) + "\n");
	const files = result.projects.reduce((n, p) => n + p.files.length, 0);
	console.log(`${name}: ${result.projects.length} project(s), ${files} files${result.error ? ", error: " + result.error.join(" | ").slice(0, 200) : ""}`);
}
