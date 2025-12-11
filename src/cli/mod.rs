//! CLI module for the Incan compiler
//!
//! This module provides the command-line interface for the compiler.
//!
//! ## Commands
//!
//! - `build <file>` - Compile to Rust and build executable
//! - `run <file>` - Compile and run the program
//! - `fmt <file|dir>` - Format Incan source files
//! - `test [path]` - Run tests (pytest-style)
//!
//! ## Modules
//!
//! - `commands` - Command implementations
//! - `prelude` - Stdlib/prelude loading
//! - `test_runner` - Test discovery and execution

pub mod commands;
pub mod prelude;
pub mod test_runner;

use std::env;
use std::fs;
use std::process;

/// ASCII art logo - embedded at compile time from assets/logo.txt
const LOGO: &str = include_str!("../../assets/logo.txt");
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Main CLI entry point
pub fn run() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "--help" | "-h" => print_usage(),
        "--version" | "-V" => print_version(),
        "--lex" => {
            if args.len() < 3 {
                eprintln!("Error: --lex requires a file path");
                process::exit(1);
            }
            commands::lex_file(&args[2]);
        }
        "--parse" => {
            if args.len() < 3 {
                eprintln!("Error: --parse requires a file path");
                process::exit(1);
            }
            commands::parse_file(&args[2]);
        }
        "--check" => {
            if args.len() < 3 {
                eprintln!("Error: --check requires a file path");
                process::exit(1);
            }
            commands::check_file(&args[2]);
        }
        "--emit-rust" => {
            if args.len() < 3 {
                eprintln!("Error: --emit-rust requires a file path");
                process::exit(1);
            }
            commands::emit_rust(&args[2]);
        }
        "build" => {
            if args.len() < 3 {
                eprintln!("Error: build requires a file path");
                process::exit(1);
            }
            let output_dir = if args.len() >= 4 { Some(&args[3]) } else { None };
            commands::build_file(&args[2], output_dir);
        }
        "run" => {
            // Support: incan run <file> | incan run -c "<code>" | incan run --command "<code>"
            if args.len() >= 3 && (args[2] == "-c" || args[2] == "--command") {
                // Capture the entire remaining argument tail as the inline program
                let code = if args.len() > 3 {
                    args[3..].join(" ")
                } else {
                    String::new()
                };
                if code.is_empty() {
                    eprintln!("Error: -c/--command requires source code string");
                    process::exit(1);
                }
                // If the snippet already declares a main, leave as-is; otherwise, append a stub main that calls any top-level code.
                // We keep the user code at module scope so imports/expressions (e.g., `import this`) run as written.
                let wrapped = if code.contains("def main") {
                    code
                } else {
                    format!("{code}\n\ndef main() -> Unit:\n  pass\n")
                };
                // Write code to a temporary file and run it.
                let tmp_path = env::temp_dir().join(format!(
                    "incan_cmd_{}_{}.incn",
                    process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0)
                ));
                if let Err(e) = fs::write(&tmp_path, wrapped) {
                    eprintln!("Error writing temporary command file: {}", e);
                    process::exit(1);
                }
                commands::run_file(tmp_path.to_string_lossy().as_ref());
                let _ = fs::remove_file(&tmp_path);
            } else {
                if args.len() < 3 {
                    eprintln!("Error: run requires a file path or -c \"code\"");
                    process::exit(1);
                }
                commands::run_file(&args[2]);
            }
        }
        "fmt" => {
            if args.len() < 3 {
                eprintln!("Error: fmt requires a file or directory path");
                process::exit(1);
            }
            let check_mode = args.iter().any(|a| a == "--check");
            let diff_mode = args.iter().any(|a| a == "--diff");
            let path = args.iter()
                .skip(2)
                .find(|a| !a.starts_with("--"))
                .map(|s| s.as_str())
                .unwrap_or(".");
            commands::format_files(path, check_mode, diff_mode);
        }
        "test" => {
            let verbose = args.iter().any(|a| a == "-v" || a == "--verbose");
            let stop_on_fail = args.iter().any(|a| a == "-x" || a == "--exitfirst");
            let include_slow = args.iter().any(|a| a == "--slow");
            let filter = args.iter()
                .position(|a| a == "-k")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            let path = args.iter()
                .skip(2)
                .find(|a| !a.starts_with("-"))
                .map(|s| s.as_str())
                .unwrap_or(".");
            test_runner::run_tests(path, verbose, stop_on_fail, include_slow, filter);
        }
        file_path => {
            // Default: type check the file
            commands::check_file(file_path);
        }
    }
}

/// Print usage information
fn print_logo() {
    // Color scheme inspired by the wordmark:
    // - Solid blocks (█) = Gold
    // - Shadow blocks (░) = Cyan/Magenta based on position
    let gold = "\x1b[1;33m";
    let cyan = "\x1b[1;36m";
    let magenta = "\x1b[1;35m";
    let reset = "\x1b[0m";
    
    for line in LOGO.lines() {
        let mut colored_line = String::new();
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        
        for (i, ch) in chars.iter().enumerate() {
            let color = if *ch == '░' {
                // Shadow chars: cyan on left half, magenta on right half (diagonal effect)
                if i < len / 2 { cyan } else { magenta }
            } else if *ch == '█' || *ch == '█' {
                gold
            } else {
                gold // Default to gold for spaces and other chars
            };
            colored_line.push_str(color);
            colored_line.push(*ch);
        }
        eprintln!("{}{}", colored_line, reset);
    }
}

fn print_version() {
    print_logo();
    eprintln!("Incan v{}", VERSION);
}

fn print_usage() {
    print_logo();
    eprintln!("Usage: incan <command> [options] <file>");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  build <file> [output_dir]  Compile to Rust and build executable");
    eprintln!("  run <file>                 Compile and run the program");
    eprintln!("  run -c \"code\"            Run inline source string (use shell escaping for quotes)");
    eprintln!("  fmt <file|dir> [--check] [--diff]");
    eprintln!("                             Format Incan source files");
    eprintln!("  test [path] [options]      Run tests (pytest-style)");
    eprintln!("                             -v, --verbose: verbose output");
    eprintln!("                             -x, --exitfirst: stop on first failure");
    eprintln!("                             -k <expr>: filter tests by keyword");
    eprintln!("                             --slow: include slow tests");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --lex <file>       Tokenize only");
    eprintln!("  --parse <file>     Parse only");
    eprintln!("  --check <file>     Type check only");
    eprintln!("  --emit-rust <file> Emit generated Rust code");
    eprintln!("  --help, -h         Show this help");
    eprintln!("  --version, -V      Show version");
}
