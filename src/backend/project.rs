//! Project generator - creates the output Rust project structure
//!
//! Generates:
//! - Cargo.toml with dependencies
//! - src/main.rs or src/lib.rs
//! - Invokes cargo build

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const INCAN_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Project generator for creating runnable Rust projects from Incan code
pub struct ProjectGenerator {
    /// Output directory for the generated project
    output_dir: PathBuf,
    /// Project name
    name: String,
    /// Whether this is a binary (true) or library (false)
    is_binary: bool,
    /// Whether serde is needed (for Serialize/Deserialize derives)
    needs_serde: bool,
    /// Whether tokio is needed (for async runtime)
    needs_tokio: bool,
    /// Whether axum is needed (for web framework)
    needs_axum: bool,
    /// Additional Rust crate dependencies from `rust::` imports
    /// Key: crate name, Value: optional version spec (if None, uses latest)
    rust_crate_deps: std::collections::HashMap<String, Option<String>>,
}

impl ProjectGenerator {
    pub fn new(output_dir: impl AsRef<Path>, name: &str, is_binary: bool) -> Self {
        Self {
            output_dir: output_dir.as_ref().to_path_buf(),
            name: name.to_string(),
            is_binary,
            needs_serde: false,
            needs_tokio: false,
            needs_axum: false,
            rust_crate_deps: std::collections::HashMap::new(),
        }
    }
    
    /// Enable serde support (for JSON serialization)
    pub fn with_serde(mut self) -> Self {
        self.needs_serde = true;
        self
    }
    
    /// Set whether serde is needed
    pub fn set_needs_serde(&mut self, needs: bool) {
        self.needs_serde = needs;
    }
    
    /// Enable tokio support (for async runtime)
    pub fn with_tokio(mut self) -> Self {
        self.needs_tokio = true;
        self
    }
    
    /// Set whether tokio is needed
    pub fn set_needs_tokio(&mut self, needs: bool) {
        self.needs_tokio = needs;
    }
    
    /// Enable axum support (for web framework)
    pub fn with_axum(mut self) -> Self {
        self.needs_axum = true;
        self
    }
    
    /// Set whether axum is needed
    pub fn set_needs_axum(&mut self, needs: bool) {
        self.needs_axum = needs;
    }
    
