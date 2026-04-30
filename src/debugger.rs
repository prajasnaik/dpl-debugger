use anyhow::{Context, Result};
use nix::sys::ptrace;
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{execv, fork, ForkResult, Pid};
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::path::Path;

use crate::source_map::{SourceMap, VarKind};

// ─── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum DebugEvent {
    /// Execution stopped at a known DPL statement.
    HitBreakpoint { stmt_idx: usize, line: u32 },
    /// The process exited normally.
    Exited { status: i32 },
    /// The process was killed by a signal.
    Signaled { signal: Signal },
}

#[derive(Debug, Clone)]
pub enum DplValue {
    Int(i64),
    Float(f64),
    String(String),
}

impl std::fmt::Display for DplValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DplValue::Int(i) => write!(f, "{i}"),
            DplValue::Float(v) => write!(f, "{v}"),
            DplValue::String(s) => write!(f, "{s}"),
        }
    }
}

// ─── Debugger ─────────────────────────────────────────────────────────────────

pub struct Debugger {
    pub child: Pid,
    pub source_map: SourceMap,

    /// stmt_idx → virtual address (0 = unknown / not in binary).
    pub stmt_addresses: Vec<u64>,

    /// virtual address → stmt_idx (reverse lookup).
    addr_to_stmt: HashMap<u64, usize>,

    /// User-set persistent breakpoints (by stmt_idx).
    pub user_breakpoints: HashSet<usize>,

    /// Currently active INT3 patches: address → original byte that was there.
    active_breakpoints: HashMap<u64, u8>,

    /// Statement index we are currently stopped at (None = not started / running).
    pub current_stmt: Option<usize>,
}

impl Debugger {
    /// Fork, exec the binary under ptrace, and return a ready-to-use Debugger.
    /// An automatic one-shot breakpoint is inserted at `dpl_stmt_0` so that
    /// the first `run` / `continue` stops at the first DPL source line.
    pub fn launch(
        binary_path: &Path,
        source_map: SourceMap,
        label_to_addr: &HashMap<String, u64>,
    ) -> Result<Self> {
        // Build ordered address table aligned with source_map.statements.
        let stmt_addresses: Vec<u64> = source_map
            .statements
            .iter()
            .map(|s| *label_to_addr.get(&s.label).unwrap_or(&0))
            .collect();

        let mut addr_to_stmt: HashMap<u64, usize> = HashMap::new();
        for (i, &addr) in stmt_addresses.iter().enumerate() {
            if addr != 0 {
                addr_to_stmt.insert(addr, i);
            }
        }

        let binary_cstr =
            CString::new(binary_path.to_str().context("binary path is not UTF-8")?)
                .context("binary path contains NUL byte")?;

        match unsafe { fork().context("fork failed")? } {
            ForkResult::Child => {
                ptrace::traceme().expect("PTRACE_TRACEME failed");
                execv(&binary_cstr, &[&binary_cstr]).expect("execv failed");
                unreachable!()
            }
            ForkResult::Parent { child } => {
                // Wait for the initial SIGTRAP that fires after execv.
                match waitpid(child, None)? {
                    WaitStatus::Stopped(_, Signal::SIGTRAP) => {}
                    other => anyhow::bail!("unexpected initial stop: {other:?}"),
                }

                let mut dbg = Debugger {
                    child,
                    source_map,
                    stmt_addresses,
                    addr_to_stmt,
                    user_breakpoints: HashSet::new(),
                    active_breakpoints: HashMap::new(),
                    current_stmt: None,
                };

                // Auto-breakpoint at the very first statement so `run` stops immediately.
                if let Some(&addr) = dbg.stmt_addresses.first() {
                    if addr != 0 {
                        dbg.insert_bp(addr)?;
                    }
                }

                Ok(dbg)
            }
        }
    }

    // ─── Breakpoint management ────────────────────────────────────────────────

    fn insert_bp(&mut self, addr: u64) -> Result<()> {
        if self.active_breakpoints.contains_key(&addr) {
            return Ok(()); // already patched
        }
        let word = ptrace::read(self.child, addr as ptrace::AddressType)? as u64;
        let saved = (word & 0xFF) as u8;
        let patched = (word & !0xFF) | 0xCC; // replace low byte with INT3
        unsafe {
            ptrace::write(
                self.child,
                addr as ptrace::AddressType,
                patched as *mut std::ffi::c_void,
            )?;
        }
        self.active_breakpoints.insert(addr, saved);
        Ok(())
    }

