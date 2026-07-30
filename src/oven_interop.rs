//! Oven-owned interop requirements and their portable lock projection.
//!
//! An Oven interop declaration names package-owned inputs and compatibility requirements for one target. It does not
//! claim that a compiler or SDK has already been selected, perform ambient discovery, compile a shim, or decide
//! application semantics. Oven resolves those requirements and records its selections in a separate build receipt.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path};

use semver::VersionReq;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::manifest::ProjectManifest;

/// Current compatibility format for the `[oven.interop]` manifest section.
pub const OVEN_INTEROP_SCHEMA_VERSION: u32 = 1;

/// Oven-owned manifest settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct OvenSection {
    /// Target-specific checked-interop requirements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interop: Option<OvenInteropSection>,
}

/// Target-specific package inputs and compatibility requirements for checked interop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct OvenInteropSection {
    /// Version of the Oven interop requirement schema.
    pub schema: u32,
    /// Independently declared requirements for each target triple.
    #[serde(default)]
    pub targets: Vec<OvenInteropTarget>,
}

impl OvenInteropSection {
    /// Validate configuration that is independent from the package filesystem.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != OVEN_INTEROP_SCHEMA_VERSION {
            return Err(format!(
                "[oven.interop].schema must be {OVEN_INTEROP_SCHEMA_VERSION}, found {}",
                self.schema
            ));
        }
        if self.targets.is_empty() {
            return Err("[oven.interop] requires at least one [[oven.interop.targets]] entry".to_string());
        }

        let mut target_names = BTreeSet::new();
        for target in &self.targets {
            if target.target.trim().is_empty() || !target_names.insert(target.target.clone()) {
                return Err(format!(
                    "Oven interop target `{}` must be a unique non-empty target triple",
                    target.target
                ));
            }
            if let Some(toolchain) = &target.toolchain {
                toolchain.validate(&format!("Oven interop target `{}` toolchain", target.target))?;
            }
            if let Some(sdk) = &target.sdk {
                sdk.validate(&format!("Oven interop target `{}` SDK", target.target))?;
            }
            validate_interop_paths(
                &target.headers,
                &format!("Oven interop target `{}` header", target.target),
            )?;
            if target.definitions.iter().any(|definition| definition.trim().is_empty()) {
                return Err(format!(
                    "Oven interop target `{}` contains an empty preprocessor definition",
                    target.target
                ));
            }

            let mut artifact_names = BTreeSet::new();
            for artifact in &target.artifacts {
                validate_artifact(artifact, &target.target, &mut artifact_names)?;
            }
            for artifact in &target.artifacts {
                validate_artifact_dependencies(artifact, &target.target, &artifact_names)?;
            }
            let mut shim_names = BTreeSet::new();
            for shim in &target.shims {
                if shim.name.trim().is_empty() || !shim_names.insert(shim.name.clone()) {
                    return Err(format!(
                        "Oven interop target `{}` shim names must be unique and non-empty",
                        target.target
                    ));
                }
                if shim.sources.is_empty() {
                    return Err(format!(
                        "interop shim `{}` requires at least one source input",
                        shim.name
                    ));
                }
                validate_interop_paths(&shim.sources, &format!("interop shim `{}` source", shim.name))?;
                validate_interop_paths(&shim.headers, &format!("interop shim `{}` header", shim.name))?;
                if shim.output.trim().is_empty() {
                    return Err(format!("interop shim `{}` requires a logical output name", shim.name));
                }
            }
        }
        Ok(())
    }
}

