use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// One DPL statement entry in the map file.
/// `label` is the assembly symbol (e.g. `dpl_stmt_3`).
/// `line`  is the 1-based source line it corresponds to.
#[derive(Debug, Deserialize, Clone)]
pub struct StmtEntry {
    pub label: String,
    pub line: u32,
}

/// How a variable is stored on the stack.
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VarKind {
    Int,
    Float,
}

/// Stack location and type for one variable.
#[derive(Debug, Deserialize, Clone)]
pub struct VarEntry {
    /// Positive offset: variable lives at [rbp - rbp_offset].
    pub rbp_offset: i32,
    pub kind: VarKind,
}

/// The full source map produced by the Zig compiler alongside the `.s` file.
#[derive(Debug, Deserialize)]
pub struct SourceMap {
    pub statements: Vec<StmtEntry>,
    pub variables: HashMap<String, VarEntry>,
}

impl SourceMap {
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let map: SourceMap = serde_json::from_str(&data)?;
        Ok(map)
    }

    /// Return the index of the statement whose line is closest to `line`.
    pub fn stmt_idx_for_line(&self, line: u32) -> Option<usize> {
        self.statements
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| s.line.abs_diff(line))
            .map(|(i, _)| i)
    }

    // /// Return the exact line numbers that have statements (for display).
    // pub fn statement_lines(&self) -> Vec<u32> {
    //     self.statements.iter().map(|s| s.line).collect()
    // }
}
