//! Compiler-backed checked C binding inspection.
//!
//! `incan inspect bindings` projects C declaration facts from the checked typechecker descriptor. The shared checked
//! analysis runs the ordinary host-target C verifier, but this report is not a reusable verification receipt and does
//! not resolve native artifacts or infer bridge and facade roles from source layout.

use std::env;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use incan_core::lang::c_abi::scalar_type_as_str;
use serde::Serialize;

use crate::cli::prelude::ParsedModule;
use crate::cli::{CliError, CliResult, ExitCode};
use crate::frontend::ast::Span;
use crate::frontend::typechecker::{CBindingDescriptor, CBindingType};
use crate::provider::FeatureSelection;

use super::common::{CompilationAnalysis, CompilationSession, collect_modules_detailed_with_session};

/// Compatibility version for the checked C binding inspection report.
const BINDING_INSPECTION_SCHEMA_VERSION: u32 = 1;

/// Output format for `incan inspect bindings`.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingInspectionFormat {
    /// Concise declaration summary for terminal use.
    Text,
    /// Deterministic structured declaration facts for tools.
    Json,
}

/// Stable checked C binding declaration report.
#[derive(Debug, Serialize)]
struct BindingInspectionReport {
    schema_version: u32,
    bindings: Vec<BindingReport>,
}

/// One C binding declaration as retained by successful typechecking.
#[derive(Debug, Serialize)]
struct BindingReport {
    module: Vec<String>,
    name: String,
    header: String,
    system_library: String,
    source: BindingSourceSpan,
    symbols: Vec<SymbolReport>,
    enums: Vec<EnumReport>,
    structs: Vec<StructReport>,
}

/// One declaration-level C symbol contract.
#[derive(Debug, Serialize)]
struct SymbolReport {
    name: String,
    native: String,
    parameters: Vec<ParameterReport>,
    return_type: BindingTypeReport,
}

/// One named C parameter contract.
#[derive(Debug, Serialize)]
struct ParameterReport {
    name: String,
    #[serde(rename = "type")]
    ty: BindingTypeReport,
}

/// One C enum declaration and its target-verified carrier contract.
#[derive(Debug, Serialize)]
struct EnumReport {
    name: String,
    carrier: String,
    variants: Vec<EnumVariantReport>,
}

/// One declared native enum constant spelling.
#[derive(Debug, Serialize)]
struct EnumVariantReport {
    name: String,
    native: String,
}

/// One plain C structure declaration.
#[derive(Debug, Serialize)]
struct StructReport {
    name: String,
    native: String,
    fields: Vec<StructFieldReport>,
}

/// One declared plain C structure field.
#[derive(Debug, Serialize)]
struct StructFieldReport {
    name: String,
    #[serde(rename = "type")]
    ty: BindingTypeReport,
}

/// A C type in the checked binding vocabulary, represented structurally rather than as generated Rust text.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BindingTypeReport {
    Scalar {
        spelling: String,
    },
    Pointer {
        mutable: bool,
        pointee: Box<BindingTypeReport>,
    },
    Struct {
        name: String,
    },
    Void,
}

/// A byte- and editor-addressable source span for one binding declaration.
#[derive(Debug, Serialize)]
struct BindingSourceSpan {
    file: String,
    start: usize,
    end: usize,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
}

/// Inspect checked C binding declarations for one source file or project root.
///
/// The command runs the ordinary compiler collection and typecheck path exactly once, including host-target C
/// verification, then projects only descriptors produced by that pass. It is strict: invalid source returns ordinary
/// compiler diagnostics instead of a partial report whose facts could be mistaken for a checked contract.
pub fn inspect_bindings(
    path: &Path,
    format: BindingInspectionFormat,
    feature_selection: &FeatureSelection,
    sdk_profile_override: Option<&str>,
) -> CliResult<ExitCode> {
    let entry_path = resolve_binding_entry_path(path)?;
    let session = CompilationSession::discover_with_selections(&entry_path, feature_selection, sdk_profile_override)?;
    let modules = collect_modules_detailed_with_session(entry_path, &session)
        .map_err(|failure| CliError::failure(failure.render_human()))?;
    let analysis = session
        .analyze_modules(
            &modules,
            #[cfg(feature = "rust_inspect")]
            None,
        )
        .map_err(|failure| CliError::failure(failure.render_human()))?;
    let report = binding_inspection_report(&modules, &analysis)?;
    render_binding_inspection_report(&report, format)
}