/// A compiler or SDK capability accepted by one package target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct CapabilityRequirement {
    /// Stable capability identity resolved by Oven.
    pub capability: String,
    /// Optional compatible version requirement, not a claim about the locally selected version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl CapabilityRequirement {
    /// Reject an empty capability or a version expression outside the supported semantic-version requirement grammar.
    fn validate(&self, label: &str) -> Result<(), String> {
        if self.capability.trim().is_empty() {
            return Err(format!("{label} capability must be non-empty"));
        }
        if self.version.as_ref().is_some_and(|version| version.trim().is_empty()) {
            return Err(format!("{label} version requirement must be non-empty"));
        }
        if let Some(version) = &self.version {
            VersionReq::parse(version).map_err(|error| format!("{label} version requirement is invalid: {error}"))?;
        }
        Ok(())
    }

    /// Canonicalize a validated capability and version requirement before recording it in the semantic lock.
    fn normalized(&self) -> Result<Self, String> {
        let version = self
            .version
            .as_deref()
            .map(VersionReq::parse)
            .transpose()
            .map_err(|error| format!("invalid Oven capability version requirement: {error}"))?
            .map(|requirement| requirement.to_string());
        Ok(Self {
            capability: self.capability.trim().to_string(),
            version,
        })
    }
}

/// Package inputs and compatibility requirements declared for one target triple.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct OvenInteropTarget {
    /// Compilation and deployment target triple requested by the package.
    pub target: String,
    /// Optional compatible Clang-family capability; Oven records the selected executable separately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<CapabilityRequirement>,
    /// Optional compatible SDK capability; Oven records the selected SDK and sysroot separately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk: Option<CapabilityRequirement>,
    /// Package-relative public or shim headers used for verification.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<String>,
    /// Explicit preprocessor definitions supplied to verification and later shim baking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub definitions: Vec<String>,
    /// Package-owned artifacts or system capabilities required by this target.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<InteropArtifact>,
    /// Authored C or C++ shim source inputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shims: Vec<InteropShim>,
}

/// One declared physical interop artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct InteropArtifact {
    /// Binding-local logical name for this artifact.
    pub name: String,
    /// How the requested target consumes this artifact.
    pub kind: InteropArtifactKind,
    /// Package-relative file for static and bundled artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Toolchain or SDK capability for a system-provided artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    /// Runtime loader name required for a bundled artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_name: Option<String>,
    /// Platform-packaging destination for a bundled artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<String>,
    /// Minimum platform constraint for a bundled artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_platform: Option<String>,
    /// Logical names of transitive interop artifact dependencies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

/// Deployment class for a declared interop artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteropArtifactKind {
    /// An archive linked directly into the generated product.
    Static,
    /// A dynamic library or framework staged for a platform packager.
    Bundled,
    /// A library or framework supplied by the resolved SDK or toolchain capability.
    System,
}

/// One authored shim whose bounded exported surface will be verified and built by Oven.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct InteropShim {
    /// Stable package-local shim name.
    pub name: String,
    /// Source language accepted by the future managed shim baker.
    pub language: InteropShimLanguage,
    /// Package-relative authored C or C++ source files.
    pub sources: Vec<String>,
    /// Package-relative headers that describe the shim's bounded exported contract.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<String>,
    /// Logical name for the generated interop output.
    pub output: String,
}

/// Language of one authored interop shim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteropShimLanguage {
    /// C source compiled by the selected managed Clang-compatible toolchain.
    C,
    /// C++ source compiled only behind a bounded C-compatible shim surface.
    Cxx,
}

/// Portable Oven interop requirements frozen in one canonical semantic lock state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedInteropTarget {
    /// Target triple requested by the package plan.
    pub target: String,
    /// Compatible toolchain capability requested by the package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<CapabilityRequirement>,
    /// Compatible SDK capability requested by the package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk: Option<CapabilityRequirement>,
    /// Explicit definitions sorted as part of the target configuration identity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub definitions: Vec<String>,
    /// Header inputs and their content identities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<LockedInteropInput>,
    /// Declared static, bundled, or system artifact identities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<LockedInteropArtifact>,
    /// Authored shim source and header identities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shims: Vec<LockedInteropShim>,
}

/// One package-relative immutable interop input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedInteropInput {
    /// Package-relative portable input path.
    pub path: String,
    /// Content digest computed from the exact declared file bytes.
    pub digest: String,
}

