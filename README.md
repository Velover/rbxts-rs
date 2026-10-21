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

Dumps of the real project stay outside the repo (scratchpad `rbxts-rs/project-dumps`). `cargo test -p rbxts_project
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
in `RBXTS_RS_FIXTURE_NODE_MODULES` (default: the roblox-ts tests project's `node_modules`), and compares byte for byte.

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

`rbxtsc` (crate `rbxts_cli`) is that binary without plugins. `rbxts-bench build --plugin <name>` registers the test
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
  texts, the plugin list and tsconfig entries) or when a plugin's `setup` runs again; otherwise roblox-ts's closure
  (changed files plus everything that reaches them through the reversed references), TypeScript's global rule (a
  changed or reached script file, `declare global` or ambient module declaration invalidates every file), the
  augmentation edges, case-only renames, and the files whose output is missing. `assumeChangesOnlyAffectDirectDependencies` is refused.
- **What is kept:** each file's facts (text version, references, augmentations), its plugin file parts and the late
  strings its Luau uses (in memory; the Luau of files with late strings, for re-rendering), the plugins' committed
  states with the `Shared` made from them and the build inputs their `setup` read, and the artifacts of the last
  build. Only a build without errors commits; a failing build writes nothing and changes none of it.
- **The commit:** Luau and assets, plugin artifacts, then deletions (stale outputs, as roblox-ts's clean of the
  output roots with its `getProjectOutputs` keep set, declaration outputs and both build infos included, plus the
  include folder; links removed as links; artifacts of the last build that this one did not write), plugin state
  files, the incremental state last.
- **Errors before writes:** the output set (roblox-ts's `getProjectOutputs`: two inputs emitting to one output, an
  output root holding sources) is computed before anything is compiled, and the assets are read before the first
  write, so a failing build writes nothing; only I/O errors can stop a commit midway.
- **Watch mode:** `notify` events (the sources, configs, Rojo files, plugin inputs and `watch_dir` roots; under
  `node_modules`, never recursively: `node_modules` and its scopes for packages added or removed, and the packages in
  use, named by an unresolved import or by a plugin's `resolve_module` for their `package.json` and declaration
  files; a directory created or moved in counts, a directory's own modification does not), a trailing 100 ms
  debounce, `--usePolling`, `--events json` (`build-start` and
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
