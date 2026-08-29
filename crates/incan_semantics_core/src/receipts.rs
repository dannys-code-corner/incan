//! RFC 104 operation receipts.
//!
//! A receipt is the durable record of what one capability-aware operation actually did. It is the counterpart to
//! [`crate::facts::AuthorityDecision`]: the decision says whether an operation was permitted, and the receipt says
//! what happened once that decision was applied — including the case where the decision was a denial and the
//! operation never ran.
//!
//! The receipt is deliberately one shape for every publisher. A stdlib host boundary, a package-defined domain
//! operation, and a provider operation all emit this, so a backend execution receipt can *reference* a receipt
//! without copying or reinterpreting its authority, redaction, or replay semantics. Two owners for redaction would
//! mean two chances to disagree about what was persisted.

use crate::facts::{AuthorityDecision, CanonicalSymbolId};

/// What happened to one capability-aware operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReceiptStatus {
    /// The operation ran and was recorded, without authority being enforced.
    Observed,
    /// Authority was granted and the operation ran to completion.
    Allowed,
    /// Authority was refused, so the operation never performed its behavior.
    Denied,
    /// Authority was granted but the operation itself failed.
    Failed,
    /// The operation ran, but its recorded attributes were redacted before reaching a sink.
    Redacted,
    /// The operation was not attempted, for a reason other than a denial.
    Skipped,
    /// The operation performed part of its work before stopping.
    Partial,
}

impl ReceiptStatus {
    /// Return the compact snapshot spelling for this status.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Failed => "failed",
            Self::Redacted => "redacted",
            Self::Skipped => "skipped",
            Self::Partial => "partial",
        }
    }
}

/// How replayable an operation is.
///
/// RFC 104 does not require the runtime to implement replay. It requires the runtime not to make dishonest replay
/// claims, which is why this is recorded per receipt rather than inferred later from the operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReplayClassification {
    /// Replayable from recorded local inputs, such as a filesystem write determined by its recorded arguments.
    Deterministic,
    /// Replay depends on an external system and cannot be exact without a recording.
    External,
    /// Replay needs a recorded fixture or test double.
    FixtureRequired,
    /// Replay data existed but was intentionally not persisted.
    Redacted,
    /// Replay is not supported for this operation.
    Unavailable,
}

impl ReplayClassification {
    /// Return the compact snapshot spelling for this classification.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::External => "external",
            Self::FixtureRequired => "fixture-required",
            Self::Redacted => "redacted",
            Self::Unavailable => "unavailable",
        }
    }
}

/// How sensitive one recorded attribute's value is.
///
/// This travels with the attribute rather than being decided at the sink, so a receipt that crosses a boundary keeps
/// the provenance a redaction policy needs (RFC 103).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttributeSensitivity {
    /// Safe to record as written.
    Public,
    /// Recorded locally, but not intended for export.
    Internal,
    /// Never recorded in the clear.
    Secret,
}

impl AttributeSensitivity {
    /// Return the compact snapshot spelling for this sensitivity level.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Secret => "secret",
        }
    }
}

/// One attribute recorded on a receipt.
///
/// A redacted attribute keeps its key and its sensitivity and drops only the value. That is what lets a reader see
/// *that* an HTTP URL was recorded and deliberately withheld, rather than being unable to tell a withheld value from
/// an operation that never had one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptAttribute {
    /// The attribute's name, such as `http.method` or `fs.path_policy`.
    pub key: String,
    /// The recorded value, or `None` when it was redacted.
    pub value: Option<String>,
    /// How sensitive the value is, recorded whether or not it was persisted.
    pub sensitivity: AttributeSensitivity,
}

impl ReceiptAttribute {
    /// Record an attribute whose value is safe to persist as written.
    pub fn public(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: Some(value.into()),
            sensitivity: AttributeSensitivity::Public,
        }
    }

    /// Record an attribute whose value was withheld.
    ///
    /// RFC 104 redacts sensitive values by default, so this is the constructor a publisher reaches for whenever the
    /// value is not `Public`; persisting one takes a deliberate call to [`Self::public`].
    pub fn redacted(key: impl Into<String>, sensitivity: AttributeSensitivity) -> Self {
        Self {
            key: key.into(),
            value: None,
            sensitivity,
        }
    }

    /// Whether this attribute's value was withheld.
    pub const fn is_redacted(&self) -> bool {
        self.value.is_none()
    }
}

