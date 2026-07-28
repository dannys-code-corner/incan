Warning: truncated output (original token count: 38920)
Total output lines: 4174

//! Golden snapshot tests for codegen
//!
//! These tests generate Rust code from `.incn` input files and compare the output against stored snapshots.
//! This ensures codegen changes are reviewed and intentional.
//!
//! Run with: `cargo test --test codegen_snapshot_tests`
//! Review changes: `cargo insta review`

use incan::backend::IrCodegen;
use incan::frontend::{lexer, parser};
use std::fs;

#[path = "support/builtin_stdlib.rs"]
mod builtin_stdlib_support;

fn codegen_with_builtin_stdlib_inventory() -> IrCodegen<'static> {
    let mut codegen = IrCodegen::new();
    codegen.set_sdk_provider_module_paths(builtin_stdlib_support::artifact_module_paths());
    codegen
}

/// Generate Rust code from Incan source
fn generate_rust(source: &str) -> String {
    let Ok(tokens) = lexer::lex(source) else {
        panic!("lexer failed");
    };
    let Ok(ast) = parser::parse(&tokens) else {
        panic!("parser failed");
    };
    let code = match codegen_with_builtin_stdlib_inventory().try_generate(&ast) {
        Ok(code) => code,
        Err(e) => panic!("codegen snapshot inputs must typecheck: {e:?}"),
    };
    normalize_codegen_output(&code)
}

/// Generate Rust with the same source-module and package identity context that the CLI supplies for registry code.
fn generate_registry_rust(source: &str, module_name: &str) -> String {
    let Ok(tokens) = lexer::lex(source) else {
        panic!("lexer failed");
    };
    let Ok(ast) = parser::parse(&tokens) else {
        panic!("parser failed");
    };
    let mut codegen = IrCodegen::new();
    codegen.set_root_source_module_name(Some(module_name.to_string()));
    codegen.set_registry_package_identity(Some(module_name.to_string()));
    let code = match codegen.try_generate(&ast) {
        Ok(code) => code,
        Err(error) => panic!("registry codegen snapshot inputs must typecheck: {error:?}"),
    };
    normalize_codegen_output(&code)
}

fn parse_incan_program(source: &str, context: &str) -> incan::frontend::ast::Program {
    let tokens = lexer::lex(source).unwrap_or_else(|errs| panic!("{context} lexer failed: {errs:?}"));
    parser::parse(&tokens).unwrap_or_else(|errs| panic!("{context} parser failed: {errs:?}"))
}

/// Generate Rust code from Incan source with a populated library index
fn generate_rust_with_widgets_manifest(source: &str) -> String {
    use incan::frontend::library_manifest_index::{
        LibraryArtifactMetadata, LibraryManifestIndex, LibraryManifestIndexEntry,
    };
    use incan::library_manifest::{
        ConstExport, FunctionExport, LibraryManifest, ModelExport, ParamExport, ParamKindExport, StaticExport, TypeRef,
    };
    use std::collections::HashMap;

    let Ok(tokens) = lexer::lex(source) else {
        panic!("lexer failed");
    };
    let Ok(ast) = parser::parse(&tokens) else {
        panic!("parser failed");
    };

    let mut artifact_root = std::env::temp_dir();
    artifact_root.push("incan_test_widgets_artifacts");
    artifact_root.push("target");
    artifact_root.push("lib");

    let mut manifest = LibraryManifest::new("widgets_core", "0.1.0");
    manifest.exports.models.push(ModelExport {
        name: "Widget".to_string(),
        type_params: Vec::new(),
        traits: Vec::new(),
        trait_adoptions: Vec::new(),
        derives: Vec::new(),
        fields: Vec::new(),
        properties: Vec::new(),
        methods: Vec::new(),
    });
    manifest.exports.functions.push(FunctionExport {
        name: "make_widget".to_string(),
        emitted_name: None,
        type_params: Vec::new(),
        params: vec![ParamExport {
            name: "name".to_string(),
            ty: TypeRef::Named {
                name: "str".to_string(),
            },
            kind: ParamKindExport::Normal,
            has_default: false,
            default: None,
        }],
        return_type: TypeRef::Named {
            name: "Widget".to_string(),
        },
        is_async: false,
    });
    manifest.exports.consts.push(ConstExport {
        name: "DEFAULT_NAME".to_string(),
        ty: TypeRef::Named {
            name: "str".to_string(),
        },
    });
    manifest.exports.statics.push(StaticExport {
        name: "SHARED_COUNT".to_string(),
        ty: TypeRef::Named {
            name: "int".to_string(),
        },
    });
    manifest.exports.statics.push(StaticExport {
        name: "SHARED_ITEMS".to_string(),
        ty: TypeRef::Applied {
            name: "list".to_string(),
            args: vec![TypeRef::Named {
                name: "int".to_string(),
            }],
        },
    });

    let index = LibraryManifestIndex::from_entries(HashMap::from([(
        "widgets".to_string(),
        LibraryManifestIndexEntry::Loaded {
            manifest: Box::new(manifest),
            metadata: LibraryArtifactMetadata::from_crate_root("widgets", "widgets_core", artifact_root),
        },
    )]));

    let mut codegen = codegen_with_builtin_stdlib_inventory();
    codegen.set_library_manifest_index(index);
    let code = match codegen.try_generate(&ast) {
        Ok(c) => c,
        Err(e) => panic!("codegen snapshot inputs must typecheck: {e:?}"),
    };
    normalize_codegen_output(&code)
}

#[cfg(feature = "rust_inspect")]
fn generate_rust_with_substrait_probe(source: &str) -> String {
    let tmp = match tempfile::tempdir() {
        Ok(tmp) => tmp,
        Err(err) => panic!("failed to create substrait probe tempdir: {err}"),
    };
    let root = tmp.path();
    if let Err(err) = fs::create_dir_all(root.join("src")) {
        panic!("failed to create probe src dir: {err}");
    }
    if let Err(err) = fs::create_dir_all(root.join("substrait").join("src")) {
        panic!("failed to create substrait src dir: {err}");
    }
    if let Err(err) = fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "ra_substrait_probe"
version = "0.1.0"
edition = "2021"

[dependencies]
substrait = { path = "substrait" }
"#,
    ) {
        panic!("failed to write probe Cargo.toml: {err}");
    }
    if let Err(err) = fs::write(
        root.join("src/lib.rs"),
        "pub fn touch() { let _ = substrait::proto::PlanRel; }\n",
    ) {
        panic!("failed to write probe lib.rs: {err}");
    }
    if let Err(err) = fs::write(
        root.join("substrait").join("Cargo.toml"),
        r#"[package]
name = "substrait"
version = "0.63.0"
edition = "2021"
"#,
    ) {
        panic!("failed to write substrait Cargo.toml: {err}");
    }
    if let Err(err) = fs::write(
        root.join("substrait").join("src/lib.rs"),
        r#"pub mod proto {
    pub struct PlanRel;

    pub struct Rel {
        pub rel_type: std::option::Option<rel::RelType>,
    }

    pub struct ReadRel;

    pub mod rel {
        pub enum RelType {
            Read(Box<super::ReadRel>),
        }
    }
}
"#,
    ) {
        panic!("failed to write substrait lib.rs: {err}");
    }

    let Ok(tokens) = lexer::lex(source) else {
        panic!("lexer failed");
    };
    let Ok(ast) = parser::parse(&tokens) else {
        panic!("parser failed");
    };
    let mut codegen = codegen_with_builtin_stdlib_inventory();
    codegen.set_rust_inspect_manifest_dir(root.to_path_buf());
    let code = match codegen.try_generate(&ast) {
        Ok(c) => c,
        Err(e) => panic!("codegen snapshot inputs must typecheck: {e:?}"),
    };
    normalize_codegen_output(&code)
}

