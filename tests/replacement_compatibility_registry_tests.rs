//! Contract tests for the replacement compatibility control-plane registry.

use incan::replacement_compatibility::{
    ComparisonEvidence, IndependentComparisonState, LandingProvenanceState,
    REPLACEMENT_COMPATIBILITY_INVENTORY_SCHEMA_VERSION, checked_v0_5_public_capability_baseline,
    render_developer_projection, render_machine_readable_inventory, replacement_compatibility_registry,
    validate_replacement_compatibility_registry,
};

#[test]
fn release_pinned_baseline_is_checked_and_complete() -> Result<(), Box<dyn std::error::Error>> {
    let baseline = checked_v0_5_public_capability_baseline()?;

    assert_eq!(baseline.release.tag, "v0.5.0");
    assert_eq!(baseline.capabilities.len(), 67);
    assert_eq!(baseline.release.source_blob, "42f718a9c35f816a68bb3ff13578eaf6725e3d0b");
    assert!(
        baseline
            .capabilities
            .iter()
            .any(|capability| capability.id == "FirstClassFunctions")
    );
    assert!(baseline.capabilities.iter().any(|capability| capability.id == "StdWeb"));
    let mut unresolved = baseline
        .capabilities
        .iter()
        .filter(|capability| {
            matches!(
                capability.landing_provenance.state,
                LandingProvenanceState::HistoricalDiscrepancyUnresolved
            )
        })
        .map(|capability| capability.id.as_str())
        .collect::<Vec<_>>();
    unresolved.sort_unstable();
    assert_eq!(
        unresolved,
        [
            "AsyncAwait",
            "CodegraphInspection",
            "StdWeb",
            "TypeTokensReflection",
            "ValueEnums"
        ]
    );
    assert!(
        baseline
            .capabilities
            .iter()
            .filter(|capability| {
                matches!(
                    capability.landing_provenance.state,
                    LandingProvenanceState::HistoricalDiscrepancyUnresolved
                )
            })
            .all(|capability| capability.landing_provenance.owner_issue == Some(1153))
    );
    Ok(())
}

#[test]
fn registry_covers_the_baseline_without_claiming_parity() -> Result<(), Box<dyn std::error::Error>> {
    let baseline = checked_v0_5_public_capability_baseline()?;
    let registry = replacement_compatibility_registry();

    validate_replacement_compatibility_registry(&baseline, &registry)?;
    assert_eq!(registry.features.len(), 27);
    assert!(registry.features.iter().all(|feature| {
        !feature.evidence.is_parity_green()
            && matches!(
                feature.evidence.independent_comparison,
                IndependentComparisonState::NonGreenShadowUnavailable
            )
    }));
    let scalar = registry
        .features
        .iter()
        .find(|feature| feature.id == "language.numeric-and-scalar")
        .ok_or("missing scalar direct-profile feature")?;
    assert!(!scalar.evidence.is_parity_green());
    assert_eq!(scalar.evidence.surfaces.scoped_comparisons.len(), 1);
    let compared_case = &scalar.evidence.surfaces.scoped_comparisons[0];
    assert_eq!(compared_case.case_id, "replacement-body-v0-001");
    assert!(matches!(compared_case.state, IndependentComparisonState::ComparedMatch));
    assert!(matches!(&compared_case.evidence, ComparisonEvidence::Paired { .. }));
    assert!(
        registry
            .features
            .iter()
            .filter(|feature| feature.id != scalar.id)
            .all(|feature| { feature.evidence.surfaces.scoped_comparisons.is_empty() })
    );
    Ok(())
}

#[test]
fn joined_projection_is_deterministic_and_exposes_the_callable_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let baseline = checked_v0_5_public_capability_baseline()?;
    let registry = replacement_compatibility_registry();

    let projection = render_developer_projection(&baseline, &registry)?;
    assert!(projection.contains("# Replacement compatibility inventory"));
    assert!(projection.contains("`call.stored-callables`"));
    assert!(projection.contains("NonGreenShadowUnavailable"));
    assert!(projection.contains("#1152"));
    assert!(projection.contains("HistoricalDiscrepancyUnresolved; owner #1153"));
    assert!(projection.contains("replacement-body-v0-001: ComparedMatch"));
    assert!(projection.contains("legacy_receipt_identity"));
    assert!(projection.contains("replacement_receipt_identity"));
    assert!(projection.contains("completed comparison infrastructure #1146"));
    assert!(projection.contains("outstanding evidence owner #1152"));
    assert!(projection.contains("unscheduled evidence debt"));
    assert!(!projection.contains("unavailable via #1146"));

    let machine: serde_json::Value = serde_json::from_str(&render_machine_readable_inventory(&baseline, &registry)?)?;
    assert!(machine.is_object());
    assert_eq!(
        machine.get("schema_version").and_then(serde_json::Value::as_u64),
        Some(u64::from(REPLACEMENT_COMPATIBILITY_INVENTORY_SCHEMA_VERSION))
    );
    assert!(machine.get("baseline").is_some());
    assert!(machine.get("registry").is_some());
    Ok(())
}
