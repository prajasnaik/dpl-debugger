use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct CompileOutput {
    /// The compiled binary ready for execution.
    pub binary_path: PathBuf,
    /// The generated assembly file.
    pub asm_path: PathBuf,
    /// The source-map JSON sidecar produced alongside the assembly.
    pub map_path: PathBuf,
}

/// Compile a `.dpl` source file all the way to a debuggable binary.
///
/// Steps performed:
///   1. `glp_zcompiler <source> -o <tmp>.s`  → also emits `<tmp>.map`
///   2. `gcc <tmp>.s -o <tmp> -lm -no-pie`   → non-PIE so symbol VAs are fixed
pub fn compile(source_path: &Path, compiler_binary: &str) -> Result<CompileOutput> {
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("source file has no stem")?;

    let tmp = std::env::temp_dir();
    let asm_path = tmp.join(format!("{stem}.s"));
    let binary_path = tmp.join(stem);
    // The Zig compiler writes <stem>.map next to the .s file.
    let map_path = tmp.join(format!("{stem}.map"));

    // ── Step 1: DPL → assembly + source map ──────────────────
    println!("[compile] {} → {}", source_path.display(), asm_path.display());
    let status = Command::new(compiler_binary)
        .arg(source_path)
        .arg("-o")
        .arg(&asm_path)
        .status()
        .with_context(|| format!("failed to run '{compiler_binary}'"))?;

    if !status.success() {
        anyhow::bail!(
            "glp_zcompiler failed (exit {:?}). Is '{compiler_binary}' in PATH?",
            status.code()
        );
    }

    // ── Step 2: assembly → non-PIE binary ────────────────────
    println!("[compile] {} → {}", asm_path.display(), binary_path.display());
    let status = Command::new("gcc")
        .arg(&asm_path)
        .arg("-o")
        .arg(&binary_path)
        .arg("-lm")
        .arg("-no-pie") // critical: fixed VAs so ELF symbol addresses == runtime addresses
        .status()
        .context("failed to run 'gcc'")?;

    if !status.success() {
        anyhow::bail!("gcc failed (exit {:?})", status.code());
    }

    Ok(CompileOutput {
        binary_path,
        asm_path,
        map_path,
    })
}