/// Generate Rust from source that includes imported vocab blocks desugared via a WASM artifact.
fn generate_rust_with_vocab_wasm_desugaring(source: &str) -> String {
    use incan::frontend::library_manifest_index::{
        LibraryArtifactMetadata, LibraryManifestIndex, LibraryManifestIndexEntry,
    };
    use incan::frontend::vocab_desugar_pass::desugar_program_vocab_blocks;
    use incan::library_manifest::{LibraryManifest, VocabDesugarerArtifact, VocabExports};
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;

    let response = incan_vocab::DesugarResponse::statements(vec![incan_vocab::IncanStatement::Let {
        name: "generated".to_string(),
        mutable: false,
        value: incan_vocab::IncanExpr::Int(1),
    }]);
    let output_payload = match serde_json::to_string(&response) {
        Ok(payload) => payload,
        Err(err) => panic!("failed to serialize desugar response: {err}"),
    };
    let wat_bytes_string = |bytes: &[u8]| {
        let mut escaped = String::new();
        for byte in bytes {
            escaped.push('\\');
            escaped.push_str(&format!("{byte:02x}"));
        }
        escaped
    };
    let wat_i32_cell = |value: i32| wat_bytes_string(&value.to_le_bytes());

    let output_ptr_cell = 0usize;
    let output_len_cell = 4usize;
    let error_ptr_cell = 8usize;
    let error_len_cell = 12usize;
    let input_ptr_cell = 16usize;
    let input_capacity_cell = 20usize;
    let input_len_cell = 24usize;
    let output_offset = 128usize;
    let error_offset = 256usize;
    let input_offset = 384usize;
    let input_capacity = 4096usize;
    let wat_source = format!(
        r#"(module
  (memory (export "memory") 1)
  (global (export "__incan_input_ptr") i32 (i32.const {input_ptr_cell}))
  (global (export "__incan_input_capacity") i32 (i32.const {input_capacity_cell}))
  (global (export "__incan_input_len") i32 (i32.const {input_len_cell}))
  (global (export "__incan_output_ptr") i32 (i32.const {output_ptr_cell}))
  (global (export "__incan_output_len") i32 (i32.const {output_len_cell}))
  (global (export "__incan_error_ptr") i32 (i32.const {error_ptr_cell}))
  (global (export "__incan_error_len") i32 (i32.const {error_len_cell}))
  (data (i32.const {output_ptr_cell}) "{output_ptr_data}")
  (data (i32.const {output_len_cell}) "{output_len_data}")
  (data (i32.const {error_ptr_cell}) "{error_ptr_data}")
  (data (i32.const {error_len_cell}) "{error_len_data}")
  (data (i32.const {input_ptr_cell}) "{input_ptr_data}")
  (data (i32.const {input_capacity_cell}) "{input_capacity_data}")
  (data (i32.const {input_len_cell}) "{input_len_data}")
  (data (i32.const {output_offset}) "{out_data}")
  (func (export "__incan_init_desugarer"))
  (func (export "desugar_block") (result i32)
    (i32.const 0)
  )
)"#,
        input_ptr_cell = input_ptr_cell,
        input_capacity_cell = input_capacity_cell,
        input_len_cell = input_len_cell,
        output_ptr_cell = output_ptr_cell,
        output_len_cell = output_len_cell,
        error_ptr_cell = error_ptr_cell,
        error_len_cell = error_len_cell,
        output_ptr_data = wat_i32_cell(output_offset as i32),
        output_len_data = wat_i32_cell(output_payload.len() as i32),
        error_ptr_data = wat_i32_cell(error_offset as i32),
        error_len_data = wat_i32_cell(0),
        input_ptr_data = wat_i32_cell(input_offset as i32),
        input_capacity_data = wat_i32_cell(input_capacity as i32),
        input_len_data = wat_i32_cell(0),
        output_offset = output_offset,
        out_data = wat_bytes_string(output_payload.as_bytes()),
    );
    let wasm_bytes = match wat::parse_str(wat_source) {
        Ok(bytes) => bytes,
        Err(err) => panic!("failed to compile wat: {err}"),
    };

    let mut artifact_root = std::env::temp_dir();
    artifact_root.push("incan_test_vocab_desugar_artifacts");
    artifact_root.push("target");
    artifact_root.push("lib");
    let desugarer_dir = artifact_root.join("desugarers");
    if let Err(err) = std::fs::create_dir_all(&desugarer_dir) {
        panic!("failed to create desugarer artifact dir: {err}");
    }
    let desugarer_path = desugarer_dir.join("routes_desugarer.wasm");
    if let Err(err) = std::fs::write(&desugarer_path, &wasm_bytes) {
        panic!("failed to write desugarer artifact: {err}");
    }
    if let Err(err) = std::fs::create_dir_all(artifact_root.join("src")) {
        panic!("failed to create crate src dir: {err}");
    }
    if let Err(err) = std::fs::write(
        artifact_root.join("Cargo.toml"),
        "[package]\nname = \"routes_core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ) {
        panic!("failed to write Cargo.toml: {err}");
    }
    if let Err(err) = std::fs::write(artifact_root.join("src/lib.rs"), "pub fn ready() {}\n") {
        panic!("failed to write lib.rs: {err}");
    }

    let mut manifest = LibraryManifest::new("routes_core", "0.1.0");
    manifest.vocab = Some(VocabExports {
        crate_path: "vocab_companion".to_string(),
        package_name: "vocab_companion".to_string(),
        keyword_registrations: vec![incan_vocab::KeywordRegistration {
            activation: incan_vocab::KeywordActivation::OnImport {
                namespace: "routes.dsl".to_string(),
            },
            keywords: vec![incan_vocab::KeywordSpec {
                name: "route".to_string(),
                surface_kind: incan_vocab::KeywordSurfaceKind::BlockDeclaration,
                compound_tokens: Vec::new(),
                placement: incan_vocab::KeywordPlacement::TopLevel,
            }],
            valid_decorators: Vec::new(),
        }],
        dsl_surfaces: Vec::new(),
        provider_manifest: incan_vocab::LibraryManifest::default(),
        desugarer_artifact: Some(VocabDesugarerArtifact {
            artifact_kind: incan_vocab::DesugarerArtifactKind::WasmModule,
            abi_version: incan_vocab::WASM_DESUGAR_ABI_VERSION,
            relative_path: "desugarers/routes_desugarer.wasm".to_string(),
            target: "wasm32-wasip1".to_string(),
            profile: "release".to_string(),
            entrypoint: "desugar_block".to_string(),
            sha256: hex::encode(Sha256::digest(&wasm_bytes)),
        }),
    });

    let index = LibraryManifestIndex::from_entries(HashMap::from([(
        "routes".to_string(),
        LibraryManifestIndexEntry::Loaded {
            manifest: Box::new(manifest),
            metadata: LibraryArtifactMetadata::from_crate_root("routes", "routes_core", artifact_root),
        },
    )]));
    let imported_vocab = index.library_imported_vocab();

    let tokens = match lexer::lex(source) {
        Ok(tokens) => tokens,
        Err(errs) => panic!("lexer failed: {errs:?}"),
    };
    let mut ast = match parser::parse_with_context(
        &tokens,
        Some("tests/codegen_snapshots/vocab_block_desugaring.incn"),
        Some(&imported_vocab),
    ) {
        Ok(ast) => ast,
        Err(errs) => panic!("parser failed: {errs:?}"),
    };
    if let Err(errs) = desugar_program_vocab_blocks(
        &mut ast,
        Some("tests/codegen_snapshots/vocab_block_desugaring.incn"),
        &index,
    ) {
        panic!("desugar pass failed: {errs:?}");
    }

    let mut codegen = codegen_with_builtin_stdlib_inventory();
    codegen.set_library_manifest_index(index);
    let code = match codegen.try_generate(&ast) {
        Ok(code) => code,
        Err(err) => panic!("codegen failed: {err}"),
    };
    normalize_codegen_output(&code)
}

