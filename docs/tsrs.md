# tsrs: how rbxts-rs depends on it, and how to update it

rbxts-rs links tsrs, the Rust port of TypeScript 7, directly as Rust crates in the same process. There is no wrapper,
no IPC and no patched tsrs code. Everything roblox-ts-specific lives in rbxts-rs. That includes the places where we
emulate TypeScript 5 behaviour that TypeScript 7 changed.

## The fork
- **Where:** a worktree of the tsrs clone at `E:\Projects\TS\Flamework\worktrees\tsrs-rbxts-fork`, on branch
  `rbxts-fork`. `Cargo.toml` points at its crates by absolute path, so every rbxts-rs worktree uses the same tsrs.
- **What:** tsrs **v0.18.0** (`5f92808`) plus commits that change only visibility, `pub(crate)` → `pub`. That is
  19 lines in 13 files:

| Commit | Item | File | Used for |
|---|---|---|---|
| 63b22eb | `Checker::create_array_type` | `tsrs_checker/src/checker_12.rs` | the transform's type queries |
| 63b22eb | `Checker::get_string_literal_type` | `tsrs_checker/src/checker_12.rs` | the transform's type queries |
| 63b22eb | `Checker::get_number_literal_type` | `tsrs_checker/src/checker_12.rs` | the transform's type queries |
| 63b22eb | `Checker::get_type_of_assignment_pattern` | `tsrs_checker/src/services.rs` | destructuring assignments |
| cf8498b | `Program::get_semantic_diagnostics_with_checker` | `tsrs_compiler/src/program.rs` | checking that honours `@ts-ignore` and the other comment directives |
| adadb25 | `Checker::get_contextual_signature` | `tsrs_checker/src/checker_05.rs` | nothing since TypeScript 5's context sensitivity is marked on the tree (below); kept, as the fork is shared |
| 87958da | `Checker::is_readonly_array_type` | `tsrs_checker/src/checker_11.rs` | Flamework's macro types (`flamework_rs`) |
| 87958da | `Checker::get_literal_type_from_property` | `tsrs_checker/src/checker_13.rs` | Flamework's macros (`flamework_rs`) |
| 87958da | `is_declaration_readonly` | `tsrs_checker/src/utilities.rs`, `tsrs_checker/src/lib.rs` | Flamework's macro types (`flamework_rs`) |
| 0f5217e | `Checker::choose_overload` | `tsrs_checker/src/checker_05.rs` | TS5's TS2769 text (`rbxts_project/src/overload_errors.rs`) |
| 0f5217e | `Checker::reorder_candidates` | `tsrs_checker/src/checker_05.rs` | TS5's TS2769 text (`rbxts_project/src/overload_errors.rs`) |
| 0f5217e | `Checker::is_signature_applicable` | `tsrs_checker/src/checker_05.rs` | TS5's TS2769 text (`rbxts_project/src/overload_errors.rs`) |
| 0f5217e | `Checker::add_implementation_success_elaboration` | `tsrs_checker/src/checker_05.rs` | TS5's TS2769 text (`rbxts_project/src/overload_errors.rs`) |
| 0f5217e | `Checker::get_effective_call_arguments` | `tsrs_checker/src/checker_15.rs` | TS5's TS2769 text (`rbxts_project/src/overload_errors.rs`) |
| 0f5217e | `Checker::signature_to_string` | `tsrs_checker/src/printer.rs` | TS5's TS2769 text (`rbxts_project/src/overload_errors.rs`) |
| 0f5217e | `Checker::check_expression` | `tsrs_checker/src/checker_04.rs` | TS5's TS2769 text (`rbxts_project/src/overload_errors.rs`) |
| 0f5217e | `Checker::get_optional_expression_type` | `tsrs_checker/src/checker_14.rs` | TS5's TS2769 text (`rbxts_project/src/overload_errors.rs`) |
| aa35a73 | `Checker::get_uninstantiated_jsx_signatures_of_type` | `tsrs_checker/src/jsx.rs` | TS5's TS2769 text for JSX elements (`rbxts_project/src/overload_errors.rs`) |

  The branch also carries 67b5330 (overload internals made public) and its revert c796de98, which cancel out.

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
- `get_exports_of_module` keeps the symbol table's insertion order, which is TypeScript 5's: the binder binds a
  block's function declarations first (`bind_each_statement_functions_first`), and `export *` adds its names after the
  module's own. rbxts-rs uses that order for export tables and namespaces (`TransformState::get_module_exports`);
  TypeScript 7's API sorts the exports with `compare_symbols_exported`, which is not used.