/// Resolve either an explicit Incan file or the ordinary package entrypoint used for binding inspection.
fn resolve_binding_entry_path(path: &Path) -> CliResult<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| CliError::failure(format!("failed to determine current directory: {error}")))?
            .join(path)
    };
    if absolute.is_file() {
        return Ok(absolute);
    }
    if absolute.is_dir() {
        for candidate in [absolute.join("src/main.incn"), absolute.join("src/lib.incn")] {
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        return Err(CliError::failure(format!(
            "binding inspection requires an Incan source file, or a project directory with `src/main.incn` or `src/lib.incn`: {}",
            absolute.display()
        )));
    }
    Err(CliError::failure(format!(
        "binding inspection path does not exist: {}",
        absolute.display()
    )))
}

/// Project all checked binding descriptors from one successful shared compilation analysis.
fn binding_inspection_report(
    modules: &[ParsedModule],
    analysis: &CompilationAnalysis,
) -> CliResult<BindingInspectionReport> {
    let mut bindings = Vec::new();
    for module in modules {
        let type_info = analysis
            .type_info_for_module_path(&module.path_segments)
            .ok_or_else(|| CliError::failure(format!("missing session analysis for {}", module.file_path.display())))?;
        for descriptor in type_info.c_abi.bindings.values() {
            bindings.push(binding_report(module, descriptor));
        }
    }
    bindings.sort_by(|left, right| {
        left.module
            .cmp(&right.module)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.source.file.cmp(&right.source.file))
    });
    Ok(BindingInspectionReport {
        schema_version: BINDING_INSPECTION_SCHEMA_VERSION,
        bindings,
    })
}