/// Generate Rust from source desugared through a helper-backed vocab WASM artifact.
fn generate_rust_with_helper_backed_vocab_wasm_desugaring(source: &str) -> String {
    use incan::frontend::library_manifest_index::{
        LibraryArtifactMetadata, LibraryManifestIndex, LibraryManifestIndexEntry,
    };
    use incan::frontend::vocab_desugar_pass::desugar_program_vocab_blocks;
    use incan::library_manifest::{
        FunctionExport, LibraryManifest, ParamExport, ParamKindExport, TypeRef, VocabDesugarerArtifact, VocabExports,
    };
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;

    let response = incan_vocab::DesugarResponse::expression(incan_vocab::IncanExpr::Call {
        callee: Box::new(incan_vocab::IncanExpr::Helper("filter".to_string())),
        args: vec![incan_vocab::IncanExpr::Int(1)],
    });
    let output_payload = match serde_json::to_string(&response) {
        Ok(payload) => payload,
        Err(err) => panic!("failed to serialize desugar response: {err}"),
    };
    let wat_bytes_string = |bytes: &[u8]| {
        let mut escaped = String::new();
        for byte in bytes {
            escaped.push('\\');
            escaped.push_str(&format!("{byte:02x}"));
        }
        escaped
    };
    let wat_i32_cell = |value: i32| wat_bytes_string(&value.to_le_bytes());

    let output_ptr_cell = 0usize;
    let output_len_cell = 4usize;
    let error_ptr_cell = 8usize;
    let error_len_cell = 12usize;
    let input_ptr_cell = 16usize;
    let input_capacity_cell = 20usize;
    let input_len_cell = 24usize;
    let output_offset = 128usize;
    let error_offset = 256usize;
    let input_offset = 384usize;
    let input_capacity = 4096usize;
    let wat_source = format!(
        r#"(module
  (memory (export "memory") 1)
  (global (export "__incan_input_ptr") i32 (i32.const {input_ptr_cell}))
  (global (export "__incan_input_capacity") i32 (i32.const {input_capacity_cell}))
  (global (export "__incan_input_len") i32 (i32.const {input_len_cell}))
  (global (export "__incan_output_ptr") i32 (i32.const {output_ptr_cell}))
  (global (export "__incan_output_len") i32 (i32.const {output_len_cell}))
  (global (export "__incan_error_ptr") i32 (i32.const {error_ptr_cell}))
  (global (export "__incan_error_len") i32 (i32.const {error_len_cell}))
  (data (i32.const {output_ptr_cell}) "{output_ptr_data}")
  (data (i32.const {output_len_cell}) "{output_len_data}")
  (data (i32.const {error_ptr_cell}) "{error_ptr_data}")
  (data (i32.const {error_len_cell}) "{error_len_data}")
  (data (i32.const {input_ptr_cell}) "{input_ptr_data}")
  (data (i32.const {input_capacity_cell}) "{input_capacity_data}")
  (data (i32.const {input_len_cell}) "{input_len_data}")
  (data (i32.const {output_offset}) "{out_data}")
  (func (export "__incan_init_desugarer"))
  (func (export "desugar_block") (result i32)
    (i32.const 0)
  )
)"#,
        input_ptr_cell = input_ptr_cell,
        input_capacity_cell = input_capacity_cell,
        input_len_cell = input_len_cell,
        output_ptr_cell = output_ptr_cell,
        output_len_cell = output_len_cell,
        error_ptr_cell = error_ptr_cell,
        error_len_cell = error_len_cell,
        output_ptr_data = wat_i32_cell(output_offset as i32),
        output_len_data = wat_i32_cell(output_payload.len() as i32),
        error_ptr_data = wat_i32_cell(error_offset as i32),
        error_len_data = wat_i32_cell(0),
        input_ptr_data = wat_i32_cell(input_offset as i32),
        input_capacity_data = wat_i32_cell(input_capacity as i32),
        input_len_data = wat_i32_cell(0),
        output_offset = output_offset,
        out_data = wat_bytes_string(output_payload.as_bytes()),
    );
    let wasm_bytes = match wat::parse_str(wat_source) {
        Ok(bytes) => bytes,
        Err(err) => panic!("failed to compile wat: {err}"),
    };

    let mut artifact_root = std::env::temp_dir();
    artifact_root.push("incan_test_vocab_helper_artifacts");
    artifact_root.push("target");
    artifact_root.push("lib");
    let desugarer_dir = artifact_root.join("desugarers");
    if let Err(err) = std::fs::create_dir_all(&desugarer_dir) {
        panic!("failed to create desugarer artifact dir: {err}");
    }
    let desugarer_path = desugarer_dir.join("query_desugarer.wasm");
    if let Err(err) = std::fs::write(&desugarer_path, &wasm_bytes) {
        panic!("failed to write desugarer artifact: {err}");
    }
    if let Err(err) = std::fs::create_dir_all(artifact_root.join("src")) {
        panic!("failed to create crate src dir: {err}");
    }
    if let Err(err) = std::fs::write(
        artifact_root.join("Cargo.toml"),
        "[package]\nname = \"query_core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ) {
        panic!("failed to write Cargo.toml: {err}");
    }
    if let Err(err) = std::fs::write(
        artifact_root.join("src/lib.rs"),
        "pub fn filter(value: i64) -> i64 { value }\n",
    ) {
        panic!("failed to write lib.rs: {err}");
    }

    let mut manifest = LibraryManifest::new("query_core", "0.1.0");
    manifest.exports.functions.push(FunctionExport {
        name: "filter".to_string(),
        emitted_name: None,
        type_params: Vec::new(),
        params: vec![ParamExport {
            name: "value".to_string(),
            ty: TypeRef::Named {
                name: "int".to_string(),
            },
            kind: ParamKindExport::Normal,
            has_default: false,
            default: None,
        }],
        return_type: TypeRef::Named {
            name: "int".to_string(),
        },
        is_async: false,
    });
    manifest.vocab = Some(VocabExports {
        crate_path: "vocab_companion".to_string(),
        package_name: "vocab_companion".to_string(),
        keyword_registrations: vec![incan_vocab::KeywordRegistration {
            activation: incan_vocab::KeywordActivation::OnImport {
                namespace: "query.dsl".to_string(),
            },
            keywords: vec![incan_vocab::KeywordSpec {
                name: "where".to_string(),
                surface_kind: incan_vocab::KeywordSurfaceKind::BlockDeclaration,
                compound_tokens: Vec::new(),
                placement: incan_vocab::KeywordPlacement::TopLevel,
            }],
            valid_decorators: Vec::new(),
        }],
        dsl_surfaces: Vec::new(),
        provider_manifest: incan_vocab::LibraryManifest {
            helper_bindings: vec![incan_vocab::HelperBinding {
                key: "filter".to_string(),
                exported_name: "filter".to_string(),
            }],
            ..incan_vocab::LibraryManifest::default()
        },
        desugarer_artifact: Some(VocabDesugarerArtifact {
            artifact_kind: incan_vocab::DesugarerArtifactKind::WasmModule,
            abi_version: incan_vocab::WASM_DESUGAR_ABI_VERSION,
            relative_path: "desugarers/query_desugarer.wasm".to_string(),
            target: "wasm32-wasip1".to_string(),
            profile: "release".to_string(),
            entrypoint: "desugar_block".to_string(),
            sha256: hex::encode(Sha256::digest(&wasm_bytes)),
        }),
    });

    let index = LibraryManifestIndex::from_entries(HashMap::from([(
        "query".to_string(),
        LibraryManifestIndexEntry::Loaded {
            manifest: Box::new(manifest),
            metadata: LibraryArtifactMetadata::from_crate_root("query", "query_core", artifact_root),
        },
    )]));
    let imported_vocab = index.library_imported_vocab();

    let tokens = match lexer::lex(source) {
        Ok(tokens) => tokens,
        Err(errs) => panic!("lexer failed: {errs:?}"),
    };
    let mut ast = match parser::parse_with_context(
        &tokens,
        Some("tests/codegen_snapshots/vocab_helper_backed_desugaring.incn"),
        Some(&imported_vocab),
    ) {
        Ok(ast) => ast,
        Err(errs) => panic!("parser failed: {errs:?}"),
    };
    if let Err(errs) = desugar_program_vocab_blocks(
        &mut ast,
        Some("tests/codegen_snapshots/vocab_helper_backed_desugaring.incn"),
        &index,
    ) {
        panic!("desugar pass failed: {errs:?}");
    }

    let mut codegen = codegen_with_builtin_stdlib_inventory();
    codegen.set_library_manifest_index(index);
    let code = match codegen.try_generate(&ast) {
        Ok(code) => code,
        Err(err) => panic!("codegen failed: {err}"),
    };
    normalize_codegen_output(&code)
}

