//! CLI command implementations

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use crate::frontend::{lexer, parser, typechecker, diagnostics};
use crate::frontend::ast::Program;
use crate::backend::{RustCodegen, ProjectGenerator};
use crate::format::{format_source, format_diff};

use super::prelude::{ParsedModule, load_prelude};

/// Read source file contents
pub fn read_source(file_path: &str) -> String {
    match fs::read_to_string(file_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading file '{}': {}", file_path, e);
            process::exit(1);
        }
    }
}

/// Collect and parse the entry file and all its dependencies
pub fn collect_modules(entry_path: &str) -> Vec<ParsedModule> {
    let path = Path::new(entry_path);
    let base_dir = path.parent().unwrap_or(Path::new("."));

    let _prelude = load_prelude();

    let mut modules = Vec::new();
    let mut processed = HashSet::new();
    // (file_path, module_name, path_segments)
    let mut to_process: Vec<(String, String, Vec<String>)> = vec![
        (entry_path.to_string(), "main".to_string(), vec!["main".to_string()])
    ];

    while let Some((file_path, module_name, path_segments)) = to_process.pop() {
        if processed.contains(&file_path) {
            continue;
        }
        processed.insert(file_path.clone());

        let source = read_source(&file_path);
        let tokens = match lexer::lex(&source) {
            Ok(t) => t,
            Err(errs) => {
                for err in &errs {
                    diagnostics::print_error(&file_path, &source, err);
                }
                process::exit(1);
            }
        };

        let ast = match parser::parse(&tokens) {
            Ok(a) => a,
            Err(errs) => {
                for err in &errs {
                    diagnostics::print_error(&file_path, &source, err);
                }
                process::exit(1);
            }
        };

        // Find imports and add them to process queue
        for decl in &ast.declarations {
            if let crate::frontend::ast::Declaration::Import(import) = &decl.node {
                let import_path = match &import.kind {
                    crate::frontend::ast::ImportKind::Module(path) if !path.segments.is_empty() => {
                        Some(path)
                    }
                    crate::frontend::ast::ImportKind::From { module, .. } if !module.segments.is_empty() => {
                        Some(module)
                    }
                    _ => None,
                };

                if let Some(path) = import_path {
                    if path.segments.is_empty() || path.segments.first() == Some(&"std".to_string()) {
                        continue;
                    }

                    let mut target_dir = base_dir.to_path_buf();

                    if path.is_absolute {
                        let mut project_root = base_dir.to_path_buf();
                        while !project_root.join("Cargo.toml").exists() && !project_root.join("src").exists() {
                            if let Some(parent) = project_root.parent() {
                                project_root = parent.to_path_buf();
                            } else {
                                break;
                            }
                        }
                        if project_root.join("src").exists() {
                            target_dir = project_root.join("src");
                        } else {
                            target_dir = project_root;
                        }
                    } else {
                        for _ in 0..path.parent_levels {
                            target_dir = target_dir.parent().map(|p| p.to_path_buf()).unwrap_or(target_dir);
                        }
                    }

                    let module_segments = match &import.kind {
                        crate::frontend::ast::ImportKind::From { module, .. } => {
                            module.segments.clone()
                        }
                        crate::frontend::ast::ImportKind::Module(p) => {
                            if p.segments.len() > 1 {
                                p.segments[..p.segments.len() - 1].to_vec()
                            } else {
                                p.segments.clone()
                            }
                        }
                        _ => continue,
                    };

                    if module_segments.is_empty() {
                        continue;
                    }

                    let mut dep_path = target_dir.clone();
                    for segment in &module_segments {
                        dep_path = dep_path.join(segment);
                    }

                    dep_path.set_extension("incn");
                    let mut found_path: Option<PathBuf> = None;

                    if dep_path.exists() {
                        found_path = Some(dep_path.clone());
                    } else {
                        dep_path.set_extension("incan");
                        if dep_path.exists() {
                            found_path = Some(dep_path.clone());
                        }
                    }

                    if let Some(path) = found_path {
                        let dep_path_str = path.to_string_lossy().to_string();
                        let module_name = module_segments.join("_");
                        if !processed.contains(&dep_path_str) {
                            to_process.push((dep_path_str, module_name, module_segments.clone()));
                        }
                    }
                }
            }
        }

        modules.push(ParsedModule {
            name: module_name,
            path_segments,
            source,
            ast,
        });
    }

    modules.reverse();
    modules
}

