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
