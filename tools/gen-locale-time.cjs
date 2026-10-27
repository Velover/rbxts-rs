// Generates crates/rbxts_cli/src/locale_time.rs's table: how Node's `new Date().toLocaleTimeString()` (the time
// TypeScript's watch status prints) formats a time in each locale Windows can have as its user locale.
//
//   node tools/gen-locale-time.cjs            (on Windows, with the Node whose output rbxtsc should match)
//
// The locale names are the cultures Windows knows (PowerShell 5.1's .NET Framework, which asks Windows' NLS: the
// names `GetUserDefaultLocaleName` gives). For each, Node's ICU gives the format (`formatToParts` of every hour of a
// day): the parts in order, the hour cycle and width, the minute and second widths, the day period of each hour (two
// strings, AM and PM, or one per hour for a locale whose day periods are finer), and the digits (the numbering
// system's zero). Each record is checked by rendering it as Rust will, for every hour and several minutes and
// seconds, against Node's own output. Writes the table and `crates/rbxts_cli/tests/fixtures/locale_time_samples.txt`
// (one line per locale: name, then the outputs at three times) for the Rust test.
"use strict";
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const root = path.join(__dirname, "..");
const names = execFileSync(
	"powershell.exe",
	["-NoProfile", "-Command", "[Globalization.CultureInfo]::GetCultures([Globalization.CultureTypes]::AllCultures) | % { $_.Name }"],
	{ encoding: "utf8" },
)
	.split(/\r?\n/)
	.map(s => s.trim())
	.filter(s => s.length > 0);

// V8 prints ICU's narrow no-break space (U+202F, CLDR 42+ before `AM`/`PM`) as a plain space in `format` and
// `toLocaleTimeString` (a web-compatibility measure), but not in `formatToParts`
const nnbsp = text => text.replace(/ /g, " ");
const OPTIONS = { hour: "numeric", minute: "numeric", second: "numeric" };
const at = (h, m, s) => new Date(2026, 0, 1, h, m, s);

function render(rec, h, m, s) {
	const digits = (n, width) => {
		let text = String(n);
		if (width === 2 && text.length < 2) text = "0" + text;
		return [...text].map(c => String.fromCodePoint(rec.zero + (c.charCodeAt(0) - 48))).join("");
	};
	let hour;
	switch (rec.cycle) {
		case 11: hour = h % 12; break;
		case 12: hour = h % 12 === 0 ? 12 : h % 12; break;
		case 23: hour = h; break;
		case 24: hour = h === 0 ? 24 : h; break;
	}
	let out = "";
	for (const part of rec.parts) {
		if (part.lit !== undefined) out += part.lit;
		else if (part.kind === "hour") out += digits(hour, rec.hourWidth);
		else if (part.kind === "minute") out += digits(m, rec.minuteWidth);
		else if (part.kind === "second") out += digits(s, rec.secondWidth);
		else out += rec.periods.length === 2 ? rec.periods[h < 12 ? 0 : 1] : rec.periods[h];
	}
	return out;
}

function recordOf(name) {
	let dtf;
	try {
		dtf = new Intl.DateTimeFormat(name, OPTIONS);
	} catch {
		return undefined;
	}
	const resolved = dtf.resolvedOptions();
	const shape = p => p.map(x => (x.type === "literal" ? "L" + x.value : x.type)).join("|");
	const parts13 = dtf.formatToParts(at(13, 1, 2));
	const rec = { parts: [], cycle: 0, hourWidth: 1, minuteWidth: 2, secondWidth: 2, periods: [], zero: 48 };
	for (const p of parts13) {
		if (p.type === "literal") rec.parts.push({ lit: nnbsp(p.value) });
		else if (["hour", "minute", "second", "dayPeriod"].includes(p.type)) rec.parts.push({ kind: p.type });
		else throw new Error(`${name}: unexpected part ${p.type}`);
	}
	const minute = parts13.find(p => p.type === "minute").value;
	rec.zero = minute.codePointAt(0);
	const value = text => Number([...text].map(c => String(c.codePointAt(0) - rec.zero)).join(""));
	rec.minuteWidth = [...minute].length;
	rec.secondWidth = [...parts13.find(p => p.type === "second").value].length;
	const hours = [];
	const periods = [];
	for (let h = 0; h < 24; h++) {
		const parts = dtf.formatToParts(at(h, 1, 2));
		if (shape(parts).replace(/\|?dayPeriod\|?/, "") !== shape(parts13).replace(/\|?dayPeriod\|?/, "")) {
			// a locale whose literals change with the hour: not modelled
			throw new Error(`${name}: the parts change with the hour (${shape(parts)} vs ${shape(parts13)})`);
		}
		const hour = parts.find(p => p.type === "hour").value;
		hours.push(value(hour));
		if ([...hour].length === 2 && value(hour) < 10) rec.hourWidth = 2;
		const period = parts.find(p => p.type === "dayPeriod");
		periods.push(period ? nnbsp(period.value) : "");
	}
	const cycles = { 11: h => h % 12, 12: h => (h % 12 === 0 ? 12 : h % 12), 23: h => h, 24: h => (h === 0 ? 24 : h) };
	rec.cycle = Number(Object.keys(cycles).find(c => hours.every((v, h) => v === cycles[c](h))));
	if (!rec.cycle) throw new Error(`${name}: no hour cycle fits ${hours}`);
	const am = periods.slice(0, 12);
	const pm = periods.slice(12);
	rec.periods = am.every(p => p === am[0]) && pm.every(p => p === pm[0]) ? [am[0], pm[0]] : periods;
	if (!rec.parts.some(p => p.kind === "dayPeriod")) rec.periods = [];
	// checked against Node, as Rust renders it
	for (let h = 0; h < 24; h++) {
		for (const [m, s] of [[0, 0], [5, 9], [59, 59], [10, 30]]) {
			const want = dtf.format(at(h, m, s));
			const got = render(rec, h, m, s);
			if (want !== got) throw new Error(`${name} ${h}:${m}:${s}: Node ${JSON.stringify(want)} ${[...want].map(c => c.codePointAt(0).toString(16))}, table ${JSON.stringify(got)} ${[...got].map(c => c.codePointAt(0).toString(16))}`);
			if (want !== at(h, m, s).toLocaleTimeString(name)) throw new Error(`${name}: toLocaleTimeString differs`);
		}
	}
	return rec;
}