/// Normalize generated output so snapshots don't churn on version bumps.
fn normalize_codegen_output(code: &str) -> String {
    let from = format!(
        "// Generated by the Incan compiler v{}\n\n",
        incan::version::INCAN_VERSION
    );
    let to = "// Generated by the Incan compiler v<INCAN_VERSION>\n\n";
    code.replace(&from, to)
        .lines()
        .map(|line| {
            if line.starts_with("incan_stdlib::__incan_stdlib_version_check!(") {
                "incan_stdlib::__incan_stdlib_version_check!(\"<INCAN_STDLIB_VERSION>\");"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Load a test file from the codegen_snapshots directory
fn load_test_file(name: &str) -> String {
    let path = format!("tests/codegen_snapshots/{}.incn", name);
    let Ok(content) = fs::read_to_string(&path) else {
        panic!("Failed to read test file: {}", path);
    };
    content
}

#[test]
fn test_pub_import_expressions_codegen() {
    let source = load_test_file("pub_import_expressions");
    let rust_code = generate_rust_with_widgets_manifest(&source);
    insta::assert_snapshot!("pub_import_expressions", rust_code);
}

#[test]
fn test_pub_import_module_alias_codegen() {
    let source = load_test_file("pub_import_module_alias");
    let rust_code = generate_rust_with_widgets_manifest(&source);
    insta::assert_snapshot!("pub_import_module_alias", rust_code);
}

#[test]
fn test_vocab_block_desugaring_codegen() {
    let source = load_test_file("vocab_block_desugaring");
    let rust_code = generate_rust_with_vocab_wasm_desugaring(&source);
    insta::assert_snapshot!("vocab_block_desugaring", rust_code);
}

#[test]
fn test_vocab_helper_backed_desugaring_codegen() {
    let source = "import pub::query\n\ndef main() -> None:\n  where true:\n    pass\n";
    let rust_code = generate_rust_with_helper_backed_vocab_wasm_desugaring(source);
    insta::assert_snapshot!("vocab_helper_backed_desugaring", rust_code);
}

#[test]
fn test_basic_function_codegen() {
    let source = load_test_file("basic_function");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("basic_function", rust_code);
}

#[test]
fn test_function_references_codegen() {
    let source = load_test_file("function_references");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("function_references", rust_code);
}

#[test]
fn test_user_defined_decorators_codegen() {
    let source = load_test_file("user_defined_decorators");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("user_defined_decorators", rust_code);
}

#[test]
fn test_decorated_variadic_function_codegen() {
    let source = load_test_file("decorated_variadic_function");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("decorated_variadic_function", rust_code);
}

#[test]
fn test_user_defined_method_decorators_codegen() {
    let source = load_test_file("user_defined_method_decorators");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("user_defined_method_decorators", rust_code);
}

#[test]
fn test_user_defined_mutable_method_decorators_codegen() {
    let source = load_test_file("user_defined_mutable_method_decorators");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("user_defined_mutable_method_decorators", rust_code);
}

#[test]
fn test_rfc070_result_combinators_codegen() {
    let source = r#"
def double(value: int) -> int:
  return value * 2

def keep_positive(value: int) -> Result[int, str]:
  if value > 0:
    return Ok(value)
  return Err("not positive")

def observe_int(_value: int) -> None:
  pass

from std.traits.callable import Callable1

model Observer with Callable1[int, None]:
  def __call__(self, value: int) -> None:
    pass

def main(result: Result[int, str]) -> Result[int, str]:
  observer = Observer()
  return result.map(double).and_then(keep_positive).inspect(observe_int).inspect(observer)
"#;
    let rust_code = generate_rust(source);
    assert!(
        rust_code.contains("crate::__incan_std::result::map(result, double)"),
        "map with a named function callback should dogfood the std.result helper:\n{rust_code}"
    );
    assert!(
        rust_code.contains("crate::__incan_std::result::and_then"),
        "and_then with a named function callback should dogfood the std.result helper:\n{rust_code}"
    );
    assert!(
        rust_code.contains("crate::__incan_std::result::inspect"),
        "inspect with a named function callback should dogfood the std.result helper:\n{rust_code}"
    );
    assert!(
        rust_code.contains("observe_int"),
        "inspect should pass Copy named observers through the std.result helper without cloning:\n{rust_code}"
    );
    assert!(
        rust_code.contains(".inspect(|__incan_result_value|"),
        "callable-object inspect should use Rust's borrowed Result observer surface:\n{rust_code}"
    );
    assert!(
        rust_code.contains("observer.__call__(*__incan_result_value)"),
        "callable objects should route through __call__ inside Result combinators:\n{rust_code}"
    );
    assert!(
        !rust_code.contains("clone()"),
        "Copy observer adaptation should not introduce clone calls:\n{rust_code}"
    );
}

#[test]
fn test_rfc070_result_unwrap_codegen_does_not_require_debug_err() {
    let source = r#"
model PlainError:
  message: str

pub def direct(result: Result[int, PlainError]) -> int:
  return result.unwrap()
"#;
    let rust_code = generate_rust(source);
    let compact = rust_code.split_whitespace().collect::<String>();
    assert!(
        compact.contains("matchresult{Ok(__incan_ok)=>__incan_ok,Err(_)=>panic!"),
        "Result.unwrap should lower to an explicit match that discards Err without a Debug bound:\n{rust_code}"
    );
    assert!(
        !compact.contains("result.unwrap()"),
        "Result.unwrap should not lower to Rust unwrap(), which requires E: Debug:\n{rust_code}"
    );
}

#[test]
fn test_rfc070_result_inspect_non_copy_observer_borrows_payload() {
    let source = r#"
model Payload:
  name: str

def observe_payload(_payload: Payload) -> None:
  pass

from std.traits.callable import Callable1

model PayloadObserver with Callable1[Payload, None]:
  def __call__(self, _payload: Payload) -> None:
    pass

pub def transform(result: Result[Payload, str]) -> Result[Payload, str]:
  return result.inspect(observe_payload)

pub def transform_with_observer(result: Result[Payload, str]) -> Result[Payload, str]:
  observer = PayloadObserver()
  return result.inspect(observer)
"#;
    let rust_code = generate_rust(source);
    assert!(
        rust_code.contains("fn __incan_borrow_adapter_observe_payload_0(_: &Payload)"),
        "non-Copy named observer callbacks should get a generated borrowed function adapter:\n{rust_code}"
    );
    assert!(
        rust_code.contains("crate::__incan_std::result::inspect(")
            && rust_code.contains("__incan_borrow_adapter_observe_payload_0"),
        "inspect should pass the borrowed adapter into the Incan-authored std.result helper:\n{rust_code}"
    );
    assert!(
        !rust_code.contains("__incan_result_observer_borrow_observe_payload"),
        "named function observers should use the generic borrowed adapter, not the old Result-specific helper:\n{rust_code}"
    );
    assert!(
        rust_code.contains("fn __incan_result_observer_borrow___call__(&self, _: &Payload)"),
        "non-Copy callable observers should get a generated borrowed __call__ helper:\n{rust_code}"
    );
    assert_eq!(
        rust_code.matches("fn __incan_result_observer_borrow___call__").count(),
        1,
        "callable-object borrowed observer helper should be emitted once:\n{rust_code}"
    );
    assert!(
        rust_code.contains("observer.__incan_result_observer_borrow___call__(__incan_result_value)"),
        "inspect should route non-Copy callable objects through the borrowed __call__ helper:\n{rust_code}"
    );
    assert!(
        !rust_code.contains("__incan_result_value).clone()"),
        "non-Copy inspect observers must not clone the payload:\n{rust_code}"
    );
}

#[test]
fn test_dict_operations_codegen() {
    let source = load_test_file("dict_operations");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("dict_operations", rust_code);
}

#[test]
fn test_model_struct_codegen() {
    let source = load_test_file("model_struct");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("model_struct", rust_code);
}

#[test]
fn test_uppercase_var_field_access_codegen() {
    let source = load_test_file("uppercase_var_field_access");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("uppercase_var_field_access", rust_code);
}

#[test]
fn test_model_with_alias_codegen() {
    let source = load_test_file("model_with_alias");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("model_with_alias", rust_code);
}

#[test]
fn test_model_with_serde_alias_codegen() {
    let source = load_test_file("model_with_serde_alias");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("model_with_serde_alias", rust_code);
}

#[test]
fn test_model_alias_expressions_codegen() {
    // RFC 021: Test alias-aware expression lowering (constructor, field access, patterns)
    let source = load_test_file("model_alias_expressions");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("model_alias_expressions", rust_code);
}

#[test]
fn test_model_alias_self_access_codegen() {
    // RFC 021: Ensure `self.<alias>` field access lowers to canonical field name
    let source = load_test_file("model_alias_self_access");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("model_alias_self_access", rust_code);
}

#[test]
fn test_web_route_extractors_codegen() {
    let source = load_test_file("web_route_extractors");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("web_route_extractors", rust_code);
}

#[test]
fn test_std_web_routing_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/web/routing.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    assert!(
        rust_code.contains("incan_stdlib::errors::__private::raise_runtime_misuse"),
        "proc-macro decorator runtime misuse should route through a named helper:\n{rust_code}"
    );
    assert!(
        !rust_code.contains("panic!(\"decorator marker"),
        "proc-macro decorator runtime misuse must not emit raw panic!:\n{rust_code}"
    );
    insta::assert_snapshot!("std_web_routing_compiled", rust_code);
}

#[test]
fn imported_stdlib_static_method_defaults_expand_at_call_site_issue500() {
    let source = r#"
from std.web import App

def main() -> None:
  App.run()
"#;
    let rust_code = generate_rust(source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("App::run(\"127.0.0.1\".to_string(),8080)"),
        "imported stdlib static method call should expand omitted defaults:\n{rust_code}"
    );
}

#[test]
fn imported_stdlib_associated_function_defaults_expand_in_generated_rust() {
    let source = r#"
from std.collections import OrdinalMapError

def main() -> None:
  error = OrdinalMapError.invalid_key_record("bad key")
  print(error.message())
"#;
    let tokens = lexer::lex(source).expect("fixture should lex");
    let ast = parser::parse(&tokens).expect("fixture should parse");
    let plan = incan::provider::ProviderPlan::default().with_bootstrap_sdk_namespace_roots(["collections".to_string()]);
    let mut codegen = IrCodegen::new();
    codegen.set_provider_plan(std::sync::Arc::new(plan));
    let rust_code = normalize_codegen_output(
        &codegen
            .try_generate(&ast)
            .expect("provider-bootstrap fixture should typecheck and lower"),
    );
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("OrdinalMapError::invalid_key_record(\"badkey\".to_string(),-1)"),
        "imported stdlib associated-function calls must expand omitted defaults:\n{rust_code}"
    );
}

#[test]
fn test_web_route_extractors_nested_module_codegen() {
    let main_source = r#"
import std.async
import api::routes

def main() -> None:
  pass
"#;
    let routes_source = r#"
import std.async
from std.web import route, POST

@route("/things", methods=[POST])
async def create(id: int) -> int:
  return id

@route("/search")
async def search(id: int) -> int:
  return id
"#;

    let Ok(main_tokens) = lexer::lex(main_source) else {
        panic!("lexer failed")
    };
    let Ok(main_ast) = parser::parse(&main_tokens) else {
        panic!("parser failed")
    };
    let Ok(routes_tokens) = lexer::lex(routes_source) else {
        panic!("lexer failed")
    };
    let Ok(routes_ast) = parser::parse(&routes_tokens) else {
        panic!("parser failed")
    };

    let mut codegen = codegen_with_builtin_stdlib_inventory();
    codegen.add_module_with_path_segments("api_routes", &routes_ast, vec!["api".to_string(), "routes".to_string()]);
    let Ok((main_code, _modules)) =
        codegen.try_generate_multi_file_nested(&main_ast, &[vec!["api".to_string(), "routes".to_string()]])
    else {
        panic!("codegen must succeed");
    };
    let rust_code = normalize_codegen_output(&main_code);
    insta::assert_snapshot!("web_route_extractors_nested_module", rust_code);
}

#[test]
fn test_web_route_private_nested_module_codegen() {
    let main_source = r#"
import std.async
import api::routes
from std.web import App

def main() -> None:
  App.run(host="127.0.0.1", port=0)
"#;
    let routes_source = r#"
import std.async
from std.web import route, Json
from std.serde import json

@derive(json)
model User:
  id: int
  name: str

@route("/users/{id}")
async def list_user(id: int) -> Json[User]:
  return Json(User(id=id, name="Ada"))
"#;

    let Ok(main_tokens) = lexer::lex(main_source) else {
        panic!("lexer failed")
    };
    let Ok(main_ast) = parser::parse(&main_tokens) else {
        panic!("parser failed")
    };
    let Ok(routes_tokens) = lexer::lex(routes_source) else {
        panic!("lexer failed")
    };
    let Ok(routes_ast) = parser::parse(&routes_tokens) else {
        panic!("parser failed")
    };

    let routes_path = vec!["api".to_string(), "routes".to_string()];
    let mut codegen = codegen_with_builtin_stdlib_inventory();
    codegen.set_preserve_dependency_public_items(false);
    codegen.add_module_with_path_segments("api_routes", &routes_ast, routes_path.clone());
    let Ok((main_code, modules)) =
        codegen.try_generate_multi_file_nested(&main_ast, std::slice::from_ref(&routes_path))
    else {
        panic!("codegen must succeed");
    };
    let Some(routes_code) = modules.get(&routes_path) else {
        panic!("routes module should be emitted");
    };
    let main_code = normalize_codegen_output(&main_code);
    let routes_code = normalize_codegen_output(routes_code);

    assert!(
        routes_code.contains("#[incan_web_macros::route(\"/users/{id}\")]"),
        "route proc-macro attribute should be retained in dependency module:\n{routes_code}"
    );
    assert!(
        routes_code.contains("struct User"),
        "private response model should be retained in dependency module:\n{routes_code}"
    );
    assert!(
        !routes_code.contains("pub struct User"),
        "route response model should not be forced public:\n{routes_code}"
    );
    assert!(
        routes_code.contains("async fn list_user"),
        "private route handler should be retained in dependency module:\n{routes_code}"
    );
    assert!(
        !routes_code.contains("pub async fn list_user"),
        "route handler should not be forced public:\n{routes_code}"
    );
    assert!(
        !main_code.contains("api::routes::list_user"),
        "main module should not call dependency route handler directly:\n{main_code}"
    );
}

#[test]
fn test_async_main_runtime_bootstrap_codegen() {
    let source = r#"
import std.async

async def main() -> None:
  println("hello")
"#;
    let rust_code = generate_rust(source);
    insta::assert_snapshot!("async_main_runtime_bootstrap", rust_code);
}

// ============================================================================
// RFC 022: Codegen emits incan_stdlib handoff, not framewor…18920 tokens truncated…le pure Incan
/// functions in the same module compile normally.
#[test]
fn test_rust_extern_delegation_codegen() {
    let source = load_test_file("rust_extern_delegation");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("rust_extern_delegation", rust_code);
}

/// RFC 023 Phase 5: compile the real `std.testing` module source.
#[test]
fn test_std_testing_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/testing.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_testing_compiled", rust_code);
}

/// RFC 041 / Phase E: compile `std.async.task` from `.incn` source.
#[test]
fn test_std_async_task_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/async/task.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_async_task_compiled", rust_code);
}