/// Lex and display tokens
pub fn lex_file(file_path: &str) {
    let source = read_source(file_path);
    let tokens = match lexer::lex(&source) {
        Ok(toks) => toks,
        Err(errs) => {
            for err in &errs {
                diagnostics::print_error(file_path, &source, err);
            }
            process::exit(1);
        }
    };

    for tok in &tokens {
        println!("{:?}", tok);
    }
}

/// Parse and display AST
pub fn parse_file(file_path: &str) {
    let source = read_source(file_path);
    let tokens = match lexer::lex(&source) {
        Ok(t) => t,
        Err(errs) => {
            for err in &errs {
                diagnostics::print_error(file_path, &source, err);
            }
            process::exit(1);
        }
    };

    match parser::parse(&tokens) {
        Ok(ast) => {
            println!("{:#?}", ast);
        }
        Err(errs) => {
            for err in &errs {
                diagnostics::print_error(file_path, &source, err);
            }
            process::exit(1);
        }
    }
}

/// Type check a file
pub fn check_file(file_path: &str) {
    let modules = collect_modules(file_path);

    let Some(main_module) = modules.last() else {
        eprintln!("No modules found");
        process::exit(1);
    };

    let deps: Vec<(&str, &Program)> = modules[..modules.len()-1]
        .iter()
        .map(|m| (m.name.as_str(), &m.ast))
        .collect();

    let mut checker = typechecker::TypeChecker::new();
    match checker.check_with_imports(&main_module.ast, &deps) {
        Ok(()) => {
            println!("✓ Type check passed!");
        }
        Err(errs) => {
            for err in &errs {
                diagnostics::print_error(file_path, &main_module.source, err);
            }
            process::exit(1);
        }
    }
}

/// Emit generated Rust code
pub fn emit_rust(file_path: &str) {
    let modules = collect_modules(file_path);

    let Some(main_module) = modules.last() else {
        eprintln!("No modules found");
        process::exit(1);
    };

    let mut codegen = RustCodegen::new();

    for module in &modules[..modules.len()-1] {
        codegen.add_module(&module.name, &module.ast);
    }

    let rust_code = codegen.generate(&main_module.ast);
    println!("{}", rust_code);
}