/// Convert one checked descriptor into the stable tooling projection without reinterpreting the source AST.
fn binding_report(module: &ParsedModule, descriptor: &CBindingDescriptor) -> BindingReport {
    BindingReport {
        module: module.path_segments.clone(),
        name: descriptor.class_name.clone(),
        header: descriptor.header.clone(),
        system_library: descriptor.system_library.clone(),
        source: binding_source_span(&module.file_path, &module.source, descriptor.span),
        symbols: descriptor
            .symbols
            .iter()
            .map(|symbol| SymbolReport {
                name: symbol.name.clone(),
                native: symbol.native.clone(),
                parameters: symbol
                    .parameters
                    .iter()
                    .map(|parameter| ParameterReport {
                        name: parameter.name.clone(),
                        ty: binding_type_report(&parameter.ty),
                    })
                    .collect(),
                return_type: binding_type_report(&symbol.return_type),
            })
            .collect(),
        enums: descriptor
            .enums
            .iter()
            .map(|enumeration| EnumReport {
                name: enumeration.name.clone(),
                carrier: scalar_type_as_str(enumeration.carrier).to_string(),
                variants: enumeration
                    .variants
                    .iter()
                    .map(|variant| EnumVariantReport {
                        name: variant.name.clone(),
                        native: variant.native.clone(),
                    })
                    .collect(),
            })
            .collect(),
        structs: descriptor
            .structs
            .iter()
            .map(|structure| StructReport {
                name: structure.name.clone(),
                native: structure.native.clone(),
                fields: structure
                    .fields
                    .iter()
                    .map(|field| StructFieldReport {
                        name: field.name.clone(),
                        ty: binding_type_report(&field.ty),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Translate the checked typechecker representation into a stable, source-vocabulary-oriented report value.
fn binding_type_report(ty: &CBindingType) -> BindingTypeReport {
    match ty {
        CBindingType::Scalar(scalar) => BindingTypeReport::Scalar {
            spelling: scalar_type_as_str(*scalar).to_string(),
        },
        CBindingType::Pointer { mutable, pointee } => BindingTypeReport::Pointer {
            mutable: *mutable,
            pointee: Box::new(binding_type_report(pointee)),
        },
        CBindingType::Struct(name) => BindingTypeReport::Struct { name: name.clone() },
        CBindingType::Void => BindingTypeReport::Void,
    }
}

/// Render one stable report as either machine-readable JSON or a concise terminal summary.
fn render_binding_inspection_report(
    report: &BindingInspectionReport,
    format: BindingInspectionFormat,
) -> CliResult<ExitCode> {
    match format {
        BindingInspectionFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(report)
                .map_err(|error| CliError::failure(format!("failed to serialize binding inspection: {error}")))?
        ),
        BindingInspectionFormat::Text => render_binding_inspection_text(report),
    }
    Ok(ExitCode::SUCCESS)
}

/// Print the human-readable declaration summary without creating a second serialization contract.
fn render_binding_inspection_text(report: &BindingInspectionReport) {
    if report.bindings.is_empty() {
        println!("No checked C bindings.");
        return;
    }
    for binding in &report.bindings {
        println!("Binding {} ({})", binding.name, binding.module.join("::"));
        println!("  header: {}", binding.header);
        println!("  link: c.system_library(\"{}\")", binding.system_library);
        println!(
            "  source: {}:{}:{}",
            binding.source.file, binding.source.start_line, binding.source.start_column
        );
        for symbol in &binding.symbols {
            let parameters = symbol
                .parameters
                .iter()
                .map(|parameter| format!("{}: {}", parameter.name, format_binding_type(&parameter.ty)))
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "  symbol {}({parameters}) -> {} [native: {}]",
                symbol.name,
                format_binding_type(&symbol.return_type),
                symbol.native
            );
        }
        for enumeration in &binding.enums {
            println!("  enum {}: {}", enumeration.name, enumeration.carrier);
            for variant in &enumeration.variants {
                println!("    {} = {}", variant.name, variant.native);
            }
        }
        for structure in &binding.structs {
            println!("  struct {} [native: {}]", structure.name, structure.native);
            for field in &structure.fields {
                println!("    {}: {}", field.name, format_binding_type(&field.ty));
            }
        }
    }
}

/// Format one structured binding type with the corresponding Incan C vocabulary spelling for terminal output.
fn format_binding_type(ty: &BindingTypeReport) -> String {
    match ty {
        BindingTypeReport::Scalar { spelling } => spelling.clone(),
        BindingTypeReport::Pointer { mutable, pointee } => {
            let constructor = if *mutable { "c.MutPtr" } else { "c.ConstPtr" };
            format!("{constructor}[{}]", format_binding_type(pointee))
        }
        BindingTypeReport::Struct { name } => name.clone(),
        BindingTypeReport::Void => "None".to_string(),
    }
}

/// Convert the compiler's byte span into a deterministic source anchor for external tooling.
fn binding_source_span(path: &Path, source: &str, span: Span) -> BindingSourceSpan {
    let start = span.start.min(source.len());
    let end = span.end.min(source.len()).max(start);
    let (start_line, start_column) = line_column_for_offset(source, start);
    let (end_line, end_column) = line_column_for_offset(source, end);
    BindingSourceSpan {
        file: path.to_string_lossy().to_string(),
        start,
        end,
        start_line,
        start_column,
        end_line,
        end_column,
    }
}

/// Convert a byte offset into 1-based display coordinates while preserving byte offsets for precise tool anchors.
fn line_column_for_offset(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let mut line = 1usize;
    let mut column = 1usize;
    for (index, character) in source.char_indices() {
        if index >= offset {
            break;
        }
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}
