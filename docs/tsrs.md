# tsrs: how rbxts-rs depends on it, and how to update it

rbxts-rs links tsrs, the Rust port of TypeScript 7, directly as Rust crates in the same process. There is no wrapper,
no IPC and no patched tsrs code. Everything roblox-ts-specific lives in rbxts-rs. That includes the places where we
emulate TypeScript 5 behaviour that TypeScript 7 changed.

## The fork
- **Where:** a worktree of the tsrs clone at `E:\Projects\TS\Flamework\worktrees\tsrs-rbxts-fork`, on branch
  `rbxts-fork`. `Cargo.toml` points at its crates by absolute path, so every rbxts-rs worktree uses the same tsrs.
- **What:** tsrs **v0.18.0** (`5f92808`) plus three commits that change only visibility, `pub(crate)` → `pub`. That is
  6 lines in 4 files:

| Commit | Item | File | Used for |
|---|---|---|---|
| 63b22eb | `Checker::create_array_type` | `tsrs_checker/src/checker_12.rs` | the transform's type queries |
| 63b22eb | `Checker::get_string_literal_type` | `tsrs_checker/src/checker_12.rs` | the transform's type queries |
| 63b22eb | `Checker::get_number_literal_type` | `tsrs_checker/src/checker_12.rs` | the transform's type queries |
| 63b22eb | `Checker::get_type_of_assignment_pattern` | `tsrs_checker/src/services.rs` | destructuring assignments |
| cf8498b | `Program::get_semantic_diagnostics_with_checker` | `tsrs_compiler/src/program.rs` | checking that honours `@ts-ignore` and the other comment directives |
| adadb25 | `Checker::get_contextual_signature` | `tsrs_checker/src/checker_05.rs` | TS5's contextual `this` in `util/is_method.rs` |

- **Rule:** the fork gets visibility flips and nothing else. Behaviour fixes and workarounds go in rbxts-rs with a
  `// rbxts-rs:` comment. If a needed function is too entangled to make public, copy a small helper into rbxts-rs
  with a note instead.
- **Upstreaming:** these flips are reasonable public-API requests to tsrs. Each one upstream accepts is one less to
  carry.

## Updating tsrs
Pin the version and update on purpose, never automatically. tsrs is pre-1.0, so a release can change its API.

1. **Rebase the flips.** Fetch the new release in the tsrs clone. Make a new branch from the release tag and
   cherry-pick the three commits above. If upstream has made an item public already, drop that flip.
   - Keep the old `rbxts-fork` branch until the update is merged, so you can roll back.
2. **Update `Cargo.lock`.** It started as a copy of tsrs's lock file, so the transitive versions match what tsrs tests
   with. Re-sync the shared dependencies with the new tsrs lock file.
3. **Fix API breaks.** Build the workspace and fix every compile error: renamed or reshaped checker methods, AST
   accessors, flags and symbol lookups. This is mechanical, and the compiler finds all of it.
4. **Catch behaviour changes.** These are the subtle part, because they compile cleanly but change the output. Run
   parity on both corpora and compare with the baselines:
   - the roblox-ts tests project, 72 files;
   - the real project with Flamework off, 605 files.

   Use `merge\parity.sh` and `merge\parity-{tests,real}-main.txt` in the scratchpad's `batch\rbxts-rs` folder, plus
   `cargo test --workspace` for the fixtures and plugin tests. Some files differ from stock on purpose, where
   rbxts-rs fixes a roblox-ts bug. `rbxts-bench build --parity REF --divergences crates/rbxts_bench/parity/divergences.json
   --corpus {roblox-ts-tests,real-noplugin}` reports those as `known-fix` while their output is unchanged (see
   `crates/rbxts_bench/src/divergences.rs`). Any other differing file, or a `stale` one, is a behaviour change. Find
   the tsrs commit that caused it, then either adapt rbxts-rs (with a `// rbxts-rs:` note) or report it upstream.
5. **Re-measure** `rbxts-bench load` and a full build (see `bench/RESULTS.md`), so a performance regression doesn't
   slip in.
6. **Process:** as with every change, a worker does the update and a fresh validator re-runs parity before it merges.

## TypeScript 7 behaviour the port depends on
Re-check these after an update; they are where tsrs's behaviour reaches our output.
- `getTrueType()` without an argument is the *regular* true type (`regular_true_type`), not the fresh one. `??`
  lowering depends on it.
- Module exports are sorted with `compare_symbols_exported`.
- An omitted binding element (`[, a]`) is a nameless `BindingElement`.
- `Node::text()` panics on a `QualifiedName`.
- Positions are UTF-8 byte offsets. `tsrs_ast::positionmap` converts them to UTF-16 where text positions reach the
  output.
- **TS7 gives no contextual `this`** to a function expression that doesn't use `this`; TS5 did. `is_method` emulates
  TS5 through `get_contextual_signature` (see `util/is_method.rs`).
- `implements` entries are TypeReference nodes. `getIndexTypeOfType` takes a key type. A property access through an
  index signature gives an `__index` symbol. There is no `__String`.