/// A reference from another receipt to this one.
///
/// A backend execution receipt holds this rather than a copy. Copying would give redaction and replay two owners, and
/// two owners eventually disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationReceiptRef {
    /// The referenced receipt's sequence id within its run.
    pub sequence_id: u64,
}

/// Ways a receipt can contradict itself.
///
/// These are contract violations rather than runtime errors: each one means the receipt claims something its own
/// other fields deny, which is exactly the dishonesty RFC 104 asks the runtime to avoid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptContractViolation {
    /// The status says denied but the linked authority decision allowed the operation, or the reverse.
    StatusContradictsAuthority {
        /// The status the receipt claims.
        status: ReceiptStatus,
        /// Whether the linked decision actually allowed the operation.
        authority_allowed: bool,
    },
    /// The receipt withheld attribute values yet claims replay from recorded local inputs.
    DeterministicReplayOverRedactedAttributes {
        /// The keys whose values were withheld.
        redacted_keys: Vec<String>,
    },
}

impl std::fmt::Display for ReceiptContractViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StatusContradictsAuthority {
                status,
                authority_allowed,
            } => write!(
                f,
                "receipt status `{}` contradicts an authority decision that {}",
                status.as_str(),
                if *authority_allowed { "allowed" } else { "denied" },
            ),
            Self::DeterministicReplayOverRedactedAttributes { redacted_keys } => write!(
                f,
                "receipt claims deterministic replay but withheld {}",
                redacted_keys.join(", "),
            ),
        }
    }
}

/// The durable record of one capability-aware operation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationReceipt {
    /// Position within the emitting run, which is also this receipt's identity for references.
    pub sequence_id: u64,
    /// The capability whose authority the operation needed.
    pub capability: CanonicalSymbolId,
    /// The operation that ran, or would have run.
    pub operation: CanonicalSymbolId,
    /// The publisher's own name for what kind of operation this was, such as `http.request`.
    pub operation_kind: String,
    /// What happened.
    pub status: ReceiptStatus,
    /// The authority decision this receipt records the outcome of.
    pub authority: AuthorityDecision,
    /// Where the operation was written.
    pub source_span: crate::HirSourceSpan,
    /// The enclosing context's sequence id, when the operation ran inside one.
    pub parent_context: Option<u64>,
    /// Operation-specific attributes, redacted or not.
    pub attributes: Vec<ReceiptAttribute>,
    /// How replayable this operation is.
    pub replay: ReplayClassification,
}

impl OperationReceipt {
    /// Build the receipt for a governed denial.
    ///
    /// A denial produces a receipt without the provider ever being invoked, which is the point: RFC 104 treats a
    /// refusal as a first-class outcome to be recorded, not as an absence of one. The status and replay
    /// classification both follow from the decision, so a caller cannot accidentally record a denial as a success.
    pub fn denied(
        sequence_id: u64,
        authority: AuthorityDecision,
        operation_kind: impl Into<String>,
        source_span: crate::HirSourceSpan,
    ) -> Self {
        Self {
            sequence_id,
            capability: authority.capability.clone(),
            operation: authority.provenance.operation.clone(),
            operation_kind: operation_kind.into(),
            status: ReceiptStatus::Denied,
            authority,
            source_span,
            parent_context: None,
            attributes: Vec::new(),
            // Nothing ran, so there are no recorded inputs to replay from.
            replay: ReplayClassification::Unavailable,
        }
    }

    /// A reference other receipts can hold instead of a copy.
    pub const fn reference(&self) -> OperationReceiptRef {
        OperationReceiptRef {
            sequence_id: self.sequence_id,
        }
    }

    /// The keys whose values were withheld.
    pub fn redacted_keys(&self) -> Vec<String> {
        self.attributes
            .iter()
            .filter(|attribute| attribute.is_redacted())
            .map(|attribute| attribute.key.clone())
            .collect()
    }

