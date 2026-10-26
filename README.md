# rbxts-rs

roblox-ts rewritten in Rust on [tsrs](https://github.com/oxc-project/tsrs) (the Rust port of TypeScript 7), with
Flamework as an in-process plugin. Private and local: never pushed or published.

- Design: `E:\Projects\TS\Flamework\scratchpad\batch\rbxts-rs\DESIGN.md`.
- Measurements: `bench/RESULTS.md`.
- tsrs: a fork branch `rbxts-fork` in `E:\Projects\TS\Flamework\worktrees\tsrs-rbxts-fork`, used through path
  dependencies (see `Cargo.toml`).

## Build and measure

```sh
cargo build --profile measure
cd <project with tsconfig.json>
<rbxts-rs>/target/measure/rbxts-bench.exe load [--checkers N] [--check project|all|none] [--lazy-dts] [--json]
```

`dev` builds are optimised too (a debug checker is unusably slow). Use `measure` for any number that goes into
`bench/RESULTS.md`, and interleave runs with the reference because machine load moves wall times.

## Luau render parity (`rbxts_luau`)

`crates/rbxts_luau` is `@roblox-ts/luau-ast` 2.1.2 in Rust. Its renderer is checked byte for byte against roblox-ts's
on real trees: `tools/dump-luau-ast.cjs` runs roblox-ts (TS5) in-process and writes every Luau tree it renders, with
the text, as JSON lines; `rbxts-bench luau-parity` renders them again and compares.

```sh
# from the project directory (transformer plugins find the tsconfig through it)
node <rbxts-rs>/tools/dump-luau-ast.cjs <roblox-ts checkout> tsconfig.json <dump.jsonl>
<rbxts-rs>/target/measure/rbxts-bench.exe luau-parity <dump.jsonl>... [--show N] [--repeat N] [--threads N]
```

Dumps of the real project stay outside the repo (scratchpad `rbxts-rs/luau-dumps`). The committed fixture
`crates/rbxts_bench/fixtures/luau-edge-cases.jsonl` comes from `tools/luau-edge-cases.cjs` (hand-written trees,
rendered by roblox-ts's luau-ast) and is checked by `cargo test -p rbxts_bench`.

## Project setup parity (`rbxts_project`)

`crates/rbxts_project` also holds the setup roblox-ts does before transforming: `@roblox-ts/rojo-resolver` 1.3.0
(`rojo`), `@roblox-ts/path-translator` 1.1.1 (`path_translator`), Node's `path` semantics they rely on (`nodepath`),
and the single-project parts of roblox-ts's `src/Project` (`setup`: `rbxts` options, project data, compiler-option
checks, the path translator, the package resolvers and mapping, `compileFiles`'s Rojo checks), shared with plugins
as `ProjectServices` (with package.json lookups and `Project::resolve_module`). `tools/dump-project-services.cjs`
records roblox-ts's own values for a project; `rbxts-bench project-parity` computes them with the port and compares.

```sh
# RBXTS_DIR: a roblox-ts checkout with out/ built (default E:/Projects/TS/Flamework/roblox-ts-test/roblox-ts)
node <rbxts-rs>/tools/dump-project-services.cjs <project dir or tsconfig.json> <dump.json> [--timing-runs N]
<rbxts-rs>/target/measure/rbxts-bench.exe project-parity <dump.json> [--runs N] [--max-diffs N]
```

The roblox-ts checkout the dumps were made with has a patched `@roblox-ts/rojo-resolver`
(`out/RojoResolver.js` calls `realpathSync.native` where the shipped file, kept as `RojoResolver.js.orig`, calls
`realpathSync`); parity holds against either. Dumps of the real project stay outside the repo (scratchpad
`rbxts-rs/project-dumps`). `cargo test -p rbxts_project
-p rbxts_bench` runs the same comparisons on committed fixtures: Node's answers for the path functions
(`tools/gen-nodepath-cases.cjs`), the JavaScript resolver's on hand-made Rojo trees (`tools/gen-rojo-cases.cjs`) and
roblox-ts's setup on hand-made projects (`tools/gen-setup-fixtures.cjs`); rerun those scripts when the fixtures
change.

## Building a project (`rbxts_transform`)

`crates/rbxts_transform` is roblox-ts master's `src/TSTransformer` ported module by module (DESIGN §9b), with the
build pipeline (`compile_project`: load → setup → check → transform → render → write changed outputs). Node kinds
not ported yet are errors reading `not yet supported in rbxts-rs: <kind>`; the build reports every file and writes
nothing unless asked to (`--write-partial`).

```sh
cd <project>
<rbxts-rs>/target/measure/rbxts-bench.exe build [-p tsconfig.json] [--optimizedLoops] [--allowCommentDirectives] \
    [--out-dir DIR] [--write-partial] [--no-write] [--parity REF_DIR] [--report FILE] [--show N] [--json]
```

`--out-dir` writes outside the project's `outDir` (Rojo paths still resolve against `outDir`). `--parity` compares
each output with a reference build's file (identical / differing / unsupported / failed / missing), ranks the
unsupported kinds and the work areas by the files they block, and with `--report` writes the per-file list.
References: roblox-ts's tests project (`tsconfig.bench.json`, against the tsrs-backed build in scratchpad
`tsrs-roblox-ts/exp/out-C-tsrs/out`) and the real project without Flamework (scratchpad `rbxts-rs/refs`); both were
built with `--optimizedLoops --allowCommentDirectives`.

Hand-written transform fixtures live in `crates/rbxts_transform/tests/fixtures/<area>`: a roblox-ts project without
`node_modules` (`src`, `tsconfig.json`, `default.project.json`) and `expected/`, stock roblox-ts's output for it
(`node roblox-ts-test/probe/rbxts-driver.cjs <roblox-ts checkout> tsconfig.json --backend ts5`, from the fixture with a
`node_modules` linked in). `cargo test -p rbxts_transform --test fixtures` builds each one in a temporary copy, linking
in `RBXTS_RS_FIXTURE_NODE_MODULES` (default: the roblox-ts tests project's `node_modules`, found as
`roblox-ts/tests` or `roblox-ts-test/roblox-ts/tests` beside the checkout or a directory above it), and compares byte
for byte; any diagnostic fails the fixture. A fixture with a `links.json` is built in each linked layout it describes
(junctions made at test time). Without a `node_modules` the tests are skipped with a note on the terminal, or fail
when `CI` or `RBXTS_RS_FIXTURE_REQUIRE=1` is set; a variable naming a directory without the packages is an error.
The `json` fixture is compared as the written output tree (`expected/` is stock `rbxtsc build`'s `out`).

**JSON modules** (`resolveJsonModule`) are handled as stock roblox-ts handles them: no JSON is transformed. A JSON
file under a root dir is an asset, copied to the output as it is (Rojo makes it a ModuleScript) whether anything
imports it or not, and an import names that output (`TS.import(script, …, "shared", "data")`); a JSON file outside
the root dirs is imported at its source path, which the Rojo project must map. An edit of a JSON module re-copies it
and re-checks and re-transforms its importers (watch and incremental builds; `tests/watch_fixtures/basic`).

**File order.** A build orders its files as roblox-ts transforms a clean build's (`getChangedFilePaths`: each
program file followed by the files that import it, depth first; `build::transform_order`): the transform, the
once-per-compilation diagnostics (reported at the first file in that order), and the file parts plugins' `finish`
gets (`FileIndex`), so TS5 Flamework's id allocation and `flamework.build` table order are reproduced. Watch and
incremental builds use the clean build's order too, where roblox-ts follows its changed files. `rbxtsc` prints the
diagnostics sorted and deduplicated, as roblox-ts does. One deviation: roblox-ts stops transforming at the first
file with an error (its diagnostics service is global), so it reports the transform errors of that file only;
rbxts-rs transforms and reports every file.

**Project references** (`build::references`, `rbxts_project::graph`): a tsconfig with `references` builds as
roblox-ts master's `ProjectGraph` and `ProjectBuild` do. The graph walks the references in build order (a cycle is
error 6202), each project inheriting the root's command-line options, validates them (TS6306 `composite`, TS6310
`noEmit`, Rojo paths for every runtime project, overlapping output roots) and maps each referenced project's root
dirs to its Rojo output path, so an import across the boundary names the referenced project's output
(`util/create_import_expression.rs`). Each project is a `BuildSession` of its own (incremental and watch state
included), built before the projects that reference it; a project whose references failed, directly or through a
solution config, is blocked (not built, nothing written), and the build reports every project's outcome
(`CompileReport.projects`). A project that is not dirty (no change under its root dirs, program files, configs,
reads or plugin inputs, and no dirty reference: `BuildSession::needs_build`) is not built, and its last diagnostics
are reported again, as roblox-ts does in watch mode. A solution root (no files of its own) only copies the runtime
include, and only when every project built: roblox-ts copies it whatever the build does, rbxts-rs writes nothing
for a failing build (DESIGN §14), on purpose. Graph errors (TS6306, TS6310, TS6202) are printed as roblox-ts's
`DiagnosticError` prints them. The root program reads a referenced project's output
declarations where module resolution names its sources; the incremental closure and the transform order follow that
redirect, so an edit in a referenced project reaches its importers. `rbxtsc -w` watches every project of the graph
and ignores every project's output roots. Deviations: `--out-dir` builds the root project only (roblox-ts would point
every project at the one directory); what makes a project dirty is a superset of roblox-ts's (any change under a
`node_modules` or to a Rojo project file).

