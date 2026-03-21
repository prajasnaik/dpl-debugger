mod compiler;
mod debugger;
mod elf_map;
mod repl;
mod source_map;

use anyhow::{Context, Result};
use std::path::PathBuf;

fn usage(prog: &str) {
    eprintln!("Usage: {prog} [--compiler <path>] <file.dpl>");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --compiler <path>   Path to glp_zcompiler binary");
    eprintln!("                      (default: glp_zcompiler, searched in PATH)");
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let prog = &args[0];

    // ── Argument parsing ──────────────────────────────────────────────────────
    let mut source_path: Option<PathBuf> = None;
    let mut compiler_bin = "glp_zcompiler".to_string();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--compiler" => {
                i += 1;
                compiler_bin = args
                    .get(i)
                    .context("--compiler requires an argument")?
                    .clone();
            }
            "--help" | "-h" => {
                usage(prog);
                return Ok(());
            }
            path => {
                if source_path.is_some() {
                    eprintln!("Unexpected argument: {path}");
                    usage(prog);
                    std::process::exit(1);
                }
                source_path = Some(PathBuf::from(path));
            }
        }
        i += 1;
    }

    let source_path = match source_path {
        Some(p) => p,
        None => {
            usage(prog);
            std::process::exit(1);
        }
    };

    // ── Read source text (needed for the list command in the REPL) ────────────
    let source_text = std::fs::read_to_string(&source_path)
        .with_context(|| format!("Cannot read '{}'", source_path.display()))?;

    // ── Compile: .dpl → .s + .map, then .s → binary ──────────────────────────
    println!("=== DPL Debugger ===");
    println!("Source: {}", source_path.display());
    println!("Compiler: {compiler_bin}");
    println!();

    let output = compiler::compile(&source_path, &compiler_bin)?;

    // ── Parse the source map ──────────────────────────────────────────────────
    let source_map = source_map::SourceMap::load(&output.map_path)
        .with_context(|| format!("Cannot load map file '{}'", output.map_path.display()))?;

    println!(
        "  {} statements, {} variables mapped",
        source_map.statements.len(),
        source_map.variables.len()
    );

    // ── Resolve label → virtual address from the ELF ─────────────────────────
    let label_to_addr = elf_map::resolve_stmt_addresses(&output.binary_path)?;
    println!("  {} symbols resolved from ELF", label_to_addr.len());
    println!();

    // ── Launch the process under ptrace ──────────────────────────────────────
    let mut dbg = debugger::Debugger::launch(&output.binary_path, source_map, &label_to_addr)?;

    // ── Hand off to the interactive REPL ─────────────────────────────────────
    repl::run_repl(&mut dbg, &source_text)?;

    Ok(())
}