const records = [];
const keys = new Map();
const table = [];
const samples = [];
let skipped = [];
for (const name of names) {
	const rec = recordOf(name);
	if (!rec) {
		skipped.push(name);
		continue;
	}
	const key = JSON.stringify(rec);
	if (!keys.has(key)) {
		keys.set(key, records.length);
		records.push(rec);
	}
	table.push([name, keys.get(key)]);
	samples.push([name, at(23, 4, 9), at(0, 5, 6), at(12, 0, 0)].map(x => (typeof x === "string" ? x : x.toLocaleTimeString(name))).join("\t"));
}
if (records.length > 255) throw new Error(`${records.length} records: more than a byte indexes`);
table.sort((a, b) => (a[0].toLowerCase() < b[0].toLowerCase() ? -1 : a[0].toLowerCase() > b[0].toLowerCase() ? 1 : 0));

// a Rust string literal, non-ASCII escaped by code point
const rust = s =>
	`"${[...s].map(c => (c.codePointAt(0) >= 0x7f ? `\\u{${c.codePointAt(0).toString(16)}}` : JSON.stringify(c).slice(1, -1))).join("")}"`;
let out = `// Generated by tools/gen-locale-time.cjs from Node ${process.version} (ICU ${process.versions.icu}, CLDR ${process.versions.cldr}): Node's
// \`toLocaleTimeString()\` for each locale Windows knows. Do not edit; run the script again after a Node update.

/// One time format: Node's \`toLocaleTimeString()\` parts.
pub(crate) struct Format {
    pub parts: &'static [Part],
    /// 11 (0-11), 12 (1-12), 23 (0-23) or 24 (1-24).
    pub cycle: u8,
    pub hour_width: u8,
    pub minute_width: u8,
    pub second_width: u8,
    /// AM and PM, or one per hour (a locale with finer day periods), or none.
    pub periods: &'static [&'static str],
    /// The numbering system's zero.
    pub zero: char,
}

/// A part of a time format.
pub(crate) enum Part {
    Literal(&'static str),
    Hour,
    Minute,
    Second,
    Period,
}

pub(crate) static FORMATS: [Format; ${records.length}] = [
`;
for (const rec of records) {
	const parts = rec.parts
		.map(p => (p.lit !== undefined ? `Part::Literal(${rust(p.lit)})` : { hour: "Part::Hour", minute: "Part::Minute", second: "Part::Second", dayPeriod: "Part::Period" }[p.kind]))
		.join(", ");
	out += `    Format {\n        parts: &[${parts}],\n        cycle: ${rec.cycle},\n        hour_width: ${rec.hourWidth},\n        minute_width: ${rec.minuteWidth},\n        second_width: ${rec.secondWidth},\n        periods: &[${rec.periods.map(rust).join(", ")}],\n        zero: '\\u{${rec.zero.toString(16)}}',\n    },\n`;
}
out += `];

/// The locale names, lowercase, one per line, sorted; \`LOCALE_FORMATS\` holds each one's index in \`FORMATS\`.
pub(crate) static LOCALE_NAMES: &str = ${rust(table.map(t => t[0].toLowerCase()).join("\n"))};

pub(crate) static LOCALE_FORMATS: [u8; ${table.length}] = [${table.map(t => t[1]).join(", ")}];
`;
fs.writeFileSync(path.join(root, "crates/rbxts_cli/src/locale_time_table.rs"), out);
fs.writeFileSync(path.join(root, "crates/rbxts_cli/tests/fixtures/locale_time_samples.txt"), samples.join("\n") + "\n");
console.log(`${table.length} locales, ${records.length} formats; skipped ${skipped.length}: ${skipped.join(" ")}`);