**Declarations** (`project/emit_declarations.rs`): a `declaration` or `composite` build checks the declaration
diagnostics with the others and emits the `.d.ts` (and `.d.ts.map`) of every transformed file through tsrs's
emitter, then applies roblox-ts's `afterDeclarations` transformers (`/// <reference types="types" />` to
`@rbxts/types`; `transformPaths` with `baseUrl`/`paths`) as text rewrites, moving the map's mappings with them.
`.d.ts` files under the root dirs are declaration assets, reprinted as `transformDeclarationFile` does. Declarations
go through the same all-or-nothing commit as the Luau, and stale ones are removed by the `getProjectOutputs` keep set.
Declaration maps are TypeScript 7's (its mappings differ from TypeScript 5's for some declarations); the declarations
themselves equal stock's (fixture `declarations`, compared as the whole output tree).

**`--writeTransformedFiles`** (`project/write_transformed_files.rs`): stock roblox-ts writes each source as its
TypeScript transformers left it, `<outDir>/<path>.transformed`, and writes nothing when the tsconfig has no
transformer (the clean still removes stale `.transformed` files). rbxts-rs plugins are not TypeScript transformers and
change no TypeScript: it writes the TypeScript printer's output of the source (stock's text for a no-op transformer)
when an active plugin was claimed by a tsconfig `plugins` entry, and nothing otherwise, through the same commit
(stock writes them before the build's errors stop it). Fixtures `write_transformed` (a no-op transformer, compared with
stock) and `write_transformed_without_transformers`.

