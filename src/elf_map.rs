use anyhow::Result;
use object::{Object, ObjectSymbol};
use std::collections::HashMap;
use std::path::Path;

/// Parse the ELF binary and return a map of  label → virtual address
/// for every `dpl_stmt_N` symbol.  Works because we compile with `-no-pie`,
/// so the symbol addresses in `.symtab` equal the runtime load addresses.
pub fn resolve_stmt_addresses(binary_path: &Path) -> Result<HashMap<String, u64>> {
    let data = std::fs::read(binary_path)?;
    let file = object::File::parse(&*data)?;

    let mut map = HashMap::new();
    for sym in file.symbols() {
        if let Ok(name) = sym.name() {
            if name.starts_with("dpl_stmt_") && sym.address() != 0 {
                map.insert(name.to_string(), sym.address());
            }
        }
    }

    Ok(map)
}
