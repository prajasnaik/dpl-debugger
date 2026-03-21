# DPL-Debugger

This is a source-level debugger for DPL programs. It compiles a `.dpl` file, loads compiler-generated mapping metadata, and lets you debug by DPL source lines (`run`, `step`, `break`, `print`, etc.).

It is designed to work with the DPL compiler output model:
- statement labels (`dpl_stmt_N`) in assembly,
- a JSON source map (`.map`) containing statement-to-line mappings,
- variable stack metadata for runtime inspection.

## Features

- Run and stop at the first DPL statement
- Continue to next breakpoint
- Step to the next executed DPL statement (follows control flow, including loops)
- Toggle breakpoints by source line
- Inspect one variable or all locals
- Source listing around current execution point

## Prerequisites

**Supported Platform:**

- Linux (x86-64)

**Required Software:**

- Rust (Cargo)
- GCC (used internally to link generated assembly)
- DPL compiler binary (for example `dpl-compiler` or `glp_zcompiler`)

## Build

Clone and build:

```sh
git clone https://github.com/prajasnaik/dpl-debugger.git
cd dpl-debugger
cargo build
```

The debugger binary will be available at:

```sh
target/debug/dpl-debugger
```

## Usage

Run with a source file:

```sh
target/debug/dpl-debugger <file.dpl>
```

If your compiler binary is not available in `PATH`, pass it explicitly:

```sh
target/debug/dpl-debugger --compiler <path-to-compiler> <file.dpl>
```

Example (from sibling repositories):

```sh
target/debug/dpl-debugger --compiler ../dpl-compiler/zig-out/bin/dpl-compiler ../dpl-compiler/samples/09_fibonacci.dpl
```

## REPL Commands

Inside the debugger prompt:

- `r` / `run` — start execution
- `c` / `continue` — continue to next breakpoint
- `s` / `step` — step to next executed DPL statement
- `b <line>` / `break <line>` — toggle breakpoint near source line
- `p <var>` / `print <var>` — print one variable
- `locals` — print all variables
- `list` — show source around current line
- `h` / `help` — show help
- `q` / `quit` — exit debugger

## Typical Session

```text
dpl> r
  Stopped at line 6
[line 6] dpl> b 9
  Breakpoint set at line 9
[line 6] dpl> c
  Breakpoint hit at line 9
[line 9] dpl> p a
  a = 55
[line 9] dpl> locals
  a = 55
  b = 34
  i = 9
[line 9] dpl> q
```

## Notes

- The debugger is line-mapped using compiler-emitted metadata, not DWARF.
- Stepping is statement-based and follows runtime control flow.
- Variable reads are based on compiler-provided stack offsets and variable type (`int`/`float`).