## Plugins (`rbxts_plugin`, `rbxts_transform::plugins`, `rbxts_cli`)

`crates/rbxts_plugin` is the plugin API (DESIGN §11); `rbxts_transform::plugins` is the compiler side: the backends,
the dispatch tables built from each instance's `Interest`, and the build lifecycle (activation from the tsconfig's
`plugins` entries, `setup`, `finish`, the late-string key check, render after finish, the commit with atomic writes,
`explain_checker_diagnostic` on files that fail checking). The lifecycle across builds (kept `Shared`s, committed
states, cached file parts and late values) is `rbxts_transform::build` (below).

A binary with plugins:

```rust
fn main() -> std::process::ExitCode {
    rbxts_cli::Cli::new().plugin(my_plugin::MyPlugin::new()).run() // rbxtsc's `build` arguments, exit codes 0 / 1
}
```

`rbxtsc` (crate `rbxts_cli`) is that binary without plugins. It is called as stock `rbxtsc` is (no arguments in the
project folder, `-p <dir | tsconfig>`, `--verbose`, `--type`, `-i`, `--noInclude`, `--rojo`, …, parsed by yargs's
rules) and answers as stock does: exit code 0 or 1, diagnostics on stdout in TypeScript's pretty format, roblox-ts's
own errors as `TS roblox-ts` text diagnostics. A successful build copies the runtime include files (`copyInclude`)
from the roblox-ts package the project resolves (the first `node_modules/roblox-ts` from the project folder upwards),
so the include folder equals stock's; without one, from the files embedded from roblox-ts master 68648b0
(`crates/rbxts_transform/include`). The tsconfig must suit TypeScript 7: `module: "preserve"` with
`moduleResolution: "bundler"`, `paths` instead of `baseUrl`, no `downlevelIteration`, and `types` listed (TypeScript 7
includes no type roots by default), e.g. `"types": ["types", "compiler-types"]`.