/// RFC 041 / Phase E: compile `std.async.time` from `.incn` source.
#[test]
fn test_std_async_time_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/async/time.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_async_time_compiled", rust_code);
}

/// Compile `std.async.channel` from `.incn` source.
#[test]
fn test_std_async_channel_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/async/channel.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_async_channel_compiled", rust_code);
}

/// Compile `std.async.sync` from `.incn` source.
#[test]
fn test_std_async_sync_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/async/sync.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_async_sync_compiled", rust_code);
}

/// Compile `std.async.race` from `.incn` source.
#[test]
fn test_std_async_race_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/async/race.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_async_race_compiled", rust_code);
}

/// Compile `race for value:` through the shared `std.async.race` runtime helper surface.
#[test]
fn test_race_for_expression_codegen() {
    let source = r#"
import std.async

pub async def fast() -> int:
  return 1

pub async def slow() -> int:
  return 2

pub async def fastest() -> int:
  return race for value:
    await fast() => value
    await slow() => value
"#;
    let rust_code = generate_rust(source);
    insta::assert_snapshot!("race_for_expression_codegen", rust_code);
}

/// Awaiting a declared wrapper must delegate to the proven awaitable field.
#[test]
fn test_awaitable_wrapper_delegation_codegen() {
    let source = r#"
import std.async
from std.async.task import JoinHandle, TaskJoinError

pub model TaskBox[T] with Awaitable[Result[T, TaskJoinError]]:
  pub handle: JoinHandle[T]

pub async def wait_for(box: TaskBox[int]) -> Result[int, TaskJoinError]:
  return await box
"#;
    let rust_code = generate_rust(source);
    assert!(
        rust_code.contains("r#box.handle.await"),
        "awaitable wrapper should lower through its awaitable field, got:\n{rust_code}"
    );
}

// ============================================================================
// RFC 023: Compile std.derives.* trait definitions from Incan source
// ============================================================================

/// compile `std.derives.comparison` (Eq, Ord, Hash) from `.incn` source.
///
/// Verifies that source-defined abstract methods and pure-Incan default methods (`__ne__`, `__le__`, `__gt__`,
/// `__ge__`) compile through the full pipeline without a fake `rust.module()` boundary.
#[test]
fn test_std_derives_comparison_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/derives/comparison.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_derives_comparison_compiled", rust_code);
}

/// compile `std.derives.copying` (Clone, Copy, Default) from `.incn` source.
#[test]
fn test_std_derives_copying_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/derives/copying.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_derives_copying_compiled", rust_code);
}

/// compile `std.derives.string` (Debug, Display) from `.incn` source.
#[test]
fn test_std_derives_string_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/derives/string.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_derives_string_compiled", rust_code);
}

/// compile `std.derives.collection` (collection/iterator protocols and adapters) from `.incn` source.
#[test]
fn test_std_derives_collection_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/derives/collection.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    assert!(
        rust_code.contains("StoredMapFn: Clone + Callable1<T, Output>"),
        "fallible adapter storage must retain the nominal source callable bound:\n{rust_code}"
    );
    assert!(
        rust_code.contains("fn filter<Predicate: Clone + Callable1<Output, bool>>"),
        "expanded fallible defaults must substitute the adapter's adopted item type:\n{rust_code}"
    );
    assert!(
        !rust_code.contains("list<"),
        "collection types inside callable bounds must use their canonical Rust representation:\n{rust_code}"
    );
    insta::assert_snapshot!("std_derives_collection_compiled", rust_code);
}

/// RFC 023: compile `std.serde.json` (Serialize, Deserialize) from `.incn` source.
///
/// Verifies that trait declarations with `@rust.extern` methods compile through the full pipeline when serde namespace
/// is in IncanSource mode.
#[test]
fn test_std_serde_json_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/serde/json.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_serde_json_compiled", rust_code);
}

