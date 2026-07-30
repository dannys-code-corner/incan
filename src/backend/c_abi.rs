//! Target-aware Clang verification for the bounded checked C binding foundation.
//!
//! The frontend owns C binding semantics. This module deliberately receives only a checked descriptor and renders a
//! non-executable C translation unit for one selected target. It never reads headers to discover symbols, guesses a
//! signature, or searches the host for a matching library.

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::frontend::typechecker::{CBindingDescriptor, CBindingEnum, CBindingStruct, CBindingType};
use incan_core::lang::c_abi::ScalarTypeId;

type EnumValueProbeRequest = (String, String, String);

/// A Clang-compatible target supplied by the checked C foundation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Both target identities are part of the verifier contract; one is host-selected per build.
pub(crate) enum CAbiTarget {
    /// GNU-compatible Linux x86-64 ABI.
    LinuxX86_64,
    /// Apple arm64 ABI.
    MacosArm64,
}

impl CAbiTarget {
    /// Stable target triple passed to Clang.
    pub(crate) const fn triple(self) -> &'static str {
        match self {
            Self::LinuxX86_64 => "x86_64-unknown-linux-gnu",
            Self::MacosArm64 => "arm64-apple-macos11",
        }
    }

    /// Target that matches the compiler host running this invocation.
    pub(crate) const fn host() -> Option<Self> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            return Some(Self::MacosArm64);
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            return Some(Self::LinuxX86_64);
        }
        #[allow(unreachable_code)]
        None
    }
}

/// Explicit Clang executable used for one verifier invocation.
///
/// Oven will eventually provide this capability through a selected Loaf target. The first checked-binding slice
/// keeps that policy out of source declarations while accepting an explicit test/CI override and the platform's
/// Clang-compatible toolchain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClangToolchain {
    executable: PathBuf,
}

impl ClangToolchain {
    /// Select the current platform's Clang-compatible compiler without consulting binding names or headers.
    pub(crate) fn discover() -> Result<Self, CAbiVerificationError> {
        if let Some(executable) = env::var_os("INCAN_C_ABI_CLANG").filter(|value| !value.is_empty()) {
            return Ok(Self {
                executable: PathBuf::from(executable),
            });
        }
        #[cfg(target_os = "macos")]
        {
            let output = Command::new("xcrun")
                .args(["--find", "clang"])
                .output()
                .map_err(|error| CAbiVerificationError::toolchain(format!("could not select Xcode Clang: {error}")))?;
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(Self {
                        executable: PathBuf::from(path),
                    });
                }
            }
            Err(CAbiVerificationError::toolchain(format!(
                "could not select Xcode Clang: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(Self {
                executable: PathBuf::from("clang"),
            })
        }
    }

    /// Construct a test-only toolchain with an explicit executable path.
    #[cfg(test)]
    fn at(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

/// One source-anchorable verifier failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CAbiVerificationError {
    /// Binding that could not be checked, when known.
    pub(crate) binding: Option<String>,
    /// Human-readable reason safe to present as an Incan diagnostic.
    pub(crate) message: String,
}

/// Target-verified C enum values consumed by the ordinary Incan lowering path.
///
/// The C probe is the authority for these values. Generated Rust receives only
/// the resulting scalar, never a guessed header spelling or macro expansion.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CAbiVerificationReceipt {
    enum_values: BTreeMap<(String, String), i64>,
}

impl CAbiVerificationReceipt {
    /// Return one verified value by its binding-local enum and variant names.
    #[cfg(test)]
    pub(crate) fn enum_value(&self, enumeration: &str, variant: &str) -> Option<i64> {
        self.enum_values
            .get(&(enumeration.to_string(), variant.to_string()))
            .copied()
    }

    /// Iterate over every verified enum value in stable declaration-key order.
    pub(crate) fn enum_values(&self) -> impl Iterator<Item = (&(String, String), &i64)> {
        self.enum_values.iter()
    }
}

impl CAbiVerificationError {
    /// Construct a verifier failure that belongs to one checked binding.
    fn binding(binding: &CBindingDescriptor, message: impl Into<String>) -> Self {
        Self {
            binding: Some(binding.class_name.clone()),
            message: message.into(),
        }
    }