A tsconfig `plugins` entry (`transform`) that no compiled-in plugin claims is handled as stock roblox-ts 3.0.0 handles
a transformer it can't load: when Node can't resolve it from the project folder, a warning with stock's text
("Transformer `…` was not found!") and the build goes on without it; when Node would load it (stock would run it,
rbxts-rs can't), the build fails ("not compiled into this toolchain"). `rbxts-bench --ignore-unknown-plugins` ignores
both.

`rbxts-bench build --plugin <name>` registers the test
plugins of `crates/rbxts_test_plugins` (the `rbxts_plugin` crate's `stamp`, and `probe`, which reaches every compiler
primitive), with `--timings` for per-plugin hook times and `--claims-all` (or `RBXTS_PLUGIN_CLAIMS=all`) to report
double claims. `cargo test -p rbxts_transform --test plugins` builds `tests/plugin_fixtures/probe` with them against
hand-checked `expected/` Luau, and runs their failure modes.

## Incremental and watch builds (`rbxts_transform::build`, `rbxtsc -w`)

`BuildSession` (DESIGN §14) runs every build: `rbxtsc build` is one session with one build, `rbxtsc -w` one session
rebuilt on file system events. A build is clean (every file), incremental (a new process for an `incremental` or
`composite` project, reading `<base>.rbxts-rs.buildinfo`, which sits beside roblox-ts's build info) or a watch rebuild.
Every build loads the program (over a cache of parsed files) and the project setup again, as roblox-ts does, so module
resolution sees packages installed or upgraded since the last build.

- **What is transformed:** every file after a change of the salt (the compiler binary, options, tsconfig and Rojo
  texts, the plugin list and tsconfig entries) or when a plugin's `setup` runs again or read a changed build input
  (a file, an environment variable, a module resolution, a glob's matches); otherwise roblox-ts's closure
  (changed files plus everything that reaches them through the reversed references), TypeScript's global rule (a
  changed or reached script file, `declare global` or ambient module declaration invalidates every file), the
  augmentation edges, case-only renames, the files whose output is missing, the files without a cached part for
  every plugin, and the files whose hooks' recorded inputs changed (`Cx::env` in a new process; `Cx::record_glob`
  matched again in watch, and always in a new process, which has no matcher). `assumeChangesOnlyAffectDirectDependencies` is refused.
- **What is kept:** each file's facts (text version, references, augmentations), its plugin file parts, the late
  strings its Luau uses and what its hooks read (`Cx::env`, `Cx::record_glob`); in memory, the Luau of files with
  late strings (for re-rendering), the plugins' committed states with the `Shared` made from them and the build
  inputs their `setup` read, and what the last `finish` read; and the artifacts of the last build. The incremental
  state keeps the facts, the parts as the plugins encode them (`Plugin::encode_file_part`), the late strings, the
  hooks' and `setup`'s recorded inputs and the artifacts, so a new process transforms only what changed (a cached
  file whose late string changes value there, with no Luau kept, makes the build start over once, transforming it).
  Only a build without errors commits; a failing build writes nothing and changes none of it.
- **The commit:** Luau and assets, plugin artifacts, then deletions (stale outputs, as roblox-ts's clean of the
  output roots with its `getProjectOutputs` keep set, declaration outputs and both build infos included, plus the
  include folder; links removed as links; artifacts of the last build that this one did not write, and the
  directories they leave empty inside the artifact roots; then plugins' `remove_empty_dir`s), plugin state files,
  the incremental state last.
- **Errors before writes:** the output set (roblox-ts's `getProjectOutputs`: two inputs emitting to one output, an
  output root holding sources) is computed before anything is compiled, and the assets are read before the first
  write, so a failing build writes nothing. The Luau outputs (and re-rendered files) are written all or nothing:
  each is staged in a temporary file beside it first (a directory in the way fails here), and when any staging
  fails, the temporary files go and nothing is renamed or written after; only then are they renamed into place.
  Only I/O errors at a rename (a target another process keeps holding past the retries) or in the later steps can
  stop a commit midway, leaving what was renamed before; the next build repairs it.
- **Watch mode:** `notify` events on the sources, configs, Rojo files, plugin inputs and `watch_dir` roots, and on
  everything the last program load read (logged as tsrs's own watch mode does: files, directories and the paths
  module resolution probed that did not exist, in every ancestor's `node_modules`, `@types`, `typeRoots` and
  `paths`). Packages the build depends on (used, in a type root, named by an unresolved import or by a plugin's
  `resolve_module`) are watched recursively for their `package.json`, JSON, TypeScript and Luau files and folders;
  `node_modules`, its scopes and the type roots for packages added or removed, and for appearing or going themselves
  (also before the first install). A directory watched for the first time is compared with what the build read
  (entries created since the build started, the directory's own modification time, reads the disk now contradicts,
  program files' times and sizes). Plugins' recorded inputs are watched too (globs' roots recursively, what
  `finish` listed, tested or read): an event only they care about starts a rebuild when an answer changed (a file's
  glob transforms that file, a setup glob runs setup, a `finish` read runs `finish`). A trailing 100 ms debounce, `--usePolling`, `--events json` (`build-start` and
  `build-end` lines), and a restart of the watcher process past `RBXTS_WATCH_MEMORY_LIMIT_MB` (default 2048; 0 turns
  it off), since programs are never freed. A crashed rebuild is reported and the next one starts clean.

**Deviations from roblox-ts** (beside those listed in `build::outputs`): a case-only rename of a source renames its
output too, so the Rojo instance follows; roblox-ts writes into the existing file and keeps its old name (a roblox-ts
bug, kept fixed on purpose).

**Watch parity** (W1): `rbxts-bench watch-parity --source <project> --script <file.json> --scratch <dir> [...]` copies
the project, applies a scripted edit sequence to the copy as watch rebuilds (or `--incremental` builds), and after
every step compares the output tree with a clean build of the same sources seeded with the watcher's plugin states
(a failing step must write nothing and report the clean build's errors). Scripts: `crates/rbxts_bench/watch-scripts`
(roblox-ts's tests project) and `crates/rbxts_transform/tests/watch_fixtures` (`cargo test -p rbxts_transform --test
watch`, with the `stamp` plugin, without plugins and incremental; `--link-packages` lets a script install packages in
the copy). `--no-clean` runs the watcher's builds alone, for
timings and the working set (`bench/RESULTS.md`, S2).