/// RFC 024: verify `@derive(json)` resolves through stdlib derive metadata and compiles.
///
/// Exercises the stdlib import path for the json module-level derive.
#[test]
fn test_std_serde_json_import_codegen() {
    let source = load_test_file("std_serde_json_import");
    let rust_code = generate_rust(&source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("incan_stdlib::json::__private::stringify_or_raise"),
        "expected JSON stringify emission to route through stdlib helper; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("incan_stdlib::json::__private::parse_or_error"),
        "expected JSON decode emission to route through stdlib helper; generated:\n{rust_code}"
    );
    assert!(
        !compact.contains("serde_json::to_string"),
        "generated JSON stringify paths should no longer inline serde_json::to_string fallbacks; generated:\n{rust_code}"
    );
    assert!(
        !compact.contains("serde_json::from_str"),
        "generated JSON decode paths should no longer inline serde_json::from_str fallbacks; generated:\n{rust_code}"
    );
    insta::assert_snapshot!("std_serde_json_import", rust_code);
}

/// RFC 113: typed declaration registries remain source-authored while the compiler records `@describe` facts.
#[test]
fn test_std_registry_import_codegen() {
    let source = load_test_file("std_registry_import");
    let rust_code = generate_registry_rust(&source, "std_registry_import");
    insta::assert_snapshot!("std_registry_import", rust_code);
}

/// RFC 113: method descriptions lower into source-owned registry runtime registration.
#[test]
fn test_std_registry_methods_codegen() {
    let source = load_test_file("std_registry_methods");
    let rust_code = generate_registry_rust(&source, "std_registry_methods");
    insta::assert_snapshot!("std_registry_methods", rust_code);
}

/// RFC 113: explicit compilation-unit and package entries retain compiler-checked canonical subjects.
#[test]
fn test_std_registry_subjects_codegen() {
    let source = load_test_file("std_registry_subjects");
    let rust_code = generate_registry_rust(&source, "std_registry_subjects");
    insta::assert_snapshot!("std_registry_subjects", rust_code);
}

/// RFC 113: a structural descriptor can retain a concrete Incan type token without changing registry lowering.
#[test]
fn test_std_registry_type_token_codegen() {
    let source = load_test_file("std_registry_type_token");
    let rust_code = generate_registry_rust(&source, "std_registry_type_token");
    insta::assert_snapshot!("std_registry_type_token", rust_code);
}

/// RFC 047: compile `std.graph` declarations from `.incn` source.
#[test]
fn test_std_graph_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/graph.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_graph_compiled", rust_code);
}

/// RFC 061: compile the `std.compression` source modules.
#[test]
fn test_std_compression_modules_compile_codegen() -> Result<(), Box<dyn std::error::Error>> {
    let paths = [
        "crates/incan_stdlib/stdlib/compression/prelude.incn",
        "crates/incan_stdlib/stdlib/compression/_core.incn",
        "crates/incan_stdlib/stdlib/compression/_auto.incn",
        "crates/incan_stdlib/stdlib/compression/gzip.incn",
        "crates/incan_stdlib/stdlib/compression/zlib.incn",
        "crates/incan_stdlib/stdlib/compression/deflate.incn",
        "crates/incan_stdlib/stdlib/compression/zstd.incn",
        "crates/incan_stdlib/stdlib/compression/bz2.incn",
        "crates/incan_stdlib/stdlib/compression/lzma.incn",
        "crates/incan_stdlib/stdlib/compression/snappy.incn",
        "crates/incan_stdlib/stdlib/compression/snappy/raw.incn",
    ];

    for path in paths {
        let source = fs::read_to_string(path)?;
        let rust_code = generate_rust(&source);
        assert!(
            rust_code.contains("__incan"),
            "expected {path} to compile into Incan-generated Rust, got:\n{rust_code}"
        );
    }
    Ok(())
}

/// RFC 047: verify `std.graph` imports, direct constructors, DAGs, and multigraph edge ids lower to Rust.
#[test]
fn test_std_graph_import_codegen() {
    let source = load_test_file("std_graph_import");
    let rust_code = generate_rust(&source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("DiGraph::<String>::__incan_new()"),
        "expected DiGraph constructor syntax to lower through __incan_new; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("Dag::<String>::__incan_new()"),
        "expected Dag constructor syntax to lower through __incan_new; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("MultiDiGraph::<String>::__incan_new()"),
        "expected MultiDiGraph constructor syntax to lower through __incan_new; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("Result<EdgeId,GraphError>"),
        "expected multigraph add_edge to preserve EdgeId result; generated:\n{rust_code}"
    );
    insta::assert_snapshot!("std_graph_import", rust_code);
}

/// RFC 060: compile `std.uuid` declarations from `.incn` source.
#[test]
fn test_std_uuid_compiled_codegen() -> Result<(), Box<dyn std::error::Error>> {
    let path = "crates/incan_stdlib/stdlib/uuid.incn";
    let source = fs::read_to_string(path)?;
    let rust_code = generate_rust(&source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("pubstructUUID(pubu128);"),
        "expected UUID to remain a source-defined u128 newtype; generated:\n{rust_code}"
    );
    assert!(
        !rust_code.contains("uuid::Uuid::") && !rust_code.contains("uuid::Uuid;"),
        "std.uuid must not lower to a Rust uuid::Uuid-backed type; generated:\n{rust_code}"
    );
    insta::assert_snapshot!("std_uuid_compiled", rust_code);
    Ok(())
}

/// RFC 060: verify `std.uuid` imports and method calls lower without a Rust-backed UUID type.
#[test]
fn test_std_uuid_import_codegen() {
    let source = load_test_file("std_uuid_import");
    let rust_code = generate_rust(&source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("UUID::parse"),
        "expected parse call to route through the source-defined UUID type; generated:\n{rust_code}"
    );
    assert!(
        !rust_code.contains("uuid::Uuid::") && !rust_code.contains("uuid::Uuid;"),
        "std.uuid import path must not introduce a Rust uuid::Uuid-backed type; generated:\n{rust_code}"
    );
    insta::assert_snapshot!("std_uuid_import", rust_code);
}

/// RFC 059: direct imported constructors lower through the generic `__incan_new` hook.
#[test]
fn test_std_regex_import_constructor_hook_codegen() {
    let source = r#"
from std.regex import Regex, RegexError

def main() -> Result[None, RegexError]:
  _regex = Regex("x+", ignore_case=true)?
  return Ok(None)
"#;
    let rust_code = generate_rust(source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("Regex::__incan_new(\"x+\".to_string(),true,false,false,false)?"),
        "expected imported Regex constructor syntax to lower through the generic __incan_new hook; generated:\n{rust_code}"
    );
    assert!(
        !compact.contains("__incan_std::regex::compile("),
        "direct constructor lowering should not hardcode std.regex.compile; generated:\n{rust_code}"
    );
}

/// RFC 023 (#303): explicit `with Serialize` adoption should expand the stdlib default `to_json` body into the
/// generated impl while also forwarding the Rust serde derive.
#[test]
fn test_std_serde_with_serialize_trait_codegen() {
    let source = load_test_file("std_serde_with_serialize_trait");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_serde_with_serialize_trait", rust_code);
}

#[test]
fn test_newtype_with_serialize_trait_forwards_rust_derive() {
    let source = r#"
from std.serde.json import Serialize

type Wrapped = newtype str with Serialize

def main() -> None:
  println(Wrapped("ok").to_json())
"#;
    let rust_code = generate_rust(source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("#[derive(Debug,serde::Serialize)]structWrapped(pubString);")
            || compact.contains("#[derive(Debug,Clone,serde::Serialize)]structWrapped(pubString);"),
        "expected newtype `with Serialize` to forward the Rust serde derive; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("implSerializeforWrapped"),
        "expected newtype `with Serialize` to expand the stdlib Serialize impl; generated:\n{rust_code}"
    );
}

#[test]
fn test_with_serialize_keeps_ordinary_methods_inherent() {
    let source = r#"
from std.serde.json import Serialize

model Payload with Serialize:
  value: str

  def display_text(self) -> str:
    return self.value

def main() -> None:
  payload = Payload(value="ok")
  println(payload.display_text())
  println(payload.to_json())
"#;
    let rust_code = generate_rust(source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("implPayload{pubfndisplay_text(&self)->String"),
        "expected ordinary method on `with Serialize` model to emit as inherent impl; generated:\n{rust_code}"
    );
    assert!(
        !compact.contains("implSerializeforPayload{fndisplay_text"),
        "ordinary methods must not be emitted into the Serialize trait impl; generated:\n{rust_code}"
    );
}

#[test]
fn test_qualified_source_trait_dispatch_does_not_double_borrow_self() {
    let source = r#"
from std.serde import json

@derive(json)
model Payload:
  value: int

  def encode(self) -> str:
    return self.to_json()

def main() -> str:
  return Payload(value=1).encode()
"#;
    let rust_code = generate_rust(source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("Serialize::to_json(self)"),
        "expected qualified source-trait dispatch to reuse the method's borrowed self receiver; generated:\n{rust_code}"
    );
    assert!(
        !compact.contains("Serialize::to_json(&self)"),
        "qualified source-trait dispatch must not borrow an already borrowed self receiver; generated:\n{rust_code}"
    );
}

