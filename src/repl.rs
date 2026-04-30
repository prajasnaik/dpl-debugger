use anyhow::Result;
use std::io::{self, Write};

use crate::debugger::{DebugEvent, Debugger};

const HELP: &str = "\
Commands:
  r / run          Start execution (stops at first statement)
  c / continue     Continue to next breakpoint
  s / step         Step to the next DPL statement
  b / break <line> Set or toggle a breakpoint at source line <line>
  p / print <var>  Print the value of a variable
  locals           Print all variable values
  list             Show source around current line (±5 lines)
  h / help         Show this message
  q / quit         Exit the debugger
";

fn readline(prompt: &str) -> Result<Option<String>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    let n = io::stdin().read_line(&mut line)?;
    if n == 0 {
        return Ok(None); // EOF
    }
    Ok(Some(line.trim_end_matches('\n').trim_end_matches('\r').to_string()))
}

pub fn run_repl(dbg: &mut Debugger, source_text: &str) -> Result<()> {
    let source_lines: Vec<&str> = source_text.lines().collect();
    let mut program_exited = false;

    println!("\nType 'help' for commands, 'r' to start.");

    loop {
        let prompt = match dbg.current_stmt {
            Some(idx) => {
                let line = dbg.source_map.statements[idx].line;
                format!("[line {line}] dpl> ")
            }
            None if program_exited => "(exited) dpl> ".to_string(),
            None => "dpl> ".to_string(),
        };

        let input = match readline(&prompt)? {
            Some(s) => s,
            None => {
                println!("\nExiting.");
                break;
            }
        };

        let parts: Vec<&str> = input.trim().splitn(2, ' ').collect();
        if parts.is_empty() || parts[0].is_empty() {
            continue;
        }

        match parts[0] {
            // ── help ──────────────────────────────────────────────────────────
            "h" | "help" => print!("{HELP}"),

            // ── run ───────────────────────────────────────────────────────────
            "r" | "run" => {
                if program_exited {
                    println!("Program has already exited. Restart the debugger to run again.");
                    continue;
                }
                if dbg.current_stmt.is_some() {
                    println!("Already running. Use 'continue' or 'step'.");
                    continue;
                }
                println!("Running…");
                handle_event(dbg.do_continue()?, dbg, &source_lines, &mut program_exited);
            }

            // ── continue ──────────────────────────────────────────────────────
            "c" | "continue" => {
                if program_exited {
                    println!("Program has already exited.");
                    continue;
                }
                if dbg.current_stmt.is_none() {
                    println!("Program not started. Type 'run' first.");
                    continue;
                }
                handle_event(dbg.do_continue()?, dbg, &source_lines, &mut program_exited);
            }

            // ── step ──────────────────────────────────────────────────────────
            "s" | "step" => {
                if program_exited {
                    println!("Program has already exited.");
                    continue;
                }
                if dbg.current_stmt.is_none() {
                    // Treat first step like run.
                    handle_event(dbg.do_continue()?, dbg, &source_lines, &mut program_exited);
                } else {
                    handle_event(dbg.do_step()?, dbg, &source_lines, &mut program_exited);
                }
            }

            // ── break <line> ──────────────────────────────────────────────────
            "b" | "break" => {
                let arg = parts.get(1).copied().unwrap_or("").trim();
                match arg.parse::<u32>() {
                    Ok(line) => {
                        dbg.toggle_breakpoint(line)?;
                    }
                    Err(_) => println!("Usage: b <line_number>"),
                }
            }

            // ── print <var> ───────────────────────────────────────────────────
            "p" | "print" => {
                if dbg.current_stmt.is_none() && !program_exited {
                    println!("Program not started.");
                    continue;
                }
                let var_name = parts.get(1).copied().unwrap_or("").trim();
                if var_name.is_empty() {
                    println!("Usage: p <variable>");
                    continue;
                }
                match dbg.read_variable(var_name) {
                    Ok(Some((value, primed))) => {
                        println!("  {} = {}", var_name, value);
                        if let Some(p) = primed {
                            println!("  {}` = {}", var_name, p);
                        }
                    }
                    Ok(None) => println!("  Unknown variable '{var_name}'"),
                    Err(e) => println!("  Error reading '{var_name}': {e}"),
                }
            }

            // ── locals ────────────────────────────────────────────────────────
            "locals" => {
                if dbg.current_stmt.is_none() && !program_exited {
                    println!("Program not started.");
                    continue;
                }
                let vars = dbg.read_all_variables();
                if vars.is_empty() {
                    println!("  (no variables)");
                } else {
                    for (name, result) in &vars {
                        match result {
                            Ok((value, primed)) => {
                                println!("  {} = {}", name, value);
                                if let Some(p) = primed {
                                    println!("  {}` = {}", name, p);
                                }
                            }
                            Err(e) => println!("  {name} = <error: {e}>"),
                        }
                    }
                }
            }

            // ── list ──────────────────────────────────────────────────────────
            "list" => {
                let current_line = dbg
                    .current_stmt
                    .map(|i| dbg.source_map.statements[i].line as usize)
                    .unwrap_or(1);
                print_source(&source_lines, current_line, dbg);
            }

            // ── quit ──────────────────────────────────────────────────────────
            "q" | "quit" | "exit" => break,

            other => println!("Unknown command '{other}'. Type 'help'."),
        }
    }

    Ok(())
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn handle_event(
    event: DebugEvent,
    dbg: &Debugger,
    source_lines: &[&str],
    exited: &mut bool,
) {
    match event {
        DebugEvent::HitBreakpoint { stmt_idx, line } => {
            let is_user_bp = dbg.user_breakpoints.contains(&stmt_idx);
            if is_user_bp {
                println!("  Breakpoint hit at line {line}");
            } else {
                println!("  Stopped at line {line}");
            }
            print_source(source_lines, line as usize, dbg);
        }
        DebugEvent::Exited { status } => {
            println!("  Program exited with status {status}.");
            *exited = true;
        }
        DebugEvent::Signaled { signal } => {
            println!("  Program killed by signal {signal:?}.");
            *exited = true;
        }
    }
}

fn print_source(lines: &[&str], current_line: usize, dbg: &Debugger) {
    let bp_lines: std::collections::HashSet<u32> = dbg
        .user_breakpoints
        .iter()
        .map(|&i| dbg.source_map.statements[i].line)
        .collect();

    let start = current_line.saturating_sub(5);
    let end = (current_line + 5).min(lines.len());

    for (i, line_text) in lines[start..end].iter().enumerate() {
        let lineno = start + i + 1; // 1-based
        let marker = if lineno == current_line { "=>" } else { "  " };
        let bp = if bp_lines.contains(&(lineno as u32)) { "●" } else { " " };
        println!("  {bp}{marker} {:>3} │ {line_text}", lineno);
    }
}