    fn remove_bp(&mut self, addr: u64) -> Result<()> {
        if let Some(saved) = self.active_breakpoints.remove(&addr) {
            let word = ptrace::read(self.child, addr as ptrace::AddressType)? as u64;
            let restored = (word & !0xFF) | saved as u64;
            unsafe {
                ptrace::write(
                    self.child,
                    addr as ptrace::AddressType,
                    restored as *mut std::ffi::c_void,
                )?;
            }
        }
        Ok(())
    }

    /// Toggle a user-set persistent breakpoint on the statement nearest to `line`.
    /// Returns the actual source line the breakpoint was placed on.
    pub fn toggle_breakpoint(&mut self, line: u32) -> Result<u32> {
        let idx = self
            .source_map
            .stmt_idx_for_line(line)
            .context("no statements in program")?;
        let actual_line = self.source_map.statements[idx].line;
        let addr = self.stmt_addresses[idx];

        if self.user_breakpoints.contains(&idx) {
            self.user_breakpoints.remove(&idx);
            self.remove_bp(addr)?;
            println!("  Breakpoint removed at line {actual_line}");
        } else {
            self.user_breakpoints.insert(idx);
            if addr != 0 {
                self.insert_bp(addr)?;
            }
            println!("  Breakpoint set at line {actual_line}");
        }
        Ok(actual_line)
    }

    // ─── Execution control ────────────────────────────────────────────────────

    /// Continue execution until the next breakpoint or program exit.
    pub fn do_continue(&mut self) -> Result<DebugEvent> {
        self.resume_from_current(false)
    }

    /// Step to the next executed DPL statement following real control flow.
    pub fn do_step(&mut self) -> Result<DebugEvent> {
        self.resume_from_current(true)
    }

    /// Internal: handle current stop (including stepping over user INT3), optionally
    /// plant one-shot breakpoints for step semantics, then PTRACE_CONT + wait.
    fn resume_from_current(&mut self, is_step: bool) -> Result<DebugEvent> {
        let current_idx = self.current_stmt;

        if let Some(idx) = self.current_stmt {
            let addr = self.stmt_addresses[idx];

            // The instruction was already restored and RIP set back in wait_for_event.
            // If it's a user breakpoint we must single-step past it before re-installing,
            // otherwise we'd immediately retrigger INT3 on the next continue.
            if self.user_breakpoints.contains(&idx) {
                ptrace::step(self.child, None)?;
                match waitpid(self.child, None)? {
                    WaitStatus::Exited(_, code) => return Ok(DebugEvent::Exited { status: code }),
                    WaitStatus::Signaled(_, sig, _) => {
                        return Ok(DebugEvent::Signaled { signal: sig })
                    }
                    _ => {} // expected: single-step SIGTRAP
                }
                self.insert_bp(addr)?; // re-install after stepping past it
            }

            self.current_stmt = None;
        }

        // Plant one-shot breakpoints for step semantics at all statement labels
        // except the current one and user-persistent breakpoints.
        let mut temp_addrs: Vec<u64> = Vec::new();
        if is_step {
            for (i, &addr) in self.stmt_addresses.iter().enumerate() {
                if addr == 0 {
                    continue;
                }
                if Some(i) == current_idx {
                    continue;
                }
                if self.user_breakpoints.contains(&i) {
                    continue;
                }
                temp_addrs.push(addr);
            }
            temp_addrs.sort_unstable();
            temp_addrs.dedup();

            for &addr in &temp_addrs {
                self.insert_bp(addr)?;
            }
        }

        ptrace::cont(self.child, None)?;
        let event = self.wait_for_event()?;

        // Remove any remaining one-shot breakpoints.
        if is_step {
            for addr in temp_addrs {
                if self.active_breakpoints.contains_key(&addr) {
                    self.remove_bp(addr)?;
                }
            }
        }

        Ok(event)
    }

