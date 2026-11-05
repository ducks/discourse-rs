# Test impact

`crates/test-impact` runs only the tests a change can reach, from a map of
the workspace functions each test ran.

## Recording

The `Test impact` workflow (on demand, or on a pull request labeled
`test-impact-record`) builds the tests with `-C instrument-coverage` and runs
them all with `test-impact runner` as cargo's target runner. Each test runs
with a profile of its own; `llvm-profdata merge --sparse` lists the functions
it ran, which are looked up in a table of every function's line range, built
once per binary with `llvm-cov export -empty-profile`. The recordings of its
ten shares merge into the `test-impact-map` artifact.

Locally, with llvm-tools in the toolchain:

```sh
cargo build -p test-impact
export TEST_IMPACT_DIR=$PWD/tmp/test-impact
export TEST_IMPACT_LLVM_BIN=$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin
CARGO_TARGET_DIR=target/impact RUSTFLAGS='-C instrument-coverage' \
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="$PWD/target/debug/test-impact runner" \
  cargo nextest run --profile ci <filters>
LLVM_PROFILE_FILE=/dev/null CARGO_TARGET_DIR=target/impact RUSTFLAGS='-C instrument-coverage' \
  cargo nextest list --profile ci --message-format json <filters> > tmp/test-impact/list.json
target/debug/test-impact build-map tmp/test-impact map.json.gz tmp/test-impact/list.json
```

## Selecting

```sh
filter=$(test-impact select map.json.gz --base origin/main --list list.json)
cargo nextest run --profile ci -E "$filter"
```

`select` diffs the working tree against its merge base with `--base`:

- a hunk in a Rust function selects the tests that ran it (the innermost
  function: a closure is its own); one outside every function (struct
  fields, derives, consts, `use`) selects every test that ran the file;
- a template, or a file named by `include_str!` or `include_bytes!`, selects
  the tests that ran the Rust files naming it, and a template changes every
  template that extends, includes or imports it;
- a write case's `.case.json` or recording selects that case;
- `.config/test-impact.toml` maps other inputs to a filterset, to the tests
  that ran given Rust files, or to nothing;
- tests in `--list` that the map doesn't have are new, and selected.

Anything else runs the whole suite (`all()`), listed on stderr: a file no
rule names, a new source file, a crate root's module list, or a hunk outside
the functions of a file no test ran. Changed functions no test ran are listed
as untested.

`test-impact replay map.json.gz 200` selects for each of the last 200
commits on origin/main, for what selection would have saved.