/// One declared artifact projected into portable lock data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedInteropArtifact {
    /// Binding-local artifact name.
    pub name: String,
    /// Static, bundled, or system deployment class.
    pub kind: InteropArtifactKind,
    /// File input identity when the package provides artifact bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<LockedInteropInput>,
    /// Toolchain capability when the selected artifact is system-provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    /// Runtime loader name for bundled outputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_name: Option<String>,
    /// Packaging destination for bundled outputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<String>,
    /// Minimum platform constraint for bundled outputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_platform: Option<String>,
    /// Transitive interop dependencies in deterministic order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

/// One authored shim's immutable sources and intended logical output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedInteropShim {
    /// Stable package-local shim name.
    pub name: String,
    /// Selected C or C++ source language.
    pub language: InteropShimLanguage,
    /// Authored source file identities.
    pub sources: Vec<LockedInteropInput>,
    /// Shim-header identities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<LockedInteropInput>,
    /// Logical output name selected for later shim baking.
    pub output: String,
}

/// Resolve declared Oven interop files into lockable content identities without ambient host discovery.
pub fn locked_oven_interop_targets(manifest: &ProjectManifest) -> Result<Vec<LockedInteropTarget>, String> {
    locked_oven_interop_targets_from_section(manifest.project_root(), manifest.oven_interop())
}

/// Resolve a parsed Oven interop declaration for the specified project root into portable lock entries.
///
/// This accepts the already-parsed manifest section so lock generation can include interop inputs alongside the
/// provider and SDK semantic state without rediscovering or reparsing the manifest.
pub fn locked_oven_interop_targets_from_section(
    project_root: &Path,
    interop: Option<&OvenInteropSection>,
) -> Result<Vec<LockedInteropTarget>, String> {
    let Some(interop) = interop else {
        return Ok(Vec::new());
    };
    interop.validate()?;
    let mut targets = interop
        .targets
        .iter()
        .map(|target| lock_interop_target(project_root, target))
        .collect::<Result<Vec<_>, _>>()?;
    targets.sort_by(|left, right| left.target.cmp(&right.target));
    Ok(targets)
}

/// Validate one target artifact's deployment kind and the fields it is allowed to declare.
///
/// The manifest uses mutually exclusive shapes for directly linked static archives, packager-staged bundled files,
/// and required toolchain/SDK capabilities. Enforcing that distinction before any file access keeps later target
/// resolution from interpreting an ambiguous declaration as ambient host discovery.
fn validate_artifact(
    artifact: &InteropArtifact,
    target: &str,
    artifact_names: &mut BTreeSet<String>,
) -> Result<(), String> {
    if artifact.name.trim().is_empty() || !artifact_names.insert(artifact.name.clone()) {
        return Err(format!(
            "Oven interop target `{target}` artifact names must be unique and non-empty"
        ));
    }
    match artifact.kind {
        InteropArtifactKind::Static => {
            validate_required_path(
                artifact.path.as_deref(),
                &format!("static interop artifact `{}`", artifact.name),
            )?;
            if artifact.capability.is_some() {
                return Err(format!(
                    "static interop artifact `{}` cannot declare a system capability",
                    artifact.name
                ));
            }
            if artifact.runtime_name.is_some() || artifact.placement.is_some() || artifact.minimum_platform.is_some() {
                return Err(format!(
                    "static interop artifact `{}` cannot declare bundled deployment fields",
                    artifact.name
                ));
            }
        }
        InteropArtifactKind::Bundled => {
            validate_required_path(
                artifact.path.as_deref(),
                &format!("bundled interop artifact `{}`", artifact.name),
            )?;
            validate_non_empty(
                artifact.runtime_name.as_deref(),
                &format!("bundled interop artifact `{}` runtime-name", artifact.name),
            )?;
            validate_non_empty(
                artifact.placement.as_deref(),
                &format!("bundled interop artifact `{}` placement", artifact.name),
            )?;
            validate_non_empty(
                artifact.minimum_platform.as_deref(),
                &format!("bundled interop artifact `{}` minimum-platform", artifact.name),
            )?;
            if artifact.capability.is_some() {
                return Err(format!(
                    "bundled interop artifact `{}` cannot declare a system capability",
                    artifact.name
                ));
            }
        }
        InteropArtifactKind::System => {
            validate_non_empty(
                artifact.capability.as_deref(),
                &format!("system interop artifact `{}` capability", artifact.name),
            )?;
            if artifact.path.is_some() {
                return Err(format!(
                    "system interop artifact `{}` cannot declare a package path",
                    artifact.name
                ));
            }
            if artifact.runtime_name.is_some() || artifact.placement.is_some() || artifact.minimum_platform.is_some() {
                return Err(format!(
                    "system interop artifact `{}` cannot declare bundled deployment fields",
                    artifact.name
                ));
            }
        }
    }
    Ok(())
}

