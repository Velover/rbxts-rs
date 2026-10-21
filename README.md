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