#[test]
fn test_direct_json_trait_import_keeps_canonical_owner_across_modules_issue946() {
    let models_source = r#"
from std.serde import json

@derive(json)
pub model Item:
  pub value: str
"#;
    let encode_source = r#"
from std.serde.json import Serialize
from crate.models import Item

pub def encode(item: Item) -> str:
  return item.to_json()
"#;
    let root_source = r#"
from crate.encode import encode
from crate.models import Item
"#;
    let models_ast = parse_incan_program(models_source, "JSON model module");
    let encode_ast = parse_incan_program(encode_source, "JSON encoder module");
    let root_ast = parse_incan_program(root_source, "JSON library root");
    let mut codegen = codegen_with_builtin_stdlib_inventory();
    codegen.add_module_with_path_segments("models", &models_ast, vec!["models".to_string()]);
    codegen.add_module_with_path_segments("encode", &encode_ast, vec!["encode".to_string()]);
    let (_root_code, modules) = codegen
        .try_generate_multi_file_nested(&root_ast, &[vec!["models".to_string()], vec!["encode".to_string()]])
        .unwrap_or_else(|err| panic!("multi-module JSON library should codegen: {err:?}"));
    let Some(encode_module) = modules.get(&vec!["encode".to_string()]) else {
        panic!("missing generated encoder module");
    };
    let rust_code = normalize_codegen_output(encode_module);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();

    assert!(
        compact.contains("crate::__incan_std::serde::json::Serialize::to_json(&item)"),
        "directly imported source traits must preserve their canonical owner in each generated module:\n{rust_code}"
    );
    assert!(
        !compact.contains("returnjson::Serialize::to_json(&item)"),
        "the encoder module does not import the source `json` module:\n{rust_code}"
    );
}

/// RFC 024: module-level derive metadata should let `@derive(json)` adopt serde traits and emit Rust derives.
#[test]
fn test_rfc024_module_derive_json_codegen() {
    let source = load_test_file("rfc024_module_derive_json");
    let rust_code = generate_rust(&source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("serde::Serialize,serde::Deserialize"),
        "expected @derive(json) to forward serde derives; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("impljson::SerializeforPayload{fnto_json(&self)->String"),
        "expected @derive(json) to emit the adopted json.Serialize trait impl with its serde adapter; generated:\n{rust_code}"
    );
    insta::assert_snapshot!("rfc024_module_derive_json", rust_code);
}

/// RFC 024: imported trait aliases should work as partial derives.
#[test]
fn test_rfc024_partial_alias_derive_codegen() {
    let source = load_test_file("rfc024_partial_alias_derive");
    let rust_code = generate_rust(&source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("implJsonSerializeforPayload{fnto_json(&self)->String"),
        "expected @derive(JsonSerialize) to emit the adopted aliased trait impl with its serde adapter; generated:\n{rust_code}"
    );
    insta::assert_snapshot!("rfc024_partial_alias_derive", rust_code);
}

/// RFC 024: user modules can define a second serde-backed format without compiler changes.
#[test]
fn test_rfc024_user_module_serde_format_codegen() {
    let yaml_source = r#"
__derives__ = [Serialize]

@rust.derive("serde::Serialize")
pub trait Serialize:
  def to_yaml(self) -> str:
    return str("yaml")
"#;
    let source = r#"
from std.serde import json
import yaml

@derive(json, yaml)
model Payload:
  value: int

def encode_yaml[T with yaml.Serialize](value: T) -> str:
  return value.to_yaml()

def encode_json[T with json.Serialize](value: T) -> str:
  return value.to_json()

def main() -> str:
  return encode_yaml(Payload(value=1))
"#;

    let yaml_ast = parse_incan_program(yaml_source, "yaml module");
    let main_ast = parse_incan_program(source, "consumer");
    let mut codegen = codegen_with_builtin_stdlib_inventory();
    codegen.add_module_with_path_segments("yaml", &yaml_ast, vec!["yaml".to_string()]);
    let (main_code, _modules) = codegen
        .try_generate_multi_file_nested(&main_ast, &[vec!["yaml".to_string()]])
        .unwrap_or_else(|err| panic!("user serde derivable module should codegen: {err:?}"));
    let rust_code = normalize_codegen_output(&main_code);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert_eq!(
        rust_code.matches("serde::Serialize").count(),
        1,
        "expected duplicate serde derive paths to be deduplicated; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("impljson::SerializeforPayload"),
        "expected stdlib json.Serialize impl; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("implyaml::SerializeforPayload"),
        "expected user yaml.Serialize impl; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("fnto_yaml(&self)->String"),
        "expected yaml default method body to expand into the impl; generated:\n{rust_code}"
    );
}

/// RFC 024: derivable modules are not limited to serde-backed Rust derives.
#[test]
fn test_rfc024_user_module_pure_incan_derivable_codegen() {
    let schema_source = r#"
__derives__ = [Named]

pub trait Named:
  def schema_name(self) -> str:
    return str("schema")
"#;
    let source = r#"
import schema

@derive(schema)
model Payload:
  value: int

def name[T with schema.Named](value: T) -> str:
  return value.schema_name()

def main() -> str:
  return name(Payload(value=1))
"#;

    let schema_ast = parse_incan_program(schema_source, "schema module");
    let main_ast = parse_incan_program(source, "consumer");
    let mut codegen = codegen_with_builtin_stdlib_inventory();
    codegen.add_module_with_path_segments("schema", &schema_ast, vec!["schema".to_string()]);
    let (main_code, _modules) = codegen
        .try_generate_multi_file_nested(&main_ast, &[vec!["schema".to_string()]])
        .unwrap_or_else(|err| panic!("pure Incan derivable module should codegen: {err:?}"));
    let rust_code = normalize_codegen_output(&main_code);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("implschema::NamedforPayload"),
        "expected user schema.Named impl; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("fnschema_name(&self)->String"),
        "expected pure Incan default method body to expand into the impl; generated:\n{rust_code}"
    );
    assert!(
        !rust_code.contains("serde::Serialize"),
        "pure derivable fixture should not emit serde derives; generated:\n{rust_code}"
    );
}

#[test]
fn test_multi_instantiation_trait_methods_codegen_trait_impls_only() {
    let source = r#"
trait Convert[T]:
  def convert(self) -> T: ...

model Reading with Convert[int], Convert[float]:
  value: int

  def convert(self) -> int:
    return self.value

  def convert(self) -> float:
    return 1.0

def main() -> None:
  reading = Reading(value=1)
  precise: float = reading.convert()
"#;
    let rust_code = generate_rust(source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("implConvert<i64>forReading"),
        "expected Convert[int] trait impl; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("implConvert<f64>forReading"),
        "expected Convert[float] trait impl; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("let_precise:f64=reading.convert();"),
        "typed local binding must preserve the Rust return hint for same-family trait impl dispatch; generated:\n{rust_code}"
    );
    assert!(
        !compact.contains("implReading{fnconvert"),
        "same-name trait methods must not also lower as duplicate inherent methods; generated:\n{rust_code}"
    );
}

#[test]
fn test_std_json_value_indexing_emits_checked_helpers() {
    let source = r#"
from std.json import JsonValue

pub def by_name(data: JsonValue) -> Option[JsonValue]:
  return data["name"]

pub def by_index(data: JsonValue) -> Option[JsonValue]:
  return data[0]
"#;
    let rust_code = generate_rust(source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains(
            "crate::__incan_std::traits::indexing::Index::<String,Option<JsonValue>,>::__getitem__(&data,\"name\".to_string())"
        ),
        "expected object-style JsonValue indexing to use source-authored Index.__getitem__; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("crate::__incan_std::traits::indexing::Index::<i64,Option<JsonValue>,>::__getitem__(&data,0)"),
        "expected array-style JsonValue indexing to use source-authored Index.__getitem__; generated:\n{rust_code}"
    );
}

/// Issue #815: explicit `Index[K, V]` adoption on a generic carrier must produce a Rust trait impl.
#[test]
fn test_issue815_generic_index_adoption_emits_trait_impl() {
    let source = r#"
from std.traits.indexing import Index

class GenericBox[T with Clone] with Index[str, str]:
  pub label: str
  pub witness: list[T]

  def __getitem__(self, key: str) for Index[str, str] -> str:
    return key

pub def indexed_label[T with Clone](box: GenericBox[T]) -> str:
  return box["amount"]
"#;
    let rust_code = generate_rust(source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("impl<T:Clone>Index<String,String>forGenericBox<T>"),
        "generic Index adoption must emit a parameterized Rust trait impl; generated:\n{rust_code}"
    );
    assert!(
        compact.contains(
            "crate::__incan_std::traits::indexing::Index::<String,String,>::__getitem__(&r#box,\"amount\".to_string())"
        ),
        "generic index access must dispatch through the adopted Index implementation; generated:\n{rust_code}"
    );
}