- TypeScript 7 gives the related information of `TS1007` ("The parser expected to find a '}' to match the '{' token
  here.") a length of 0; rbxtsc prints it with TypeScript 5's length of 1 (`rbxts_cli/src/pretty.rs`), so its
  underline matches stock's.
- **Automatic type directives.** Without `types` in the tsconfig, TypeScript 5 includes every package of the type
  roots in the program; TypeScript 7 (tsrs `get_automatic_type_directive_names`) includes none (only `"types": ["*"]`
  lists the type roots). rbxts-rs restores TypeScript 5's rule: `rbxts_project` lists the type roots' packages into
  the parsed options' `types` when the tsconfig has none (`add_automatic_type_directives`), so a project, or a
  referenced project, whose sources import no package still sees `@rbxts/compiler-types` (TypeScript 7 alone stops at
  `TS2318: Cannot find global type 'Array'`). On the real project that loads 70 more declaration files, from 8
  packages nothing imports, as TypeScript 5 does.
- **Threading:** the checker is not `Send`, and `get_type_checker` takes a non-reentrant guard. Checking needs a
  512 MiB stack. Set only `checkers`: `single_threaded` also makes parsing and binding serial.

## TypeScript 5 behaviour rbxts-rs does not reproduce
Known, accepted differences from stock roblox-ts on TypeScript 5, each inherent to TypeScript 7. None makes an output
wrong; a few make a build fail where stock's passes, and the entry says how to write it for both.

- **Export order.** A module's export table (`return { … }`) and a namespace's exports list their names in the order
  TypeScript 7 gives a module's exports (tsrs sorts them with `compare_symbols_exported`), where TypeScript 5 keeps
  declaration order. The same names reach the same values; only the field order differs.
- **Union and mapped key order.** Where an output lists the properties of a union or a mapped type, TypeScript 7's
  type ids order them, not TypeScript 5's (`util/ts5_property_order.rs` restores TypeScript 5's order where it can be
  derived from the program). It cannot for a union whose members have the very same symbols (`PXY | PYX`: the first
  member's order is used), a mapped type whose `as` clause makes several names from one key (`` `get${K}` |
  `set${K}` ``: TypeScript 7's order), and guards and the plugin facade's `getFields`, which use the raw order (as
  TypeScript 5's `Record`/`Pick` key order follows its own type ids). Flamework sorts where the order reaches its
  output.
- **`baseUrl`.** TypeScript 7 removed `baseUrl`, so an import that only resolves through it is `error TS2307: Cannot
  find module`. Use `paths` with paths relative to the tsconfig (`"paths": { "*": ["./src/*"] }`), which both
  resolve the same way.

- **Declaration-emit symlinks.** With `declaration: true`, TypeScript 5's pre-emit diagnostics run the declaration
  emitter, and when it needs an `import("…")` specifier for a type from another module, its module-specifier code
  (`getAllModulePathsWorker`) resolves every runtime dependency (`dependencies`, `peerDependencies`,
  `optionalDependencies`) of the importing file's `package.json` and adds their links to the program's symlink cache.
  roblox-ts's `guessVirtualPath` reads that cache, so from then on (in transform order) an import of a package's real
  path, a relative import or a `paths` alias into a sibling package that is also a linked dependency, goes through
  `node_modules` (`include/node_modules/@scope/pkg`). rbxts-rs's symlink map (`util/symlink_cache.rs`) is built from
  the program's resolutions only, as TypeScript 5's cache starts out, so such an import keeps the package's own path
  (`packages/pkg`). Not modelled: rbxts-rs runs tsrs's declaration diagnostics and emit, but tsrs's specifier
  lookups do not reach rbxts-rs's symlink map, and reproducing when the emitter asks for a specifier would mean
  tracking it for every file. To get the `node_modules` path in both, import
  the package by its name. (Probe: a relative import of a sibling `file:` dependency, with a declaration build.)
- **`rootDirs` without `rootDir`.** With an `outDir` and no `rootDir`, TypeScript 5 lays the output out under the
  common source directory of the inputs. TypeScript 7 refuses that when the common source directory is not the
  tsconfig's own directory: `error TS5011: The common source directory of 'tsconfig.json' is './src'. The 'rootDir'
  setting must be explicitly set to this or another path to adjust your output's file layout.` A tsconfig that sets
  `rootDirs` (for example `["src/a", "src/b"]`) but not `rootDir` meets it whenever its files share a folder below the
  tsconfig: stock roblox-ts builds it (`out/a/…`, `out/b/…`), rbxts-rs stops at TS5011. The fix is to set `rootDir`
  as well, to that common source directory (`"rootDir": "src"`), which gives the same output in both. rbxts-rs does
  not emulate TypeScript 5's rule: the check is tsrs's option validation.
- **Grammar errors TypeScript 7 added.** TypeScript 7's checker reports some grammar errors TypeScript 5.9 does not,
  for example `error TS18064: Private identifiers cannot be used in destructuring patterns.` (`const { #x: v } = obj`).
  A file with one fails to build in rbxts-rs, where stock roblox-ts reports its own diagnostic for the construct when
  it reaches the file (or nothing, when an earlier file's error stops it). Both refuse the code; only the message
  differs.
- **Declaration maps.** TypeScript 7 maps some declarations (constructors, a specifier `transformPaths` rewrites)
  differently from TypeScript 5: `.d.ts.map` files are not byte-equal to stock roblox-ts's. The `.d.ts` files are.
