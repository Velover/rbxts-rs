// Generates crates/rbxts_project/tests/fixtures/nodepath_cases.json: inputs and Node's answers for the `path`
// functions `rbxts_project::nodepath` ports, for both flavours. Run with any Node; the output is committed.
//
//   node tools/gen-nodepath-cases.cjs

const path = require("path");
const fs = require("fs");

const CWD_WIN = "E:\\Work\\Proj";
const CWD_POSIX = "/work/proj";

const winFragments = [
	"",
	".",
	"..",
	"E:",
	"e:",
	"E:\\",
	"E:/",
	"D:\\other",
	"\\",
	"/",
	"\\\\server\\share",
	"//server/share/dir",
	"a",
	"a/b",
	"a\\b\\",
	"./a",
	"../a",
	"..\\..\\b",
	"E:\\Work\\Proj\\src",
	"E:/Work/Proj/src/",
	"e:/work/proj/SRC/x.ts",
	"E:\\Work\\Proj\\out\\init.luau",
	"E:/Work/Proj/src-extra",
	"E:\\Work\\Proj\\src\\a\\..\\b\\.\\c",
	"x.server.luau",
	".hidden",
	"..dots",
	"a..",
	"file.d.ts",
	"index.",
	"C:relative",
	"E:\\Work\\Proj\\node_modules\\@rbxts\\pkg\\out\\index.d.ts",
	// UNC edge forms: doubled separators inside the root, a server alone, three leading separators, device paths
	"//server//share",
	"//server//share//x",
	"\\\\server\\\\share\\x",
	"///server/share",
	"//server",
	"//server/",
	"\\\\?\\C:\\x",
	"//./pipe/x",
	// characters whose lowercase has another UTF-16 length (`İ`), a final sigma, a surrogate pair
	"E:\\İx\\a",
	"E:\\İx\\b\\cİd",
	"e:\\i̇x\\B",
	"E:\\Work\\Proj\\SRC\\ΑΣ",
	"E:\\Work\\Proj\\src\\😀\\x",
];
const posixFragments = [
	"",
	".",
	"..",
	"/",
	"//",
	"a",
	"a/b",
	"a/b/",
	"./a",
	"../a",
	"../../b",
	"/work/proj/src",
	"/work/proj/src/",
	"/Work/Proj/SRC/x.ts",
	"/work/proj/out/init.luau",
	"/work/proj/src-extra",
	"/work/proj/src/a/../b/./c",
	"x.server.luau",
	".hidden",
	"..dots",
	"a..",
	"file.d.ts",
	"index.",
];

function cases(p, fragments, cwd) {
	const out = { cwd, normalize: [], join: [], resolve: [], relative: [], dirname: [], basename: [], basenameExt: [], extname: [], isAbsolute: [] };
	const resolve = (...a) => p.resolve(cwd, ...a);
	for (const a of fragments) {
		out.normalize.push([a, p.normalize(a)]);
		out.dirname.push([a, p.dirname(a)]);
		out.basename.push([a, p.basename(a)]);
		out.extname.push([a, p.extname(a)]);
		out.isAbsolute.push([a, p.isAbsolute(a)]);
		for (const ext of [".luau", ".ts", ".d.ts", ""]) {
			out.basenameExt.push([a, ext, p.basename(a, ext)]);
		}
		out.resolve.push([[a], resolve(a)]);
		for (const b of fragments) {
			out.join.push([[a, b], p.join(a, b)]);
			out.resolve.push([[a, b], resolve(a, b)]);
			out.relative.push([a, b, p.relative(resolve(a), resolve(b))]);
		}
	}
	out.join.push([[], p.join()]);
	return out;
}

const result = {
	win32: cases(path.win32, winFragments, CWD_WIN),
	posix: cases(path.posix, posixFragments, CWD_POSIX),
};
const target = path.join(__dirname, "..", "crates", "rbxts_project", "tests", "fixtures", "nodepath_cases.json");
fs.writeFileSync(target, JSON.stringify(result));
console.log(`wrote ${target}`);