/// Issue #815: `Self` in an adopted Index output must become the concrete carrier in an impl header and call site.
#[test]
fn test_issue815_self_returning_index_adoption_uses_owner_type() {
    let source = r#"
from std.traits.indexing import Index

class PlainBox with Index[list[str], Self]:
  pub label: str

  def __getitem__(self, key: list[str]) for Index[list[str], Self] -> Self:
    return self

pub def nested_box(box: PlainBox) -> PlainBox:
  return box[["name"]]
"#;
    let rust_code = generate_rust(source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("implIndex<Vec<String>,PlainBox>forPlainBox"),
        "Self in an adopted Index target must emit as the owner type; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("Index::<Vec<String>,PlainBox,>::__getitem__(&r#box,vec![\"name\".to_string()])"),
        "Self-returning index access must use the concrete Index instantiation; generated:\n{rust_code}"
    );
}

#[test]
fn test_enum_multi_instantiation_trait_methods_codegen_trait_impls_only() {
    let source = r#"
trait Convert[T]:
  def convert(self) -> T: ...

enum Token with Convert[int], Convert[float]:
  Number

  def convert(self) -> int:
    return 1

  def convert(self) -> float:
    return 1.0

def main() -> None:
  token: Token = Token.Number
  precise: float = token.convert()
"#;
    let rust_code = generate_rust(source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("implConvert<i64>forToken"),
        "expected Convert[int] enum trait impl; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("implConvert<f64>forToken"),
        "expected Convert[float] enum trait impl; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("let_precise:f64=token.convert();"),
        "typed enum local binding must preserve the Rust return hint for same-family trait impl dispatch; generated:\n{rust_code}"
    );
    assert!(
        !compact.contains("implToken{pubfnconvert") && !compact.contains("implToken{fnconvert"),
        "same-name enum trait methods must not also lower as duplicate inherent methods; generated:\n{rust_code}"
    );
}

// ============================================================================
// RFC 023: Compile std.traits.* trait definitions from Incan source
// ============================================================================

#[test]
fn test_std_traits_ops_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/traits/ops.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_traits_ops_compiled", rust_code);
}

#[test]
fn test_std_traits_error_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/traits/error.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_traits_error_compiled", rust_code);
}

#[test]
fn test_std_traits_indexing_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/traits/indexing.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_traits_indexing_compiled", rust_code);
}

#[test]
fn test_std_traits_callable_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/traits/callable.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_traits_callable_compiled", rust_code);
}

#[test]
fn test_std_traits_prelude_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/traits/prelude.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_traits_prelude_compiled", rust_code);
}

#[test]
fn test_std_traits_convert_compiled_codegen() {
    let path = "crates/incan_stdlib/stdlib/traits/convert.incn";
    let Ok(source) = fs::read_to_string(path) else {
        panic!("Failed to read stdlib source file: {}", path);
    };
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_traits_convert_compiled", rust_code);
}

#[test]
fn test_std_traits_convert_usage_codegen() {
    let source = load_test_file("std_traits_convert_usage");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("std_traits_convert_usage", rust_code);
}

// ============================================================================
/// Issue #145: Full surface-semantics path for `assert` statements.
// ============================================================================
///
/// Exercises: parser `Statement::Surface` -> typechecker -> lowering to `IrExprKind::Call` with `canonical_path` ->
/// emission via `emit_canonical_callee_path()`.
#[test]
fn test_assert_surface_codegen() {
    let source = load_test_file("assert_surface");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("assert_surface", rust_code);
}

// ============================================================================
/// RFC 057: Targeted Rust lint suppression.
// ============================================================================
#[test]
fn test_rust_allow_codegen() {
    let source = load_test_file("rust_allow");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("rust_allow", rust_code);
}

// ============================================================================
// RFC 023: Trait Bound Inference and `with` Annotation
// ============================================================================

/// RFC 023: Inferred trait bounds from usage (`==`/`!=` -> PartialEq, f-string -> Display, etc.)
#[test]
fn test_trait_bound_inference_codegen() {
    let source = load_test_file("trait_bound_inference");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("trait_bound_inference", rust_code);
}

/// RFC 023: Explicit `with` bounds on type parameters.
#[test]
fn test_trait_bound_explicit_codegen() {
    let source = load_test_file("trait_bound_explicit");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("trait_bound_explicit", rust_code);
}

#[test]
fn test_ordinal_key_builtin_impls_codegen() {
    let source = load_test_file("ordinal_key_builtin_impls");
    let rust_code = generate_rust(&source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("pubusecrate::__incan_std::collections::OrdinalKey;"),
        "expected imported std.collections.OrdinalKey re-export; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("implcrate::__incan_std::collections::OrdinalKeyforStatus{")
            && compact
                .contains("fnordinal_hash(&self)->i64{incan_stdlib::collections::__private::ordinal_key_hash_bytes")
            && compact
                .contains("fnordinal_bytes_equal(&self,data:Vec<u8>)->bool{self.value().as_bytes()==data.as_slice()}"),
        "expected generated OrdinalKey impl for string value enum; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("implcrate::__incan_std::collections::OrdinalKeyforHttpStatus{")
            && compact.contains(
                "fnordinal_hash(&self)->i64{incan_stdlib::collections::__private::ordinal_key_hash_bytes"
            )
            && compact.contains("fnordinal_bytes_equal(&self,data:Vec<u8>)->bool{data.as_slice()==self.value().to_le_bytes().as_slice()}"),
        "expected generated OrdinalKey impl for integer value enum; generated:\n{rust_code}"
    );
    insta::assert_snapshot!("ordinal_key_builtin_impls", rust_code);
}

#[test]
fn test_ordinal_map_str_fast_lookup_codegen() {
    let source = load_test_file("ordinal_map_str_fast_lookup");
    let rust_code = generate_rust(&source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("columns.require(key)"),
        "expected OrdinalMap[str].require to keep the source-defined lookup path; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("columns.require_many(keys)"),
        "expected OrdinalMap[str].require_many to keep the source-defined batch lookup path; generated:\n{rust_code}"
    );
    assert!(
        !compact.contains("__incan_require_str_fast") && !compact.contains("__incan_require_many_str_fast"),
        "OrdinalMap[str] calls should not route through generated method specializations; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("vec![\"id\".to_string(),\"status\".to_string()]"),
        "expected direct string-list construction to materialize owned strings; generated:\n{rust_code}"
    );
    assert!(
        compact.contains("vec![(\"id\".to_string(),10),(\"status\".to_string(),20)]"),
        "expected direct string-pair construction to materialize owned strings; generated:\n{rust_code}"
    );
    insta::assert_snapshot!("ordinal_map_str_fast_lookup", rust_code);
}

#[test]
fn test_imported_stdlib_value_fragment_codegen() {
    let source = load_test_file("imported_stdlib_value_fragment");
    let rust_code = generate_rust(&source);
    let compact = rust_code.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    assert!(
        compact.contains("::ordinal_key_byte(7);"),
        "expected imported stdlib value fragment helper to be called directly; generated:\n{rust_code}"
    );
    assert!(
        !compact.contains("ordinal_key_append_byte"),
        "stale datetime ordinal append helper leaked into generated code; generated:\n{rust_code}"
    );
    insta::assert_snapshot!("imported_stdlib_value_fragment", rust_code);
}

/// RFC 023: Additional inference cases (Display, Dict key hashing, arithmetic, transitive propagation).
#[test]
fn test_trait_bound_inference_more_codegen() {
    let source = load_test_file("trait_bound_inference_more");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("trait_bound_inference_more", rust_code);
}

#[test]
fn test_loop_expressions_codegen() {
    let source = load_test_file("loop_expressions");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("loop_expressions", rust_code);
}

/// RFC 023: Generic bounds in return types (issue #196).
///
/// Verifies that trait bounds from return types (e.g., `impl BoundedDataSet<T>`) are properly inferred and emitted in
/// the Rust codegen, even when the bounds aren't used in the function body.
#[test]
fn test_generic_bounds_return_type_codegen() {
    let source = load_test_file("generic_bounds_return_type");
    let rust_code = generate_rust(&source);
    insta::assert_snapshot!("generic_bounds_return_type", rust_code);
}

// Glob-based test that auto-discovers all .incn files
// To enable: uncomment the test below and run `cargo test --test codegen_snapshot_tests`
//
// #[test]
// fn test_all_codegen_snapshots() {
//     insta::glob!("codegen_snapshots/*.incn", |path| {
//         let source = fs::read_to_string(path).expect("failed to read file");
//         let rust_code = generate_rust(&source);
//         let name = path.file_stem().unwrap().to_string_lossy();
//         insta::assert_snapshot!(name.to_string(), rust_code);
//     });
// }