    /// Block until the child stops at one of our breakpoints, exits, or is signalled.
    fn wait_for_event(&mut self) -> Result<DebugEvent> {
        loop {
            match waitpid(self.child, None)? {
                WaitStatus::Exited(_, code) => return Ok(DebugEvent::Exited { status: code }),
                WaitStatus::Signaled(_, sig, _) => return Ok(DebugEvent::Signaled { signal: sig }),

                WaitStatus::Stopped(_, Signal::SIGTRAP) => {
                    let regs = ptrace::getregs(self.child)?;
                    // INT3 is 1 byte; after it fires RIP points one past the breakpoint.
                    let hit_addr = regs.rip - 1;

                    if let Some(&stmt_idx) = self.addr_to_stmt.get(&hit_addr) {
                        // Restore the original instruction and back up RIP.
                        self.remove_bp(hit_addr)?;
                        let mut regs = ptrace::getregs(self.child)?;
                        regs.rip = hit_addr;
                        ptrace::setregs(self.child, regs)?;

                        let line = self.source_map.statements[stmt_idx].line;
                        self.current_stmt = Some(stmt_idx);
                        return Ok(DebugEvent::HitBreakpoint { stmt_idx, line });
                    } else {
                        // Spurious SIGTRAP (e.g. execv initial stop already consumed,
                        // or single-step SIGTRAP leaking through). Just continue.
                        ptrace::cont(self.child, None)?;
                    }
                }

                WaitStatus::Stopped(_, sig) => {
                    // Forward any other signal to the child.
                    ptrace::cont(self.child, Some(sig))?;
                }

                _ => {
                    ptrace::cont(self.child, None)?;
                }
            }
        }
    }

    // ─── Variable inspection ─────────────────────────────────────────────────

    pub fn read_variable(&self, name: &str) -> Result<Option<(DplValue, Option<DplValue>)>> {
        // Allow callers to pass either `x` or `x``; internally normalize to base name.
        let base_name = name.strip_suffix('`').unwrap_or(name);

        let entry = match self.source_map.variables.get(base_name) {
            Some(e) => e,
            None => return Ok(None),
        };

        let regs = ptrace::getregs(self.child)?;
        // Variable lives at rbp - rbp_offset (offset is stored as positive i32).
        let addr = (regs.rbp as i64 - entry.rbp_offset as i64) as u64;
        let value = self.read_value_for_kind(addr, &entry.kind)?;

        // Primed slot may not exist for variables that are never prime-assigned.
        let primed_name = format!("{}`", base_name);
        let primed_value = if let Some(primed_entry) = self.source_map.variables.get(&primed_name)
        {
            let primed_addr = (regs.rbp as i64 - primed_entry.rbp_offset as i64) as u64;
            Some(self.read_value_for_kind(primed_addr, &primed_entry.kind)?)
        } else {
            None
        };

        Ok(Some((value, primed_value)))
    }

    pub fn read_all_variables(&self) -> Vec<(String, Result<(DplValue, Option<DplValue>)>)> {
        let mut vars: Vec<(String, Result<(DplValue, Option<DplValue>)>)> = self
            .source_map
            .variables
            .keys()
            .filter(|name| !name.ends_with('`'))
            .map(|name| {
                let val = self
                    .read_variable(name)
                    .and_then(|v| v.context("variable missing from source map"));
                (name.clone(), val)
            })
            .collect();
        // Sort alphabetically for stable output.
        vars.sort_by(|a, b| a.0.cmp(&b.0));
        vars
    }

    fn read_value_for_kind(&self, stack_addr: u64, kind: &VarKind) -> Result<DplValue> {
        let raw = ptrace::read(self.child, stack_addr as ptrace::AddressType)?;
        let value = match kind {
            VarKind::Int => DplValue::Int(raw as i64),
            VarKind::Float => DplValue::Float(f64::from_bits(raw as u64)),
            VarKind::String => {
                let ptr = raw as u64;
                DplValue::String(self.read_c_string(ptr, 4096)?)
            }
        };
        Ok(value)
    }

    fn read_c_string(&self, addr: u64, max_len: usize) -> Result<String> {
        if addr == 0 {
            return Ok("<null>".to_string());
        }

        // Read memory in aligned machine words and extract bytes locally.
        // This avoids issuing potentially unsafe unaligned ptrace reads.
        let word_size = std::mem::size_of::<usize>() as u64;
        let word_mask = !(word_size - 1);

        let mut bytes = Vec::new();
        let mut cached_word_addr: Option<u64> = None;
        let mut cached_word: u64 = 0;

        for i in 0..max_len {
            let byte_addr = addr + i as u64;
            let aligned_addr = byte_addr & word_mask;

            if cached_word_addr != Some(aligned_addr) {
                cached_word = ptrace::read(self.child, aligned_addr as ptrace::AddressType)? as u64;
                cached_word_addr = Some(aligned_addr);
            }

            let byte_index = (byte_addr - aligned_addr) as u32;
            let b = ((cached_word >> (byte_index * 8)) & 0xFF) as u8;
            if b == 0 {
                break;
            }
            bytes.push(b);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}
