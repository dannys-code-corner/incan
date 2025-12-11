//! Stdlib/prelude loading utilities

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::frontend::{lexer, parser};
use crate::frontend::ast::Program;

/// Parsed module with its source (for error reporting)
pub struct ParsedModule {
    pub name: String,
    /// Path segments for nested modules (e.g., ["db", "models"] for db::models)
    pub path_segments: Vec<String>,
    pub source: String,
    pub ast: Program,
}

/// Find the stdlib directory relative to the compiler or workspace
pub fn find_stdlib_dir() -> Option<PathBuf> {
    // Try relative to current directory (development mode)
    let dev_stdlib = Path::new("stdlib");
    if dev_stdlib.exists() && dev_stdlib.is_dir() {
        return Some(dev_stdlib.to_path_buf());
    }

    // Try relative to executable location
    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // Check exe_dir/stdlib
            let stdlib = exe_dir.join("stdlib");
            if stdlib.exists() && stdlib.is_dir() {
                return Some(stdlib);
            }
            // Check exe_dir/../stdlib (for target/debug or target/release)
            if let Some(parent) = exe_dir.parent() {
                let stdlib = parent.join("stdlib");
                if stdlib.exists() && stdlib.is_dir() {
                    return Some(stdlib);
                }
                // Check exe_dir/../../stdlib (for target/debug -> project root)
                if let Some(grandparent) = parent.parent() {
                    let stdlib = grandparent.join("stdlib");
                    if stdlib.exists() && stdlib.is_dir() {
                        return Some(stdlib);
                    }
                }
            }
        }
    }

    // Try INCAN_STDLIB environment variable
    if let Ok(stdlib_path) = env::var("INCAN_STDLIB") {
        let path = PathBuf::from(stdlib_path);
        if path.exists() && path.is_dir() {
            return Some(path);
        }
    }

    None
}

/// Parse a single prelude trait file (optional, may not exist)
pub fn parse_prelude_file(stdlib_dir: &Path, relative_path: &str) -> Option<ParsedModule> {
    let path = stdlib_dir.join(relative_path);
    if !path.exists() {
        return None;
    }

    let source = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return None,
    };

    let tokens = match lexer::lex(&source) {
        Ok(t) => t,
        Err(_) => return None,
    };

    let ast = match parser::parse(&tokens) {
        Ok(a) => a,
        Err(_) => return None,
    };

    // Parse path segments from relative path (e.g., "derives/debug.incn" -> ["derives", "debug"])
    let path_segments: Vec<String> = relative_path
        .trim_end_matches(".incn")
        .split('/')
        .map(|s| s.to_string())
        .collect();

    let module_name = path_segments.join("_");

    Some(ParsedModule {
        name: module_name,
        path_segments,
        source,
        ast,
    })
}

/// Load prelude modules from stdlib
pub fn load_prelude() -> Vec<ParsedModule> {
    let mut prelude_modules = Vec::new();

    if let Some(stdlib_dir) = find_stdlib_dir() {
        // Load individual trait files in dependency order
        let prelude_files = [
            "derives/debug.incn",
            "derives/display.incn",
            "derives/eq.incn",
            "derives/ord.incn",
            "derives/clone.incn",
            "derives/default.incn",
        ];

        for file in prelude_files {
            if let Some(module) = parse_prelude_file(&stdlib_dir, file) {
                prelude_modules.push(module);
            }
        }

        // Load the main prelude file last (it re-exports from the derives)
        if let Some(module) = parse_prelude_file(&stdlib_dir, "prelude.incn") {
            prelude_modules.push(module);
        }
    }

    prelude_modules
}