/// Build an Incan file to a Rust project
pub fn build_file(file_path: &str, output_dir: Option<&String>) {
    let modules = collect_modules(file_path);

    let Some(main_module) = modules.last() else {
        eprintln!("No modules found");
        process::exit(1);
    };

    let dep_modules = &modules[..modules.len()-1];
    let deps: Vec<(&str, &Program)> = dep_modules
        .iter()
        .map(|m| (m.name.as_str(), &m.ast))
        .collect();

    let mut checker = typechecker::TypeChecker::new();
    if let Err(errs) = checker.check_with_imports(&main_module.ast, &deps) {
        for err in &errs {
            diagnostics::print_error(file_path, &main_module.source, err);
        }
        process::exit(1);
    }

    let path = Path::new(file_path);
    let project_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("incan_project");

    let out_dir = output_dir
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("target/incan/{}", project_name));

    // Use multi-file generation if there are dependency modules
    let has_deps = !dep_modules.is_empty();
    
    let mut codegen = RustCodegen::new();
    for module in dep_modules {
        codegen.add_module(&module.name, &module.ast);
    }
    codegen.scan_for_serde(&main_module.ast);
    codegen.scan_for_async(&main_module.ast);
    codegen.scan_for_web(&main_module.ast);
    codegen.scan_for_list_helpers(&main_module.ast);
    let needs_serde = codegen.needs_serde();
    let needs_tokio = codegen.needs_tokio();
    let needs_axum = codegen.needs_axum();
    let rust_crates = collect_rust_crates(&main_module.ast);

    let mut generator = ProjectGenerator::new(&out_dir, project_name, true);
    generator.set_needs_serde(needs_serde);
    generator.set_needs_tokio(needs_tokio);
    generator.set_needs_axum(needs_axum);
    for crate_name in rust_crates {
        generator.add_rust_crate(&crate_name);
    }

    if has_deps {
        // Multi-file generation with nested module paths
        let module_paths: Vec<Vec<String>> = dep_modules.iter().map(|m| m.path_segments.clone()).collect();
        let (main_code, rust_modules) = codegen.generate_multi_file_nested(&main_module.ast, &module_paths);
        
        if let Err(e) = generator.generate_nested(&main_code, &rust_modules) {
            eprintln!("Error generating project: {}", e);
            process::exit(1);
        }
    } else {
        // Single-file generation (no dependencies)
        let rust_code = codegen.generate(&main_module.ast);
        if let Err(e) = generator.generate(&rust_code) {
            eprintln!("Error generating project: {}", e);
            process::exit(1);
        }
    }

    println!("Generated Rust project in: {}", out_dir);
    println!("Building...");

    match generator.build() {
        Ok(result) => {
            if result.success {
                println!("✓ Build successful!");
                println!("Binary: {}", generator.binary_path().display());
            } else {
                eprintln!("Build failed:");
                eprintln!("{}", result.stderr);
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error running cargo: {}", e);
            process::exit(1);
        }
    }
}

/// Build and run an Incan file
pub fn run_file(file_path: &str) {
    let modules = collect_modules(file_path);

    let Some(main_module) = modules.last() else {
        eprintln!("No modules found");
        process::exit(1);
    };

    let dep_modules = &modules[..modules.len()-1];
    let deps: Vec<(&str, &Program)> = dep_modules
        .iter()
        .map(|m| (m.name.as_str(), &m.ast))
        .collect();

    let mut checker = typechecker::TypeChecker::new();
    if let Err(errs) = checker.check_with_imports(&main_module.ast, &deps) {
        for err in &errs {
            diagnostics::print_error(file_path, &main_module.source, err);
        }
        process::exit(1);
    }

    let path = Path::new(file_path);
    let project_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("incan_project");

    let out_dir = format!("target/incan/{}", project_name);

    // Use multi-file generation if there are dependency modules
    let has_deps = !dep_modules.is_empty();
    
    let mut codegen = RustCodegen::new();
    for module in dep_modules {
        codegen.add_module(&module.name, &module.ast);
    }
    codegen.scan_for_serde(&main_module.ast);
    codegen.scan_for_async(&main_module.ast);
    codegen.scan_for_web(&main_module.ast);
    codegen.scan_for_list_helpers(&main_module.ast);
    let needs_serde = codegen.needs_serde();
    let needs_tokio = codegen.needs_tokio();
    let needs_axum = codegen.needs_axum();
    let rust_crates = collect_rust_crates(&main_module.ast);

    let mut generator = ProjectGenerator::new(&out_dir, project_name, true);
    generator.set_needs_serde(needs_serde);
    generator.set_needs_tokio(needs_tokio);
    generator.set_needs_axum(needs_axum);
    for crate_name in rust_crates {
        generator.add_rust_crate(&crate_name);
    }

    if has_deps {
        // Multi-file generation with nested module paths
        let module_paths: Vec<Vec<String>> = dep_modules.iter().map(|m| m.path_segments.clone()).collect();
        let (main_code, rust_modules) = codegen.generate_multi_file_nested(&main_module.ast, &module_paths);
        
        if let Err(e) = generator.generate_nested(&main_code, &rust_modules) {
            eprintln!("Error generating project: {}", e);
            process::exit(1);
        }
    } else {
        // Single-file generation (no dependencies)
        let rust_code = codegen.generate(&main_module.ast);
        if let Err(e) = generator.generate(&rust_code) {
            eprintln!("Error generating project: {}", e);
            process::exit(1);
        }
    }

    match generator.run() {
        Ok(result) => {
            if !result.stdout.is_empty() {
                print!("{}", result.stdout);
            }
            if !result.stderr.is_empty() && !result.success {
                eprint!("{}", result.stderr);
            }
            if let Some(code) = result.exit_code {
                process::exit(code);
            }
        }
        Err(e) => {
            eprintln!("Error running program: {}", e);
            process::exit(1);
        }
    }
}