    /// Construct a verifier failure that cannot be associated with one binding.
    fn toolchain(message: impl Into<String>) -> Self {
        Self {
            binding: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for CAbiVerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(binding) = &self.binding {
            write!(formatter, "C binding `{binding}` verification failed: {}", self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl std::error::Error for CAbiVerificationError {}

/// Verify declared C symbols and plain layouts for one selected target.
///
/// This is deliberately syntax-only: it validates the resolved header's declarations before Rust project generation
/// and does not perform ambient linker probing. Linking remains a separate selected-artifact concern in #942.
pub(crate) fn verify_checked_c_binding(
    toolchain: &ClangToolchain,
    target: CAbiTarget,
    binding: &CBindingDescriptor,
) -> Result<CAbiVerificationReceipt, CAbiVerificationError> {
    let source = render_verification_probe(binding)?;
    let mut command = Command::new(&toolchain.executable);
    command
        .args([
            "-std=c11",
            "-Werror",
            "-fsyntax-only",
            "-x",
            "c",
            "-target",
            target.triple(),
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        CAbiVerificationError::binding(
            binding,
            format!(
                "could not start selected Clang toolchain `{}` for target `{}`: {error}",
                toolchain.executable.display(),
                target.triple()
            ),
        )
    })?;
    let Some(stdin) = child.stdin.as_mut() else {
        return Err(CAbiVerificationError::binding(
            binding,
            "selected Clang toolchain did not expose standard input for the verifier probe",
        ));
    };
    stdin.write_all(source.as_bytes()).map_err(|error| {
        CAbiVerificationError::binding(binding, format!("could not write C verifier probe: {error}"))
    })?;
    let output = child.wait_with_output().map_err(|error| {
        CAbiVerificationError::binding(binding, format!("could not wait for C verifier probe: {error}"))
    })?;
    if output.status.success() {
        return verify_enum_values(toolchain, target, binding);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(CAbiVerificationError::binding(
        binding,
        format!(
            "Clang rejected the declared signature or layout for target `{}`:\n{}",
            target.triple(),
            stderr.trim()
        ),
    ))
}

/// Extract every native enum expression as an `i64` from Clang's target AST.
///
/// This probe is syntax-only and does not link or execute target code. A
/// generated anonymous enum lets Clang fold C macros and constant expressions
/// using the selected target ABI, then its JSON AST reports the exact value.
fn verify_enum_values(
    toolchain: &ClangToolchain,
    target: CAbiTarget,
    binding: &CBindingDescriptor,
) -> Result<CAbiVerificationReceipt, CAbiVerificationError> {
    let (source, requested) = render_enum_value_probe(binding)?;
    if requested.is_empty() {
        return Ok(CAbiVerificationReceipt::default());
    }
    let mut command = Command::new(&toolchain.executable);
    command
        .args([
            "-std=c11",
            "-Werror",
            "-fsyntax-only",
            "-x",
            "c",
            "-target",
            target.triple(),
            "-Xclang",
            "-ast-dump=json",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        CAbiVerificationError::binding(
            binding,
            format!(
                "could not start selected Clang toolchain `{}` for enum values on target `{}`: {error}",
                toolchain.executable.display(),
                target.triple()
            ),
        )
    })?;
    let Some(stdin) = child.stdin.as_mut() else {
        return Err(CAbiVerificationError::binding(
            binding,
            "selected Clang toolchain did not expose standard input for the enum verifier probe",
        ));
    };
    stdin.write_all(source.as_bytes()).map_err(|error| {
        CAbiVerificationError::binding(binding, format!("could not write C enum verifier probe: {error}"))
    })?;
    let output = child.wait_with_output().map_err(|error| {
        CAbiVerificationError::binding(binding, format!("could not wait for C enum verifier probe: {error}"))
    })?;
    if !output.status.success() {
        return Err(CAbiVerificationError::binding(
            binding,
            format!(
                "Clang could not evaluate declared enum constants for target `{}`:\n{}",
                target.triple(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let ast = serde_json::from_slice::<serde_json::Value>(&output.stdout).map_err(|error| {
        CAbiVerificationError::binding(binding, format!("Clang returned an invalid enum AST: {error}"))
    })?;
    let mut enum_values = BTreeMap::new();
    for (enumeration, variant, generated_name) in requested {
        let Some(value) = ast_enum_constant_value(&ast, &generated_name) else {
            return Err(CAbiVerificationError::binding(
                binding,
                format!("Clang did not report a value for `{enumeration}.{variant}`"),
            ));
        };
        let value = value.parse::<i64>().map_err(|error| {
            CAbiVerificationError::binding(
                binding,
                format!("C enum value for `{enumeration}.{variant}` is outside Incan `int`: {error}"),
            )
        })?;
        enum_values.insert((enumeration, variant), value);
    }
    Ok(CAbiVerificationReceipt { enum_values })
}

/// Render a C source unit whose named anonymous-enum constants Clang can fold.
fn render_enum_value_probe(
    binding: &CBindingDescriptor,
) -> Result<(String, Vec<EnumValueProbeRequest>), CAbiVerificationError> {
    let mut probe = format!(
        "/* Generated by the Incan checked C ABI verifier. */\n#include \"{}\"\n\n",
        escape_c_include(&binding.header)
    );
    let mut requested = Vec::new();
    for enumeration in &binding.enums {
        for variant in &enumeration.variants {
            let native = checked_c_identifier(binding, &variant.native, "native enum constant")?;
            let generated_name = format!(
                "__incan_c_value_{}_{}_{}",
                c_identifier_component(&binding.class_name),
                c_identifier_component(&enumeration.name),
                c_identifier_component(&variant.name),
            );
            probe.push_str(&format!("enum {{ {generated_name} = ({native}) }};\n"));
            requested.push((enumeration.name.clone(), variant.name.clone(), generated_name));
        }
    }
    Ok((probe, requested))
}

/// Find one generated enum constant's folded integer value in Clang's JSON AST.
fn ast_enum_constant_value<'a>(value: &'a serde_json::Value, expected_name: &str) -> Option<&'a str> {
    let object = value.as_object()?;
    if object.get("kind").and_then(serde_json::Value::as_str) == Some("EnumConstantDecl")
        && object.get("name").and_then(serde_json::Value::as_str) == Some(expected_name)
    {
        return ast_constant_value(value);
    }
    object
        .get("inner")
        .and_then(serde_json::Value::as_array)
        .and_then(|children| {
            children
                .iter()
                .find_map(|child| ast_enum_constant_value(child, expected_name))
        })
}

/// Find the first folded constant expression nested beneath an enum declaration.
fn ast_constant_value(value: &serde_json::Value) -> Option<&str> {
    let object = value.as_object()?;
    if object.get("kind").and_then(serde_json::Value::as_str) == Some("ConstantExpr")
        && let Some(value) = object.get("value").and_then(serde_json::Value::as_str)
    {
        return Some(value);
    }
    object
        .get("inner")
        .and_then(serde_json::Value::as_array)
        .and_then(|children| children.iter().find_map(ast_constant_value))
}

/// Render a deterministic, non-executable C probe from one checked descriptor.
fn render_verification_probe(binding: &CBindingDescriptor) -> Result<String, CAbiVerificationError> {
    let mut probe = format!(
        "/* Generated by the Incan checked C ABI verifier. */\n#include \"{}\"\n\n",
        escape_c_include(&binding.header)
    );
    for symbol in &binding.symbols {
        let native = checked_c_identifier(binding, &symbol.native, "native symbol")?;
        let parameters = if symbol.parameters.is_empty() {
            "void".to_string()
        } else {
            symbol
                .parameters
                .iter()
                .map(|parameter| c_type_spelling(binding, &parameter.ty, false))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        };
        let result = c_type_spelling(binding, &symbol.return_type, true)?;
        probe.push_str(&format!(
            "_Static_assert(_Generic(&{native}, {result} (*)({parameters}): 1, default: 0), \"Incan C signature mismatch: {}.{}\");\n",
            binding.class_name, symbol.name
        ));
    }
    for structure in &binding.structs {
        render_structure_layout_probe(&mut probe, binding, structure)?;
    }
    for enumeration in &binding.enums {
        render_enum_carrier_probes(&mut probe, binding, enumeration)?;
    }
    Ok(probe)
}

/// Render one carrier check for every source-visible C enum constant.
///
/// The declaration's `c.*` carrier is an explicit ABI promise. C does not
/// retain a portable reflection API for enumeration values, but `_Generic`
/// checks the native constant expression after its header macro expansion.
/// That catches a missing symbol and a physical carrier mismatch without
/// inventing a source-level spelling for the platform's enum representation.
fn render_enum_carrier_probes(
    probe: &mut String,
    binding: &CBindingDescriptor,
    enumeration: &CBindingEnum,
) -> Result<(), CAbiVerificationError> {
    let carrier = c_scalar_spelling(enumeration.carrier);
    for variant in &enumeration.variants {
        let native = checked_c_identifier(binding, &variant.native, "native enum constant")?;
        probe.push_str(&format!(
            "_Static_assert(_Generic(({native}), {carrier}: 1, default: 0), \"Incan C enum carrier mismatch: {}.{}.{}\");\n",
            binding.class_name, enumeration.name, variant.name
        ));
    }
    Ok(())
}

/// Render layout equivalence checks for one explicitly listed plain C structure.
fn render_structure_layout_probe(
    probe: &mut String,
    binding: &CBindingDescriptor,
    structure: &CBindingStruct,
) -> Result<(), CAbiVerificationError> {
    let native = checked_c_type_name(binding, &structure.native, "plain structure native type")?;
    let expected = format!(
        "__incan_expected_{}_{}",
        c_identifier_component(&binding.class_name),
        c_identifier_component(&structure.name)
    );
    probe.push_str(&format!("typedef struct {expected} {{\n"));
    for field in &structure.fields {
        let ty = c_type_spelling(binding, &field.ty, false)?;
        let field_name = checked_c_identifier(binding, &field.name, "plain structure field")?;
        probe.push_str(&format!("    {ty} {field_name};\n"));
    }
    probe.push_str(&format!("}} {expected};\n"));
    probe.push_str(&format!(
        "_Static_assert(sizeof({native}) == sizeof({expected}), \"Incan C layout size mismatch: {}.{}\");\n",
        binding.class_name, structure.name
    ));
    probe.push_str(&format!(
        "_Static_assert(_Alignof({native}) == _Alignof({expected}), \"Incan C layout alignment mismatch: {}.{}\");\n",
        binding.class_name, structure.name
    ));
    for field in &structure.fields {
        let field_name = checked_c_identifier(binding, &field.name, "plain structure field")?;
        probe.push_str(&format!(
            "_Static_assert(__builtin_offsetof({native}, {field_name}) == __builtin_offsetof({expected}, {field_name}), \"Incan C layout field offset mismatch: {}.{}.{}\");\n",
            binding.class_name, structure.name, field.name
        ));
    }
    Ok(())
}

/// Render one compiler-known C type without allowing source text to introduce arbitrary C fragments.
fn c_type_spelling(
    binding: &CBindingDescriptor,
    ty: &CBindingType,
    allow_void: bool,
) -> Result<String, CAbiVerificationError> {
    match ty {
        CBindingType::Void if allow_void => Ok("void".to_string()),
        CBindingType::Void => Err(CAbiVerificationError::binding(
            binding,
            "`None` is valid only as a C function return type",
        )),
        CBindingType::Scalar(scalar) => Ok(c_scalar_spelling(*scalar).to_string()),
        CBindingType::Pointer { mutable, pointee } => {
            let pointee = c_type_spelling(binding, pointee, false)?;
            let qualifier = if *mutable { "" } else { "const " };
            Ok(format!("{qualifier}{pointee} *"))
        }
        CBindingType::Struct(name) => {
            let Some(structure) = binding.structs.iter().find(|structure| structure.name == *name) else {
                return Err(CAbiVerificationError::binding(
                    binding,
                    format!("C structure `{name}` is not declared by this binding"),
                ));
            };
            checked_c_type_name(binding, &structure.native, "plain structure native type")
        }
    }
}

/// Return a target-aware builtin spelling for an exact C scalar category.
fn c_scalar_spelling(scalar: ScalarTypeId) -> &'static str {
    match scalar {
        ScalarTypeId::I8 => "__INT8_TYPE__",
        ScalarTypeId::U8 => "__UINT8_TYPE__",
        ScalarTypeId::I16 => "__INT16_TYPE__",
        ScalarTypeId::U16 => "__UINT16_TYPE__",
        ScalarTypeId::I32 => "__INT32_TYPE__",
        ScalarTypeId::U32 => "__UINT32_TYPE__",
        ScalarTypeId::I64 => "__INT64_TYPE__",
        ScalarTypeId::U64 => "__UINT64_TYPE__",
        ScalarTypeId::Size => "__SIZE_TYPE__",
        ScalarTypeId::CChar => "char",
        ScalarTypeId::CInt => "int",
    }
}

/// Require a C identifier rather than permitting unstructured probe injection.
fn checked_c_identifier<'a>(
    binding: &CBindingDescriptor,
    value: &'a str,
    label: &str,
) -> Result<&'a str, CAbiVerificationError> {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return Err(CAbiVerificationError::binding(
            binding,
            format!("{label} cannot be empty"),
        ));
    };
    if !(first == '_' || first.is_ascii_alphabetic())
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return Err(CAbiVerificationError::binding(
            binding,
            format!("{label} `{value}` is not a supported C identifier"),
        ));
    }
    Ok(value)
}

/// Admit only a C identifier or `struct <identifier>` for declared plain layouts.
fn checked_c_type_name(
    binding: &CBindingDescriptor,
    value: &str,
    label: &str,
) -> Result<String, CAbiVerificationError> {
    if let Some(tag) = value.strip_prefix("struct ") {
        checked_c_identifier(binding, tag, label)?;
        return Ok(value.to_string());
    }
    checked_c_identifier(binding, value, label)?;
    Ok(value.to_string())
}

/// Escape an explicit header path for one generated C include directive.
fn escape_c_include(header: &str) -> String {
    header.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Convert source names into a private generated C identifier component.
fn c_identifier_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{CAbiTarget, ClangToolchain, verify_checked_c_binding};
    use crate::frontend::typechecker::{
        CBindingDescriptor, CBindingEnum, CBindingEnumVariant, CBindingParameter, CBindingStruct, CBindingStructField,
        CBindingSymbol, CBindingType,
    };
    use incan_core::lang::c_abi::ScalarTypeId;

    fn fixture_binding(header: String) -> CBindingDescriptor {
        CBindingDescriptor {
            span: crate::frontend::ast::Span::default(),
            class_name: "Fixture".to_string(),
            header,
            system_library: "fixture".to_string(),
            symbols: vec![CBindingSymbol {
                name: "absolute".to_string(),
                native: "fixture_abs".to_string(),
                parameters: vec![CBindingParameter {
                    name: "value".to_string(),
                    ty: CBindingType::Scalar(ScalarTypeId::I32),
                }],
                return_type: CBindingType::Scalar(ScalarTypeId::I32),
            }],
            enums: vec![CBindingEnum {
                name: "Status".to_string(),
                carrier: ScalarTypeId::I32,
                variants: vec![CBindingEnumVariant {
                    name: "OK".to_string(),
                    native: "FIXTURE_OK".to_string(),
                }],
            }],
            structs: vec![CBindingStruct {
                name: "Pair".to_string(),
                native: "fixture_pair".to_string(),
                fields: vec![
                    CBindingStructField {
                        name: "left".to_string(),
                        ty: CBindingType::Scalar(ScalarTypeId::I32),
                    },
                    CBindingStructField {
                        name: "right".to_string(),
                        ty: CBindingType::Scalar(ScalarTypeId::I32),
                    },
                ],
            }],
        }
    }

    fn host_clang() -> Option<ClangToolchain> {
        ClangToolchain::discover().ok()
    }

    #[test]
    fn host_clang_verifies_signature_and_plain_layout() -> Result<(), Box<dyn std::error::Error>> {
        let Some(target) = CAbiTarget::host() else {
            return Ok(());
        };
        let Some(toolchain) = host_clang() else {
            return Ok(());
        };
        let temporary = tempfile::tempdir()?;
        let header = temporary.path().join("fixture.h");
        std::fs::write(
            &header,
            "typedef struct fixture_pair { int left; int right; } fixture_pair;\n#define FIXTURE_OK 0\nint fixture_abs(int value);\n",
        )?;
        let receipt = verify_checked_c_binding(
            &toolchain,
            target,
            &fixture_binding(header.to_string_lossy().into_owned()),
        )?;
        assert_eq!(receipt.enum_value("Status", "OK"), Some(0));
        Ok(())
    }

    #[test]
    fn clang_syntax_verifies_the_foundation_fixture_for_linux_and_macos_targets()
    -> Result<(), Box<dyn std::error::Error>> {
        let Some(toolchain) = host_clang() else {
            return Ok(());
        };
        let temporary = tempfile::tempdir()?;
        let header = temporary.path().join("fixture.h");
        std::fs::write(
            &header,
            "typedef struct fixture_pair { int left; int right; } fixture_pair;\n#define FIXTURE_OK 0\nint fixture_abs(int value);\n",
        )?;
        let binding = fixture_binding(header.to_string_lossy().into_owned());
        for target in [CAbiTarget::LinuxX86_64, CAbiTarget::MacosArm64] {
            verify_checked_c_binding(&toolchain, target, &binding)?;
        }
        Ok(())
    }

    #[test]
    fn verifier_reports_mismatched_checked_signature() -> Result<(), Box<dyn std::error::Error>> {
        let Some(target) = CAbiTarget::host() else {
            return Ok(());
        };
        let Some(toolchain) = host_clang() else {
            return Ok(());
        };
        let temporary = tempfile::tempdir()?;
        let header = temporary.path().join("fixture.h");
        std::fs::write(
            &header,
            "typedef struct fixture_pair { int left; int right; } fixture_pair;\n#define FIXTURE_OK 0\nlong fixture_abs(int value);\n",
        )?;
        let error = verify_checked_c_binding(
            &toolchain,
            target,
            &fixture_binding(header.to_string_lossy().into_owned()),
        )
        .expect_err("mismatched C return type must be rejected");
        assert!(
            error.message.contains("Clang rejected"),
            "unexpected verifier error: {error}"
        );
        assert!(
            error.message.contains("Incan C signature mismatch"),
            "unexpected verifier error: {error}"
        );
        Ok(())
    }

    #[test]
    fn verifier_rejects_an_enum_constant_with_the_wrong_carrier() -> Result<(), Box<dyn std::error::Error>> {
        let Some(target) = CAbiTarget::host() else {
            return Ok(());
        };
        let Some(toolchain) = host_clang() else {
            return Ok(());
        };
        let temporary = tempfile::tempdir()?;
        let header = temporary.path().join("fixture.h");
        std::fs::write(
            &header,
            "typedef struct fixture_pair { int left; int right; } fixture_pair;\n#define FIXTURE_OK 0\nint fixture_abs(int value);\n",
        )?;
        let mut binding = fixture_binding(header.to_string_lossy().into_owned());
        binding.enums[0].carrier = ScalarTypeId::U32;
        let error = verify_checked_c_binding(&toolchain, target, &binding)
            .expect_err("an enum carrier mismatch must be rejected");
        assert!(
            error.message.contains("Incan C enum carrier mismatch"),
            "unexpected verifier error: {error}"
        );
        Ok(())
    }

    #[test]
    fn target_catalogue_names_linux_and_macos_abi_triples() {
        assert_eq!(CAbiTarget::LinuxX86_64.triple(), "x86_64-unknown-linux-gnu");
        assert_eq!(CAbiTarget::MacosArm64.triple(), "arm64-apple-macos11");
    }

    #[test]
    fn test_toolchain_constructor_is_explicit() {
        let toolchain = ClangToolchain::at("/fixture/clang");
        assert_eq!(toolchain.executable, std::path::PathBuf::from("/fixture/clang"));
    }
}