- An omitted binding element (`[, a]`) is a nameless `BindingElement`.
- `Node::text()` panics on a `QualifiedName`.
- Positions are UTF-8 byte offsets. `tsrs_ast::positionmap` converts them to UTF-16 where text positions reach the
  output.
- **Context-sensitive functions.** TypeScript 5 takes every function expression and object literal method without
  type parameters and without a `this` parameter for context sensitive (its implicit `this` is contextually typed);
  TypeScript 7's `has_context_sensitive_parameters` takes only one that uses `this` (its `ContainsThis` flag).
  That decides whether such a function gets its contextual `this` (`function () {}` in a `(this: Foo) => void` slot
  is `function(self)` in stock's output), and in an overloaded call it decides which overload fixes that `this`
  (TypeScript 5 fixes it from the first overload it checks the function against in full, even one it then rejects),
  generic inference, and when no overload matches (TS2769). `rbxts_project` (`mark_ts5_context_sensitive_functions`)
  sets `ContainsThis` on those functions after binding, before anything is checked, so TypeScript 7's checker judges
  them as TypeScript 5's; watch marks each file it binds again. Nothing else reads the flag on a function node:
  re-check that (`ContainsThis` in `tsrs_checker` and `tsrs_ast`) after an update. TypeScript 7 also takes a
  generator whose `yield` is context sensitive for context sensitive (`has_context_sensitive_yield_expression`),
  which no flag decides: see "A context-sensitive `yield`" below.
- **TS2769's elaboration.** When no overload matches a call and two or three were tried, TypeScript 5 lists the error
  of each (`Overload 1 of 2, '<signature>', gave the following error.`, each with its own chain, the related
  information of all of them on the error); TypeScript 7 (`report_call_resolution_errors`) elaborates only the last
  (`The last overload gave the following error.` and `The last overload is declared here.`), as TypeScript 5 does
  from four on. `rbxts_project/src/overload_errors.rs` makes such a TS2769 again as TypeScript 5 does: it asks the
  call's resolution again of the checker that checked the file (`reorder_candidates`, then `choose_overload` over
  the subtype and assignable relations, as `resolve_call` does) for the candidates that failed on an argument, and
  reports each one's error (`is_signature_applicable`), instantiated as inference made it, its signature written as a
  call (`WriteCallStyleSignature`: a construct signature without `new`, as TypeScript 5's `signatureToString`), the
  error on `getErrorNodeForCallNode`'s node when the overloads' errors are in different places. Every call
  `resolve_call` resolves: calls (optional chains included), `new` expressions, tagged templates, decorators (under
  their head message, TS1238 to TS1241) and JSX elements (an intrinsic element has one signature).
  `rbxts_cli`'s pretty printer adds the line end TypeScript 5's `formatDiagnosticsWithColorAndContext` prints for its
  (always present, maybe empty) related information array. Tests: `rbxts_cli/tests/fixtures/overload_error*`, stock's
  stdout byte for byte.
- **The span of a diagnostic on an anonymous function expression.** TypeScript 7's `get_error_range_for_node` spans the
  name the function is assigned to (`get_name_of_declaration`: `c1` in `const c1 = function () {}`); TypeScript 5's
  `getErrorSpanForNode` spans the function's own name, else its first token (`function`, `async`). Such a
  diagnostic is TS2683's related information "An outer value of 'this' is shadowed by this container." and TS7011:
  `rbxts_project` (`ts5_function_expression_spans`) moves each one spanning such a name to the function's first token
  (test `rbxts_cli/tests/fixtures/this_shadowed`).
- `implements` entries are TypeReference nodes. `getIndexTypeOfType` takes a key type. A property access through an
  index signature gives an `__index` symbol. There is no `__String`.
- TypeScript 7 gives the related information of `TS1007` ("The parser expected to find a '}' to match the '{' token
  here.") a length of 0; rbxtsc prints it with TypeScript 5's length of 1 (`rbxts_cli/src/pretty.rs`), so its
  underline matches stock's.
- **Automatic type directives.** Without `types` in the tsconfig, TypeScript 5 includes every package of the type
  roots in the program; TypeScript 7 (tsrs `get_automatic_type_directive_names`) includes none (only `"types": ["*"]`
  lists the type roots, by TypeScript 5's rule, with TypeScript 5's wording where the two differ: TS2688's "Entry
  point for implicit type library", "Cannot find name 'process'" without "add node to the types field"). rbxts-rs
  restores TypeScript 5's rule: `rbxts_project` puts `"types": ["*"]` into the parsed options when the tsconfig has
  no `types` (`add_automatic_type_directives`; roblox-ts's own check of `types` still sees the tsconfig's), so a
  project, or a referenced project, whose sources import no package still sees `@rbxts/compiler-types` (TypeScript 7
  alone stops at `TS2318: Cannot find global type 'Array'`). The type roots are listed through the program's file
  system, so watch and incremental builds see a package appear, change or go. On the real project that loads 70 more declaration files, from 8
  packages nothing imports, as TypeScript 5 does.
- **`baseUrl`.** TypeScript 7 removed it (`TS5102`, and an import that only resolves through it is `TS2307`).
  `rbxts_project` emulates TypeScript 5 (`emulate_base_url`): the `paths` substitutions are made absolute against
  `baseUrl`, `"*": ["<baseUrl>/*"]` takes the names no pattern matches (TypeScript 5 tries `baseUrl` only for those: a
  matched pattern that resolves nothing is TS2307 in both), and `baseUrl` is cleared.
- **`rootDirs` without `rootDir`.** With an output directory and no `rootDir`, TypeScript 5 lays the outputs out under
  the common source directory of the emitted files; TypeScript 7 under the tsconfig's directory, refusing a project
  whose files have another one (`TS5011`). `rbxts_project` gives the program TypeScript 5's common source directory
  as its `rootDir` when it is not the tsconfig's (`ts5_root_dir`; from the root files, and the program is made again
  in the rare case that its files reach further), and roblox-ts and plugins see the tsconfig's options
  (`Project::options`).
- **Threading:** the checker is not `Send`, and `get_type_checker` takes a non-reentrant guard. Checking needs a
  512 MiB stack. Set only `checkers`: `single_threaded` also makes parsing and binding serial.

## TypeScript 5 behaviour rbxts-rs does not reproduce
Known, accepted differences from stock roblox-ts on TypeScript 5, each inherent to TypeScript 7. A few make a build
fail where stock's passes, and the entry says how to write it for both; one can change a diagnostic's text. The
last section lists Luau behaviour that stock's output, and so rbxts-rs's, can meet.

- **Union and mapped key order.** Where an output lists the properties of a union or a mapped type, TypeScript 7's
  type ids order them, not TypeScript 5's (`util/ts5_property_order.rs` restores TypeScript 5's order where it can be
  derived from the program). It cannot for a union whose members have the very same symbols (`PXY | PYX`: the first
  member's order is used), a mapped type whose `as` clause makes several names from one key (`` `get${K}` |
  `set${K}` ``: TypeScript 7's order), and guards and the plugin facade's `getFields`, which use the raw order (as
  TypeScript 5's `Record`/`Pick` key order follows its own type ids). Flamework sorts where the order reaches its
  output.

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
- **Grammar errors TypeScript 7 added.** TypeScript 7's checker reports some grammar errors TypeScript 5.9 does not,
  for example `error TS18064: Private identifiers cannot be used in destructuring patterns.` (`const { #x: v } = obj`).
  A file with one fails to build in rbxts-rs, where stock roblox-ts reports its own diagnostic for the construct when
  it reaches the file (or nothing, when an earlier file's error stops it). Both refuse the code; only the message
  differs.
- **Diagnostic message text.** Every diagnostic matches stock in its code, file, position, exit code and the rule
  that a failing build writes nothing. Its message text follows TypeScript 7 where the text is the checker's own
  wording or type printing. Ported back to TypeScript 5's text: TS2769 for calls, `new`, tagged templates,
  decorators and JSX opening elements (`rbxts_project/src/overload_errors.rs`), and the TS2683/TS7011 spans.
  Known remaining cases, each present before the stockgaps round and accepted by the user (2026-10-27):
  - **Printed type order.** Literal unions print in TypeScript 7's order (`"a" | "b" | "c"`, where TypeScript 5
    gives `"a" | "c" | "b"`), and so do the keys of a mapped or `Record` type and the properties of a JSX attributes
    type (`{ a: number; children: string; }`, where TypeScript 5 gives `{ children: string; a: number; }`).
  - **Index-signature related information.** TypeScript 7 adds "The expected type comes from this index signature."
    where TypeScript 5 prints none.
  - **JSX children of overloaded components.** When the last overload fails on a child, TypeScript 7 reports a
    TS2769 at each failing child with "The last overload gave the following error.", while TypeScript 5 reports one
    "Overload N of M" TS2769 at the element. For a child-count error (TS2745/TS2746), TypeScript 5 also prints the
    TS2745/TS2746 on its own, and its chain has no "Overload N of M" line.

  TypeScript 7.1 is expected to shift some of this text again. Code, place and exit stay the contract.
- **Declaration maps.** TypeScript 7 maps some declarations (constructors, a specifier `transformPaths` rewrites)
  differently from TypeScript 5: `.d.ts.map` files are not byte-equal to stock roblox-ts's. The `.d.ts` files are.
- **A context-sensitive `yield`.** TypeScript 7 takes a generator function for context sensitive when a `yield` in it
  yields a context-sensitive expression (`has_context_sensitive_yield_expression`); TypeScript 5 looks only at
  parameters and returns. After `mark_ts5_context_sensitive_functions` (every function expression without a `this`
  parameter is context sensitive in both), this differs only for a generator function expression or method with an
  explicit `this` parameter, no unannotated parameter and such a `yield`, and only where it is an argument of a call
  whose overload choice or inference depends on when it is checked. TypeScript 7 checks it after the other arguments,
  TypeScript 5 with them: `g<T>(make: (this: void) => Generator<(x: T) => void>, value: T)` called as
  `g(function* (this: void) { yield (x) => { const n: number = x; }; }, "s")` infers `T` from `"s"` first in
  TypeScript 7 (`Type 'string' is not assignable to type 'number'.`) and fixes it to `unknown` in TypeScript 5 (`Type
  'unknown' is not assignable to type 'number'.`). No flag decides it (the check walks the yields), so it is not
  reproduced; the test `rbxts_cli/tests/fixtures/yield_context_sensitive` keeps both stdouts (`stock-stdout.txt` is
  stock's). With the `this` parameter left out, or the parameter annotated, both agree.

## Deliberate differences: roblox-ts bugs rbxts-rs fixes
Not TypeScript 7 matters, listed here too so every difference from stock is in one place (the README has them with
the build): the output fixes of the divergence manifest (`crates/rbxts_bench/parity/divergences.json`), and one in
diagnostics.

- **A comment directive's error span.** Without `--allowCommentDirectives`, roblox-ts reports each `@ts-ignore`,
  `@ts-expect-error` and `@ts-nocheck` comment. Stock's `createDiagnosticWithLocation` gives that diagnostic the
  length `range.end` instead of `end - pos` (a roblox-ts bug), so a comment after the file's first character is
  underlined from the comment on, past it, to the end of the file once the comment is far enough down. rbxts-rs
  underlines the comment (`rbxts_transform/src/compile.rs` `comment_directive_diagnostics`). Four files of roblox-ts's
  `src/diagnostics` show it (`noIndexWithoutCall.8`, `noLabeledStatement.2` and `.3`, `noPrivateIdentifier.3`): the
  same error, place and exit code, a shorter underline.

## Luau behaviour that roblox-ts output can meet
**Project rule: rbxts-rs only transpiles, 1:1 like roblox-ts, and never works around Luau behaviour.** Where stock's
output meets the Luau behaviour below, rbxts-rs emits the same output: the `luau_o2` fixture pins it as stock's.
Lune 0.10.5's Luau and Studio's differ in places, as noted.

- **Fast calls read a local argument late (by design).** Luau compiles a call of a builtin (`math.max`,
  `table.insert`, `bit32.bor`, `string.sub`, `tonumber`, `buffer.writestring`, …) with at most three arguments to a
  fast call, which at `--!optimize 2` reads a local argument after the later arguments ran. When a later argument
  reassigns that local through a closure, the builtin sees the new value; O0 and O1 (and JavaScript) see the old one.
  Roblox, asked about it, answered that this is known behaviour of fast calls, by design, and that binary operators
  can show the same effect; their advice, if it affects you: lower the optimisation level (`--!optimize 1`), or avoid
  non-lexical side effects on captured locals within a single expression. Studio (O2 by default for scripts; seen
  there with `table.insert`) and Lune agree:

  ```lua
  local a = 1
  local function rebind() a = 100; return 2 end
  print(math.max(a, rebind())) -- O0/O1: 2; O2: 100
  ```

  With four or more arguments the call is no fast call, and `select` is one only as `select(n, ...)` (Lune 0.10.5).
- **A numeric `for` from a `-0` start (reported to Roblox, no answer yet).** In Studio, at O2 and so by default, `i`
  starts at `0`, not `-0`, both for a constant start and for one that is `-0` only at runtime; O0 and O1 give `-0`.
  Lune 0.10.5 shows it only for a constant start (the engine versions differ):

  ```lua
  for i = -0, 0 do print(1 / i) end -- O0/O1: -inf; O2: inf (Studio and Lune)
  local box = { start = -0 }
  for i = box.start, 0 do print(1 / i) end -- O0/O1: -inf; O2: inf in Studio, -inf in Lune 0.10.5
  ```

  Stock's output has such loops from `--optimizedLoops` (`for (let i = -0; …)`, a `const` of `-0`, `0 * -1`) and
  from `$range`.