    /// Add a Rust crate dependency from `import rust::crate_name`
    /// Uses a default version mapping for common crates, otherwise uses latest
    pub fn add_rust_crate(&mut self, crate_name: &str) {
        // Common crate versions (maintain a mapping of known-good versions)
        let version = match crate_name {
            "serde" => Some(r#"{ version = "1.0", features = ["derive"] }"#.to_string()),
            "serde_json" => Some(r#""1.0""#.to_string()),
            "tokio" => Some(r#"{ version = "1", features = ["rt-multi-thread", "macros", "time", "sync"] }"#.to_string()),
            "time" => Some(r#"{ version = "0.3", features = ["formatting", "macros"] }"#.to_string()),
            "chrono" => Some(r#"{ version = "0.4", features = ["serde"] }"#.to_string()),
            "reqwest" => Some(r#"{ version = "0.11", features = ["json"] }"#.to_string()),
            "uuid" => Some(r#"{ version = "1.0", features = ["v4", "serde"] }"#.to_string()),
            "rand" => Some(r#""0.8""#.to_string()),
            "regex" => Some(r#""1.0""#.to_string()),
            "anyhow" => Some(r#""1.0""#.to_string()),
            "thiserror" => Some(r#""1.0""#.to_string()),
            "tracing" => Some(r#""0.1""#.to_string()),
            "clap" => Some(r#"{ version = "4.0", features = ["derive"] }"#.to_string()),
            "log" => Some(r#""0.4""#.to_string()),
            "env_logger" => Some(r#""0.10""#.to_string()),
            "sqlx" => Some(r#"{ version = "0.7", features = ["runtime-tokio-native-tls", "postgres"] }"#.to_string()),
            "futures" => Some(r#""0.3""#.to_string()),
            "bytes" => Some(r#""1.0""#.to_string()),
            "itertools" => Some(r#""0.12""#.to_string()),
            // Use latest for unknown crates
            _ => None,
        };
        self.rust_crate_deps.insert(crate_name.to_string(), version);
    }
    
    /// Add a Rust crate with a specific version spec
    pub fn add_rust_crate_with_version(&mut self, crate_name: &str, version_spec: &str) {
        self.rust_crate_deps.insert(crate_name.to_string(), Some(version_spec.to_string()));
    }

    /// Generate the project structure (single-file mode)
    pub fn generate(&self, rust_code: &str) -> io::Result<()> {
        // Create directories
        let src_dir = self.output_dir.join("src");
        fs::create_dir_all(&src_dir)?;

        // Write Cargo.toml
        let cargo_toml = self.generate_cargo_toml();
        fs::write(self.output_dir.join("Cargo.toml"), cargo_toml)?;

        // Write main source file
        let main_file = if self.is_binary {
            src_dir.join("main.rs")
        } else {
            src_dir.join("lib.rs")
        };
        fs::write(main_file, rust_code)?;

        Ok(())
    }

    /// Generate the project structure with multiple module files (flat)
    ///
    /// # Arguments
    /// * `main_code` - The main.rs code (without mod declarations, they will be prepended)
    /// * `modules` - HashMap of module name to module code (e.g., "models" -> "pub struct User { ... }")
    pub fn generate_multi(&self, main_code: &str, modules: &HashMap<String, String>) -> io::Result<()> {
        // Create directories
        let src_dir = self.output_dir.join("src");
        fs::create_dir_all(&src_dir)?;

        // Write Cargo.toml
        let cargo_toml = self.generate_cargo_toml();
        fs::write(self.output_dir.join("Cargo.toml"), cargo_toml)?;

        // Write each module file
        for (module_name, module_code) in modules {
            let module_file = src_dir.join(format!("{}.rs", module_name));
            fs::write(module_file, module_code)?;
        }

        // Build main.rs with the crate-level prelude first, then mod declarations.
        // Crate attributes (`#![...]`) must appear before any Rust items (including `mod ...;`),
        // so we insert module declarations immediately after the crate-level allow attribute.
        let mut full_main = String::new();
        full_main.push_str(main_code);

        if !modules.is_empty() {
            // Add mod declarations for each module (sorted for deterministic output)
            let mut module_names: Vec<_> = modules.keys().collect();
            module_names.sort();
            let mods: String = module_names
                .iter()
                .map(|m| format!("mod {};\n", m))
                .collect();

            // Insert right after the crate-level allow attribute line (if present),
            // otherwise prepend (best-effort).
            if let Some(attr_pos) = full_main.find("#![allow(") {
                let line_end = full_main[attr_pos..]
                    .find('\n')
                    .map(|o| attr_pos + o + 1)
                    .unwrap_or(full_main.len());
                full_main.insert_str(line_end, &mods);
                full_main.insert_str(line_end + mods.len(), "\n");
            } else {
                full_main = format!("{}\n{}", mods, full_main);
            }
        }

        // Write main source file
        let main_file = if self.is_binary {
            src_dir.join("main.rs")
        } else {
            src_dir.join("lib.rs")
        };
        fs::write(main_file, full_main)?;

        Ok(())
    }

    /// Generate the project structure with nested module directories
    ///
    /// This creates proper Rust module hierarchy:
    /// - `from db::models import User` creates `src/db/mod.rs` and `src/db/models.rs`
    /// - main.rs gets `mod db;` (top-level only)
    ///
    /// # Arguments
    /// * `main_code` - The main.rs code (without mod declarations, they will be prepended)
    /// * `modules` - HashMap of path segments to module code (e.g., ["db", "models"] -> "pub struct User { ... }")
    pub fn generate_nested(&self, main_code: &str, modules: &HashMap<Vec<String>, String>) -> io::Result<()> {
        let src_dir = self.output_dir.join("src");
        fs::create_dir_all(&src_dir)?;

        // Write Cargo.toml
        let cargo_toml = self.generate_cargo_toml();
        fs::write(self.output_dir.join("Cargo.toml"), cargo_toml)?;

        // Collect all unique directory paths and their submodules
        // For ["db", "models"], we need:
        //   - src/db/ directory
        //   - src/db/mod.rs with "pub mod models;"
        //   - src/db/models.rs with the code
        let mut dir_submodules: HashMap<Vec<String>, Vec<String>> = HashMap::new();
        let mut top_level_modules: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (path_segments, _) in modules {
            if path_segments.len() >= 1 {
                top_level_modules.insert(path_segments[0].clone());
            }
            
            // For each intermediate directory, track what submodules it contains
            for i in 0..path_segments.len() {
                let dir_path: Vec<String> = path_segments[..i].to_vec();
                let submodule = &path_segments[i];
                dir_submodules
                    .entry(dir_path)
                    .or_default()
                    .push(submodule.clone());
            }
        }

        // Remove duplicates from submodule lists
        for subs in dir_submodules.values_mut() {
            subs.sort();
            subs.dedup();
        }

        // Create directories and mod.rs files for intermediate directories
        for (dir_path, submodules) in &dir_submodules {
            if dir_path.is_empty() {
                // This is the root level - handled by main.rs
                continue;
            }

            // Create the directory
            let mut dir = src_dir.clone();
            for segment in dir_path {
                dir = dir.join(segment);
            }
            fs::create_dir_all(&dir)?;

            // Create mod.rs with pub mod declarations
            let mod_rs_content: String = submodules
                .iter()
                .map(|s| format!("pub mod {};", s))
                .collect::<Vec<_>>()
                .join("\n");
            
            let mod_rs_path = dir.join("mod.rs");
            fs::write(mod_rs_path, format!("{}\n", mod_rs_content))?;
        }

        // Write each module's code file
        for (path_segments, module_code) in modules {
            // Build the file path: src/db/models.rs for ["db", "models"]
            let mut file_path = src_dir.clone();
            for segment in &path_segments[..path_segments.len() - 1] {
                file_path = file_path.join(segment);
            }
            fs::create_dir_all(&file_path)?;
            
            let file_name = format!("{}.rs", path_segments.last().unwrap());
            file_path = file_path.join(file_name);
            
            fs::write(file_path, module_code)?;
        }

        // Build main.rs with the crate-level prelude first, then top-level mod declarations.
        // Crate attributes (`#![...]`) must appear before any Rust items (including `mod ...;`),
        // so we insert module declarations immediately after the crate-level allow attribute.
        let mut full_main = String::new();
        full_main.push_str(main_code);

        let mut sorted_top: Vec<_> = top_level_modules.into_iter().collect();
        sorted_top.sort();
        if !sorted_top.is_empty() {
            let mods: String = sorted_top
                .iter()
                .map(|m| format!("mod {};\n", m))
                .collect();

            if let Some(attr_pos) = full_main.find("#![allow(") {
                let line_end = full_main[attr_pos..]
                    .find('\n')
                    .map(|o| attr_pos + o + 1)
                    .unwrap_or(full_main.len());
                full_main.insert_str(line_end, &mods);
                full_main.insert_str(line_end + mods.len(), "\n");
            } else {
                full_main = format!("{}\n{}", mods, full_main);
            }
        }

        // Write main source file
        let main_file = if self.is_binary {
            src_dir.join("main.rs")
        } else {
            src_dir.join("lib.rs")
        };
        fs::write(main_file, full_main)?;

        Ok(())
    }

    /// Generate Cargo.toml content
    fn generate_cargo_toml(&self) -> String {
        let crate_type = if self.is_binary {
            r#"[[bin]]
name = "{name}"
path = "src/main.rs""#
        } else {
            r#"[lib]
name = "{name}"
path = "src/lib.rs""#
        };
        
        // Build dependencies list
        let mut deps = Vec::new();
        
        // Track which crates we've already added (to avoid duplicates)
        let mut added_crates: std::collections::HashSet<&str> = std::collections::HashSet::new();
        
        if self.needs_serde {
            deps.push(r#"serde = { version = "1.0", features = ["derive"] }"#.to_string());
            deps.push(r#"serde_json = "1.0""#.to_string());
            added_crates.insert("serde");
            added_crates.insert("serde_json");
        }
        
        if self.needs_axum {
            // Axum needs tokio with net feature and full features for web serving
            deps.push(r#"axum = "0.7""#.to_string());
            deps.push(r#"tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "sync", "net"] }"#.to_string());
            added_crates.insert("axum");
            added_crates.insert("tokio");
        } else if self.needs_tokio {
            deps.push(r#"tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "sync"] }"#.to_string());
            added_crates.insert("tokio");
        }
        
        // Add dependencies from rust:: imports
        for (crate_name, version_spec) in &self.rust_crate_deps {
            // Skip if already added above
            if added_crates.contains(crate_name.as_str()) {
                continue;
            }
            
            let dep_line = if let Some(spec) = version_spec {
                format!("{} = {}", crate_name, spec)
            } else {
                // Use "*" for latest version (cargo will resolve to latest compatible)
                format!("{} = \"*\"", crate_name)
            };
            deps.push(dep_line);
        }
        
        let dependencies = if deps.is_empty() {
            "# No additional dependencies".to_string()
        } else {
            deps.join("\n")
        };

        format!(
            r#"[package]
name = "{name}"
version = "{incan_version}"
edition = "2021"

# Generated by the Incan compiler

[dependencies]
{dependencies}

{crate_type}
"#,
            name = self.name,
            incan_version = INCAN_VERSION,
            dependencies = dependencies,
            crate_type = crate_type.replace("{name}", &self.name)
        )
    }

    /// Build the project using cargo
    pub fn build(&self) -> io::Result<BuildResult> {
        let output = Command::new("cargo")
            .arg("build")
            .arg("--release")
            .current_dir(&self.output_dir)
            .output()?;

        Ok(BuildResult {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }

    /// Run the project using cargo
    /// 
    /// Uses inherited stdio so output streams to terminal in real-time
    /// (important for long-running processes like web servers)
    ///
    /// Note: This is only used by `incan run` during development.
    /// Production deployments run the generated binary directly.
    pub fn run(&self) -> io::Result<RunResult> {
        let mut child = Command::new("cargo")
            .arg("run")
            .arg("--release")
            .current_dir(&self.output_dir)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;

        let status = child.wait()?;

        Ok(RunResult {
            success: status.success(),
            stdout: String::new(),  // Output went directly to terminal
            stderr: String::new(),
            exit_code: status.code(),
        })
    }

    /// Get the path to the built binary
    pub fn binary_path(&self) -> PathBuf {
        self.output_dir
            .join("target")
            .join("release")
            .join(&self.name)
    }
}

/// Result of a cargo build
#[derive(Debug)]
pub struct BuildResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Result of running the built program
#[derive(Debug)]
pub struct RunResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_cargo_toml_generation() {
        let generator = ProjectGenerator::new("/tmp/test", "hello", true);
        let toml = generator.generate_cargo_toml();
        assert!(toml.contains("name = \"hello\""));
        assert!(toml.contains("[[bin]]"));
    }

    #[test]
    fn test_generate_multi_creates_mod_declarations() {
        let temp_dir = std::env::temp_dir().join("incan_test_multi");
        let _ = fs::remove_dir_all(&temp_dir); // Clean up any previous test
        
        let generator = ProjectGenerator::new(&temp_dir, "test_multi", true);
        
        let mut modules = HashMap::new();
        modules.insert("models".to_string(), "pub struct User { pub name: String }".to_string());
        modules.insert("utils".to_string(), "pub fn greet() -> String { \"hello\".to_string() }".to_string());
        
        let main_code = "fn main() { println!(\"Hello\"); }";
        
        generator.generate_multi(main_code, &modules).unwrap();
        
        // Check main.rs has mod declarations
        let main_content = fs::read_to_string(temp_dir.join("src/main.rs")).unwrap();
        assert!(main_content.contains("mod models;"));
        assert!(main_content.contains("mod utils;"));
        assert!(main_content.contains("fn main()"));
        
        // Check module files exist
        assert!(temp_dir.join("src/models.rs").exists());
        assert!(temp_dir.join("src/utils.rs").exists());
        
        // Check module content
        let models_content = fs::read_to_string(temp_dir.join("src/models.rs")).unwrap();
        assert!(models_content.contains("pub struct User"));
        
        let utils_content = fs::read_to_string(temp_dir.join("src/utils.rs")).unwrap();
        assert!(utils_content.contains("pub fn greet"));
        
        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_generate_multi_empty_modules() {
        let temp_dir = std::env::temp_dir().join("incan_test_multi_empty");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let generator = ProjectGenerator::new(&temp_dir, "test_empty", true);
        let modules = HashMap::new();
        let main_code = "fn main() {}";
        
        generator.generate_multi(main_code, &modules).unwrap();
        
        let main_content = fs::read_to_string(temp_dir.join("src/main.rs")).unwrap();
        // Should just be the main code, no mod declarations
        assert_eq!(main_content, "fn main() {}");
        
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