    /// Check that this receipt does not contradict itself.
    ///
    /// Both rules exist because a receipt is read long after the run that produced it, by a consumer with no way to
    /// re-derive the truth: a status that disagrees with its own authority decision, or a deterministic replay claim
    /// over inputs that were never persisted, would each be believed.
    pub fn validate(&self) -> Result<(), ReceiptContractViolation> {
        let authority_allowed = self.authority.is_allowed();
        if (self.status == ReceiptStatus::Denied) == authority_allowed {
            return Err(ReceiptContractViolation::StatusContradictsAuthority {
                status: self.status,
                authority_allowed,
            });
        }

        if self.replay == ReplayClassification::Deterministic {
            let redacted_keys = self.redacted_keys();
            if !redacted_keys.is_empty() {
                return Err(ReceiptContractViolation::DeterministicReplayOverRedactedAttributes { redacted_keys });
            }
        }

        Ok(())
    }
}

impl std::fmt::Display for OperationReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "#{} {} {} {} replay={}",
            self.sequence_id,
            self.operation_kind,
            self.capability.declaration_name,
            self.status.as_str(),
            self.replay.as_str(),
        )?;
        let redacted = self.redacted_keys();
        if !redacted.is_empty() {
            write!(f, " redacted=[{}]", redacted.join(","))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{
        AuthorityDenialReason, AuthorityGrantContext, AuthorityMode, AuthorityProvenance, SemanticSourceTargetKind,
    };

    /// Build an authority decision for `host.http.request` requested by `app.billing.charge`.
    fn authority(allowed: bool) -> AuthorityDecision {
        let capability = CanonicalSymbolId::module_declaration(
            vec!["host".to_string(), "http".to_string()],
            "request",
            SemanticSourceTargetKind::Capability,
            crate::HirSourceSpan::new(10, 20),
        );
        let provenance = AuthorityProvenance {
            operation: CanonicalSymbolId::module_declaration(
                vec!["app".to_string(), "billing".to_string()],
                "charge",
                SemanticSourceTargetKind::Function,
                crate::HirSourceSpan::new(80, 96),
            ),
            request_span: crate::HirSourceSpan::new(120, 140),
            suggested_grant: "host.http.request".to_string(),
        };
        let grant = AuthorityGrantContext {
            requested_scope: Vec::new(),
            ceiling_applied: false,
        };
        if allowed {
            AuthorityDecision::allowed(capability, AuthorityMode::Governed, grant, provenance)
        } else {
            AuthorityDecision::denied(
                capability,
                AuthorityMode::Governed,
                AuthorityDenialReason::NotGranted,
                grant,
                provenance,
            )
        }
    }

    /// Build an allowed receipt with the given attributes and replay classification.
    fn allowed_receipt(attributes: Vec<ReceiptAttribute>, replay: ReplayClassification) -> OperationReceipt {
        let decision = authority(true);
        OperationReceipt {
            sequence_id: 7,
            capability: decision.capability.clone(),
            operation: decision.provenance.operation.clone(),
            operation_kind: "http.request".to_string(),
            status: ReceiptStatus::Allowed,
            authority: decision,
            source_span: crate::HirSourceSpan::new(120, 140),
            parent_context: None,
            attributes,
            replay,
        }
    }

    /// A governed denial must produce a receipt without the provider ever being invoked.
    #[test]
    fn a_governed_denial_produces_a_denied_receipt_with_nothing_executed() -> Result<(), String> {
        let receipt =
            OperationReceipt::denied(1, authority(false), "http.request", crate::HirSourceSpan::new(120, 140));

        assert_eq!(receipt.status, ReceiptStatus::Denied);
        assert!(receipt.attributes.is_empty(), "nothing ran, so nothing was recorded");
        assert_eq!(
            receipt.replay,
            ReplayClassification::Unavailable,
            "there are no recorded inputs to replay from",
        );
        assert_eq!(receipt.capability.declaration_name, "request");
        assert_eq!(receipt.operation.declaration_name, "charge");
        receipt.validate().map_err(|violation| violation.to_string())
    }

    /// An allowed receipt validates and carries its recorded attributes.
    #[test]
    fn an_allowed_receipt_records_its_attributes() -> Result<(), String> {
        let receipt = allowed_receipt(
            vec![ReceiptAttribute::public("http.method", "GET")],
            ReplayClassification::External,
        );

        assert_eq!(receipt.status, ReceiptStatus::Allowed);
        assert!(receipt.redacted_keys().is_empty());
        receipt.validate().map_err(|violation| violation.to_string())
    }

    /// A failed operation is still an allowed one: authority was granted, the operation itself did not succeed.
    #[test]
    fn a_failed_operation_keeps_its_allowed_authority() -> Result<(), String> {
        let mut receipt = allowed_receipt(Vec::new(), ReplayClassification::External);
        receipt.status = ReceiptStatus::Failed;

        receipt.validate().map_err(|violation| violation.to_string())
    }

    /// A redacted attribute keeps its key and sensitivity, and drops only the value.
    #[test]
    fn a_redacted_attribute_keeps_its_key_and_sensitivity() -> Result<(), String> {
        let receipt = allowed_receipt(
            vec![
                ReceiptAttribute::public("http.method", "GET"),
                ReceiptAttribute::redacted("http.url", AttributeSensitivity::Secret),
            ],
            ReplayClassification::Redacted,
        );

        assert_eq!(receipt.redacted_keys(), vec!["http.url".to_string()]);
        let withheld = receipt
            .attributes
            .iter()
            .find(|attribute| attribute.key == "http.url")
            .ok_or("the redacted attribute is missing")?;
        assert_eq!(withheld.value, None);
        assert_eq!(
            withheld.sensitivity,
            AttributeSensitivity::Secret,
            "sensitivity survives redaction, so a policy downstream still knows why the value is absent",
        );
        receipt.validate().map_err(|violation| violation.to_string())
    }

    /// A status that disagrees with its own authority decision is a contract violation.
    #[test]
    fn a_denied_status_over_an_allowing_decision_is_rejected() {
        let mut receipt = allowed_receipt(Vec::new(), ReplayClassification::External);
        receipt.status = ReceiptStatus::Denied;

        assert_eq!(
            receipt.validate(),
            Err(ReceiptContractViolation::StatusContradictsAuthority {
                status: ReceiptStatus::Denied,
                authority_allowed: true,
            }),
        );
    }

    /// Claiming deterministic replay over withheld inputs is the dishonest claim RFC 104 forbids.
    #[test]
    fn deterministic_replay_over_redacted_attributes_is_rejected() {
        let receipt = allowed_receipt(
            vec![ReceiptAttribute::redacted("http.url", AttributeSensitivity::Secret)],
            ReplayClassification::Deterministic,
        );

        assert_eq!(
            receipt.validate(),
            Err(ReceiptContractViolation::DeterministicReplayOverRedactedAttributes {
                redacted_keys: vec!["http.url".to_string()],
            }),
        );
    }

    /// A backend receipt references this one rather than copying it.
    #[test]
    fn a_receipt_reference_carries_only_the_sequence_id() {
        let receipt = allowed_receipt(Vec::new(), ReplayClassification::External);

        assert_eq!(receipt.reference(), OperationReceiptRef { sequence_id: 7 });
    }

    /// Every status and replay classification needs a distinct snapshot spelling.
    #[test]
    fn statuses_and_replay_classifications_have_distinct_spellings() {
        let statuses = [
            ReceiptStatus::Observed,
            ReceiptStatus::Allowed,
            ReceiptStatus::Denied,
            ReceiptStatus::Failed,
            ReceiptStatus::Redacted,
            ReceiptStatus::Skipped,
            ReceiptStatus::Partial,
        ];
        let status_spellings: std::collections::HashSet<&str> = statuses.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            status_spellings.len(),
            statuses.len(),
            "two statuses share one spelling"
        );

        let replays = [
            ReplayClassification::Deterministic,
            ReplayClassification::External,
            ReplayClassification::FixtureRequired,
            ReplayClassification::Redacted,
            ReplayClassification::Unavailable,
        ];
        let replay_spellings: std::collections::HashSet<&str> = replays.iter().map(|r| r.as_str()).collect();
        assert_eq!(
            replay_spellings.len(),
            replays.len(),
            "two replay classifications share one spelling",
        );
    }
}