/// Format Incan source files
pub fn format_files(path: &str, check_mode: bool, diff_mode: bool) {
    let path = Path::new(path);
    let files = collect_incn_files(path);

    if files.is_empty() {
        eprintln!("No .incn files found");
        process::exit(1);
    }

    let mut needs_formatting = false;
    let mut formatted_count = 0;
    let mut error_count = 0;

    for file_path in &files {
        let source = match fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading {}: {}", file_path.display(), e);
                error_count += 1;
                continue;
            }
        };

        match format_source(&source) {
            Ok(formatted) => {
                let changed = source != formatted;

                if diff_mode && changed {
                    println!("--- {}", file_path.display());
                    if let Ok(Some(diff)) = format_diff(&source) {
                        print!("{}", diff);
                    }
                    println!();
                }

                if check_mode {
                    if changed {
                        println!("Would reformat: {}", file_path.display());
                        needs_formatting = true;
                    }
                } else if diff_mode {
                    if changed {
                        needs_formatting = true;
                    }
                } else if changed {
                    if let Err(e) = fs::write(file_path, &formatted) {
                        eprintln!("Error writing {}: {}", file_path.display(), e);
                        error_count += 1;
                    } else {
                        println!("Formatted: {}", file_path.display());
                        formatted_count += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("Error formatting {}: {}", file_path.display(), e);
                error_count += 1;
            }
        }
    }

    if check_mode || diff_mode {
        if needs_formatting {
            let msg = if diff_mode { "need formatting" } else { "would be reformatted" };
            eprintln!("\n{} file(s) {}", files.len(), msg);
            process::exit(1);
        } else {
            println!("✓ {} file(s) already formatted", files.len());
        }
    } else {
        println!("\n✓ {} file(s) formatted, {} error(s)", formatted_count, error_count);
    }

    if error_count > 0 {
        process::exit(1);
    }
}

fn collect_incn_files(path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if path.is_file() {
        if path.extension().map_or(false, |ext| ext == "incn") {
            files.push(path.to_path_buf());
        }
    } else if path.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    let name = entry_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !name.starts_with('.') && name != "target" && name != "node_modules" {
                        files.extend(collect_incn_files(&entry_path));
                    }
                } else if entry_path.extension().map_or(false, |ext| ext == "incn") {
                    files.push(entry_path);
                }
            }
        }
    }

    files
}

/// Collect Rust crate names from imports
pub fn collect_rust_crates(ast: &crate::frontend::ast::Program) -> Vec<String> {
    use crate::frontend::ast::ImportKind;

    let mut crates = Vec::new();

    for decl in &ast.declarations {
        if let crate::frontend::ast::Declaration::Import(import) = &decl.node {
            match &import.kind {
                ImportKind::RustCrate { crate_name, .. } => {
                    if crate_name != "std" && !crates.contains(crate_name) {
                        crates.push(crate_name.clone());
                    }
                }
                ImportKind::RustFrom { crate_name, .. } => {
                    if crate_name != "std" && !crates.contains(crate_name) {
                        crates.push(crate_name.clone());
                    }
                }
                _ => {}
            }
        }
    }

    crates
}