/// Require each declared artifact dependency to name one distinct sibling in the same target declaration.
///
/// Interop artifact order is normalized in the lock projection, so dependencies must use stable logical names rather
/// than filesystem paths or an implicit declaration order.
fn validate_artifact_dependencies(
    artifact: &InteropArtifact,
    target: &str,
    artifact_names: &BTreeSet<String>,
) -> Result<(), String> {
    let mut dependencies = BTreeSet::new();
    for dependency in &artifact.dependencies {
        if dependency.trim().is_empty()
            || dependency == &artifact.name
            || !artifact_names.contains(dependency)
            || !dependencies.insert(dependency.clone())
        {
            return Err(format!(
                "interop artifact `{}` on target `{target}` must depend on distinct declared sibling artifacts",
                artifact.name
            ));
        }
    }
    Ok(())
}

/// Require one optional interop path field and validate it with the common package-relative path policy.
fn validate_required_path(path: Option<&str>, label: &str) -> Result<(), String> {
    let Some(path) = path else {
        return Err(format!("{label} requires a package-relative path"));
    };
    validate_interop_path(path, label)
}

/// Require one optional manifest string field to contain a non-whitespace value.
fn validate_non_empty(value: Option<&str>, label: &str) -> Result<(), String> {
    if value.is_none_or(|value| value.trim().is_empty()) {
        return Err(format!("{label} must be non-empty"));
    }
    Ok(())
}

/// Validate every package path in one manifest list using the shared normalized-relative-path contract.
fn validate_interop_paths(paths: &[String], label: &str) -> Result<(), String> {
    for path in paths {
        validate_interop_path(path, label)?;
    }
    Ok(())
}

/// Reject a path that could escape the package or acquire platform-dependent meaning outside the declaration.
///
/// Interop inputs are locked by their package-relative identity and content bytes. Absolute, parent-relative, current
/// directory, backslash, duplicate-separator, and directory spellings would make that identity ambiguous or permit
/// ambient filesystem lookup.
fn validate_interop_path(path: &str, label: &str) -> Result<(), String> {
    let candidate = Path::new(path);
    if path.trim().is_empty()
        || path.contains('\\')
        || path.contains("//")
        || path.ends_with('/')
        || candidate.is_absolute()
        || candidate.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::CurDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "{label} path `{path}` must be a non-empty normalized package-relative path"
        ));
    }
    Ok(())
}

/// Convert one validated target declaration into a canonical, content-addressed lock projection.
///
/// This stage records only already-declared package files and logical deployment facts. It does not build shims,
/// download artifacts, or probe the host for a library.
fn lock_interop_target(root: &Path, target: &OvenInteropTarget) -> Result<LockedInteropTarget, String> {
    let mut definitions = target.definitions.clone();
    definitions.sort();
    definitions.dedup();
    let headers = lock_inputs(root, &target.headers)?;
    let mut artifacts = target
        .artifacts
        .iter()
        .map(|artifact| lock_artifact(root, artifact))
        .collect::<Result<Vec<_>, _>>()?;
    artifacts.sort_by(|left, right| left.name.cmp(&right.name));
    let mut shims = target
        .shims
        .iter()
        .map(|shim| {
            Ok(LockedInteropShim {
                name: shim.name.clone(),
                language: shim.language,
                sources: lock_inputs(root, &shim.sources)?,
                headers: lock_inputs(root, &shim.headers)?,
                output: shim.output.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    shims.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(LockedInteropTarget {
        target: target.target.clone(),
        toolchain: target
            .toolchain
            .as_ref()
            .map(CapabilityRequirement::normalized)
            .transpose()?,
        sdk: target.sdk.as_ref().map(CapabilityRequirement::normalized).transpose()?,
        definitions,
        headers,
        artifacts,
        shims,
    })
}

/// Lock one physical artifact while retaining capability-only system artifacts without a package file receipt.
fn lock_artifact(root: &Path, artifact: &InteropArtifact) -> Result<LockedInteropArtifact, String> {
    let input = artifact
        .path
        .as_deref()
        .map(|path| lock_input(root, path))
        .transpose()?;
    let mut dependencies = artifact.dependencies.clone();
    dependencies.sort();
    dependencies.dedup();
    Ok(LockedInteropArtifact {
        name: artifact.name.clone(),
        kind: artifact.kind,
        input,
        capability: artifact.capability.clone(),
        runtime_name: artifact.runtime_name.clone(),
        placement: artifact.placement.clone(),
        minimum_platform: artifact.minimum_platform.clone(),
        dependencies,
    })
}

/// Resolve a list of declared package files into unique, path-sorted content receipts.
fn lock_inputs(root: &Path, paths: &[String]) -> Result<Vec<LockedInteropInput>, String> {
    let mut inputs = paths
        .iter()
        .map(|path| lock_input(root, path))
        .collect::<Result<Vec<_>, _>>()?;
    inputs.sort_by(|left, right| left.path.cmp(&right.path));
    inputs.dedup_by(|left, right| left.path == right.path);
    Ok(inputs)
}

/// Hash one declared regular package file without following a symlink or consulting an ambient search path.
fn lock_input(root: &Path, relative: &str) -> Result<LockedInteropInput, String> {
    validate_interop_path(relative, "interop input")?;
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("failed to inspect declared interop input {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "declared interop input {} must be a regular file",
            path.display()
        ));
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("failed to read declared interop input {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(LockedInteropInput {
        path: relative.to_string(),
        digest: format!("sha256:{}", hex::encode(hasher.finalize())),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn project_with_oven_interop_inputs() -> Result<(TempDir, ProjectManifest), Box<dyn std::error::Error>> {
        let workspace = TempDir::new()?;
        fs::create_dir_all(workspace.path().join("interop/include"))?;
        fs::create_dir_all(workspace.path().join("interop/src"))?;
        fs::create_dir_all(workspace.path().join("interop/lib"))?;
        fs::write(workspace.path().join("interop/include/bridge.h"), "int bridge(void);\n")?;
        fs::write(
            workspace.path().join("interop/src/bridge.c"),
            "int bridge(void) { return 7; }\n",
        )?;
        fs::write(workspace.path().join("interop/lib/libfixture.a"), b"fixture archive")?;
        let manifest_path = workspace.path().join("incan.toml");
        let manifest = ProjectManifest::from_str(
            r#"
[oven.interop]
schema = 1

[[oven.interop.targets]]
target = "aarch64-apple-ios"
toolchain = { capability = "apple-clang", version = ">=17, <18" }
sdk = { capability = "iphoneos", version = ">=18, <19" }
headers = ["interop/include/bridge.h"]
definitions = ["FIXTURE=1"]

[[oven.interop.targets.artifacts]]
name = "fixture"
kind = "static"
path = "interop/lib/libfixture.a"

[[oven.interop.targets.artifacts]]
name = "foundation"
kind = "system"
capability = "apple.framework.Foundation"

[[oven.interop.targets.shims]]
name = "fixture_bridge"
language = "c"
sources = ["interop/src/bridge.c"]
headers = ["interop/include/bridge.h"]
output = "fixture_bridge"
"#,
            &manifest_path,
        )?;
        Ok((workspace, manifest))
    }

    #[test]
    fn oven_interop_inputs_lock_portably_and_change_with_declared_bytes() -> Result<(), Box<dyn std::error::Error>> {
        let (workspace, manifest) = project_with_oven_interop_inputs()?;
        let first = locked_oven_interop_targets(&manifest)?;
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].target, "aarch64-apple-ios");
        assert_eq!(
            first[0]
                .toolchain
                .as_ref()
                .map(|requirement| requirement.capability.as_str()),
            Some("apple-clang")
        );
        assert_eq!(first[0].headers[0].path, "interop/include/bridge.h");
        assert_eq!(
            first[0].artifacts[0].input.as_ref().map(|input| input.path.as_str()),
            Some("interop/lib/libfixture.a")
        );
        assert_eq!(
            first[0].artifacts[1].capability.as_deref(),
            Some("apple.framework.Foundation")
        );
        assert_eq!(first[0].shims[0].sources[0].path, "interop/src/bridge.c");

        fs::write(
            workspace.path().join("interop/src/bridge.c"),
            "int bridge(void) { return 8; }\n",
        )?;
        let second = locked_oven_interop_targets(&manifest)?;
        assert_ne!(
            first[0].shims[0].sources[0].digest,
            second[0].shims[0].sources[0].digest
        );
        Ok(())
    }

    #[test]
    fn oven_interop_inputs_reject_ambient_paths_and_incomplete_bundles() {
        let invalid_requirement = OvenInteropSection {
            schema: OVEN_INTEROP_SCHEMA_VERSION,
            targets: vec![OvenInteropTarget {
                target: "x86_64-unknown-linux-gnu".to_string(),
                toolchain: Some(CapabilityRequirement {
                    capability: "clang".to_string(),
                    version: Some("eighteen-ish".to_string()),
                }),
                sdk: None,
                headers: Vec::new(),
                definitions: Vec::new(),
                artifacts: Vec::new(),
                shims: Vec::new(),
            }],
        };
        assert!(invalid_requirement.validate().is_err());

        let absolute = OvenInteropSection {
            schema: OVEN_INTEROP_SCHEMA_VERSION,
            targets: vec![OvenInteropTarget {
                target: "x86_64-unknown-linux-gnu".to_string(),
                toolchain: Some(CapabilityRequirement {
                    capability: "clang".to_string(),
                    version: Some(">=18, <19".to_string()),
                }),
                sdk: None,
                headers: vec!["/usr/include/fixture.h".to_string()],
                definitions: Vec::new(),
                artifacts: Vec::new(),
                shims: Vec::new(),
            }],
        };
        assert!(absolute.validate().is_err());

        let bundled = OvenInteropSection {
            schema: OVEN_INTEROP_SCHEMA_VERSION,
            targets: vec![OvenInteropTarget {
                target: "x86_64-apple-darwin".to_string(),
                toolchain: None,
                sdk: Some(CapabilityRequirement {
                    capability: "macosx".to_string(),
                    version: Some(">=15, <16".to_string()),
                }),
                headers: Vec::new(),
                definitions: Vec::new(),
                artifacts: vec![InteropArtifact {
                    name: "fixture".to_string(),
                    kind: InteropArtifactKind::Bundled,
                    path: Some("interop/lib/libfixture.dylib".to_string()),
                    capability: None,
                    runtime_name: None,
                    placement: None,
                    minimum_platform: None,
                    dependencies: Vec::new(),
                }],
                shims: Vec::new(),
            }],
        };
        assert!(bundled.validate().is_err());

        let invalid_dependency = OvenInteropSection {
            schema: OVEN_INTEROP_SCHEMA_VERSION,
            targets: vec![OvenInteropTarget {
                target: "x86_64-unknown-linux-gnu".to_string(),
                toolchain: None,
                sdk: None,
                headers: Vec::new(),
                definitions: Vec::new(),
                artifacts: vec![InteropArtifact {
                    name: "fixture".to_string(),
                    kind: InteropArtifactKind::Static,
                    path: Some("interop/lib/libfixture.a".to_string()),
                    capability: None,
                    runtime_name: None,
                    placement: None,
                    minimum_platform: None,
                    dependencies: vec!["missing".to_string()],
                }],
                shims: Vec::new(),
            }],
        };
        assert!(invalid_dependency.validate().is_err());
    }
}
