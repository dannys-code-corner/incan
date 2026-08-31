//! Hashed set and dict values for the replacement profile's membership work (#1247).
//!
//! The replacement executor refuses set and dict programs at the *aggregate* today: [`super::ReplacementValue`]
//! has no container value to construct, so `v in xs` over a set and `k in d` over a dict never reach the membership
//! helpers Body IR already names. This module supplies the representation that boundary move needs — and only the
//! representation. Wiring it into `ReplacementValue`, the rvalue-profile admission, and the four membership arms
//! (`set_contains`, `set_not_contains`, `dict_contains_key`, `dict_not_contains_key`) stays with the #1247
//! integration work, so nothing here reads spans, receipts, or the executor's dispatch tables. A refusal leaves
//! here as [`NonScalarKey`]; the arm that hits it owns the original source span and phrases the `unsupported`
//! report there, the same way the list-membership arm phrases its own scalar guard.
//!
//! ## Cost model
//!
//! Entries live in [`HashSet`]/[`HashMap`] keyed by [`HashedKey`], so a membership probe is a hashed lookup. That
//! is a contract, not an implementation detail: the source says hashed container, and
//! `incan_stdlib::collections::set_contains` takes `&HashSet` precisely so `value in set` never quietly becomes a
//! linear scan. #1247 rejected representing these containers as pair lists for the same reason — the executor's
//! answers would have agreed with the Rust-emission backend while its cost model quietly did not.
//!
//! ## Key identity
//!
//! A key is admitted exactly when `ReplacementValue::is_collection_scalar` admits the value — `int`, `bool`,
//! `str`, and the unit value — and that lockstep is pinned by test rather than assumed. Anything outside the
//! domain refuses instead of answering: at construction for elements and dict keys, because a hashed container
//! cannot even hold what it cannot hash, and at probe time for needles, including over an empty container, so a
//! `false` always means "absent" and never "could not tell". Distinct scalar kinds never compare equal — `1` is
//! not `true` here — which is the same equality the list-membership arm already applies through
//! `ReplacementValue`'s own `PartialEq`.
//!
//! ## Membership-grade only
//!
//! The surface is construction, membership, equality, and deterministic rendering. There is deliberately no
//! iteration, indexing, length, or mutation: those are #1247 non-goals, and the representation does not retain
//! source insertion order, which any future iteration support would have to revisit (the language's dicts iterate
//! in insertion order). Rendering therefore sorts entries into the canonical [`HashedKey`] order — a determinism
//! choice for receipts and debugging, not a claim about source order, and never observable through membership.

use std::collections::{HashMap, HashSet};

use incan_core::lang::surface::constructors::{ConstructorId, as_str as constructor_name};

use super::ReplacementValue;

/// Refusal for a value outside the hashed-container key domain.
///
/// Deliberately span-free: this module never sees source positions, so the executor arm that receives the refusal
/// attaches the original span and the operation's own `unsupported` spelling. The retained kind label names what
/// was refused (for example `list` or `float`) so that report can say so.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a {kind} value cannot key a hashed container in the replacement profile")]
pub struct NonScalarKey {
    /// Short kind label of the refused value, for refusal messages.
    pub kind: &'static str,
}

/// One hashed-container key in the replacement profile's collection-scalar domain.
///
/// The variants mirror `ReplacementValue::is_collection_scalar` exactly, and the lockstep is pinned by test.
/// `Float` stays out deliberately: it sits outside that guard, and its textual carrier makes equality-by-hash a
/// lie waiting to happen. The derived `Ord` exists only so rendering can sort entries deterministically; a
/// well-typed program never holds keys of mixed kinds, and ordering is never observable through membership.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HashedKey {
    /// An Incan `int` key.
    Int(i64),
    /// An Incan `bool` key.
    Bool(bool),
    /// An owned Incan `str` key.
    Str(String),
    /// The Incan `None`/unit key.
    Unit,
}

impl HashedKey {
    /// Admit one evaluated replacement value as a hashed key, or refuse it with its kind.
    ///
    /// Takes the value by ownership because every caller — aggregate construction and membership probes alike —
    /// holds an evaluated operand it no longer needs, and admitting a `str` key without re-allocating its text is
    /// the point of consuming it.
    pub fn try_from_value(value: ReplacementValue) -> Result<Self, NonScalarKey> {
        match value {
            ReplacementValue::Int(value) => Ok(Self::Int(value)),
            ReplacementValue::Bool(value) => Ok(Self::Bool(value)),
            ReplacementValue::Str(value) => Ok(Self::Str(value)),
            ReplacementValue::Unit => Ok(Self::Unit),
            other => Err(NonScalarKey {
                kind: value_kind_label(&other),
            }),
        }
    }

    /// Render the same source-observable spelling `ReplacementValue::observable_text` gives this scalar.
    ///
    /// Duplicating the four scalar spellings here, rather than converting back into a `ReplacementValue`, keeps
    /// rendering allocation-shaped like the rest of the observable-text family; the shared spelling is pinned by
    /// test so the two cannot drift apart silently.
    fn observable_text(&self) -> String {
        match self {
            Self::Int(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
            Self::Str(value) => value.clone(),
            Self::Unit => constructor_name(ConstructorId::None).to_string(),
        }
    }
}

/// Short kind label used when a value is refused as a hashed-container key.
///
/// Total over every `ReplacementValue` variant on purpose, scalars included: when the #1247 integration work adds
/// the container variants themselves, this match stops compiling and forces their labels to exist, instead of a
/// wildcard quietly labeling them wrong.
fn value_kind_label(value: &ReplacementValue) -> &'static str {
    match value {
        ReplacementValue::Int(_) => "int",
        ReplacementValue::Bool(_) => "bool",
        ReplacementValue::Str(_) => "str",
        ReplacementValue::Float(_) => "float",
        ReplacementValue::Unit => "unit",
        ReplacementValue::Range { .. } => "range",
        ReplacementValue::List { .. } => "list",
        ReplacementValue::Tuple(_) => "tuple",
        ReplacementValue::Nominal { .. } => "nominal",
        ReplacementValue::FieldlessEnum { .. } => "fieldless-enum member",
        ReplacementValue::ValueEnum { .. } => "value-enum member",
        ReplacementValue::Result { .. } => "Result",
        ReplacementValue::Callable(_) => "callable",
        ReplacementValue::Generator(_) => "generator",
        ReplacementValue::Task(_) => "task",
        ReplacementValue::Adapter(_) => "generator-adapter",
        ReplacementValue::CollectedGenerator { .. } => "collected-generator",
    }
}

/// A source-local hashed set value with a membership-grade surface.
///
/// Equality ignores construction order, as the underlying [`HashSet`] equality does; two sets are equal exactly
/// when they hold the same keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacementSet {
    /// Hashed entries. The container type is the cost-model contract — see the module docs.
    entries: HashSet<HashedKey>,
}

impl ReplacementSet {
    /// Construct a set from a set aggregate's evaluated elements, in evaluation order.
    ///
    /// Refuses the first element outside the hashed key domain: unlike a list, which may hold what membership
    /// later refuses to compare, a hashed container cannot even hold what it cannot hash, so the honest refusal
    /// site is construction — the same place the executor refuses the whole aggregate today. Duplicate elements
    /// collapse, as they do in the language.
    pub fn from_elements(elements: impl IntoIterator<Item = ReplacementValue>) -> Result<Self, NonScalarKey> {
        let entries = elements
            .into_iter()
            .map(HashedKey::try_from_value)
            .collect::<Result<HashSet<_>, _>>()?;
        Ok(Self { entries })
    }

    /// The empty set, as the zero-argument `Set()` constructor builds it: typed by the checker, holding nothing.
    ///
    /// Membership over it answers `false` for any needle the key domain admits — emptiness is an answer, not a
    /// refusal — while a non-scalar needle still refuses, exactly as it would against a populated set.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            entries: HashSet::new(),
        }
    }

    /// Whether the set holds `needle`, by hashed lookup.
    ///
    /// Consumes the evaluated needle to become the probe key without re-allocating a `str`. A needle outside the
    /// key domain refuses rather than answering `false`, even when the set is empty: every held element is already
    /// known comparable, so the needle is the one place "could not tell" could still leak in disguised as
    /// "absent". The negated source operator is the caller's complement of this answer, the same shape the
    /// list-membership arm uses.
    pub fn contains(&self, needle: ReplacementValue) -> Result<bool, NonScalarKey> {
        let key = HashedKey::try_from_value(needle)?;
        Ok(self.entries.contains(&key))
    }

    /// Deterministic source-observable spelling for receipts and debugging.
    ///
    /// Entries render in canonical [`HashedKey`] order — a determinism choice, not source order, which this
    /// representation does not retain. The empty set renders as `Set()`, its only source spelling — the `Set`
    /// collection constructor with no argument — because `{}` spells an empty dict.
    #[must_use]
    pub fn observable_text(&self) -> String {
        if self.entries.is_empty() {
            return "Set()".to_string();
        }
        let mut keys: Vec<&HashedKey> = self.entries.iter().collect();
        keys.sort();
        format!(
            "{{{}}}",
            keys.iter()
                .map(|key| key.observable_text())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// A source-local hashed dict value with a membership-grade surface.
///
/// Entry values are retained so a dict stays a faithful value — `{"a": 1}` and `{"a": 2}` must not compare equal —
/// but membership never consults them: `k in d` asks about keys, matching `HelperOp::DictContainsKey` and the
/// `dict_contains_key` runtime helper it lowers toward. Equality compares key sets and their values, ignoring
/// construction order.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplacementDict {
    /// Hashed key entries with their retained values. The container type is the cost-model contract.
    entries: HashMap<HashedKey, ReplacementValue>,
}

impl ReplacementDict {
    /// Construct a dict from a dict literal's evaluated `key: value` pairs, in entry order.
    ///
    /// A later entry overwrites an earlier one with the same key — the precedence `Rvalue::Dict` documents as a
    /// property of dict construction, which insertion order delivers here. Keys refuse outside the hashed key
    /// domain, at construction, for the reason given on [`ReplacementSet::from_elements`]; values are
    /// unrestricted, because they are stored and compared but never hashed and never consulted by membership.
    pub fn from_entries(
        entries: impl IntoIterator<Item = (ReplacementValue, ReplacementValue)>,
    ) -> Result<Self, NonScalarKey> {
        let mut map = HashMap::new();
        for (key, value) in entries {
            map.insert(HashedKey::try_from_value(key)?, value);
        }
        Ok(Self { entries: map })
    }

    /// The empty dict, as `{}` constructs it: typed by the checker, holding nothing.
    ///
    /// Membership over it answers `false` for any needle the key domain admits, while a non-scalar needle still
    /// refuses, exactly as it would against a populated dict.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Whether the dict has an entry for `needle` — key membership, never value membership — by hashed lookup.
    ///
    /// Consumes the evaluated needle and refuses one outside the key domain, for the reasons given on
    /// [`ReplacementSet::contains`]. A value stored in the dict is not found by this probe unless it is also a
    /// key: `1 in {"a": 1}` is `false`.
    pub fn contains_key(&self, needle: ReplacementValue) -> Result<bool, NonScalarKey> {
        let key = HashedKey::try_from_value(needle)?;
        Ok(self.entries.contains_key(&key))
    }

    /// Deterministic source-observable spelling for receipts and debugging.
    ///
    /// Entries render as `key: value` in canonical [`HashedKey`] order — a determinism choice, not source order,
    /// which this representation does not retain. The empty dict renders as `{}`, matching its literal.
    #[must_use]
    pub fn observable_text(&self) -> String {
        let mut entries: Vec<(&HashedKey, &ReplacementValue)> = self.entries.iter().collect();
        entries.sort_by(|left, right| left.0.cmp(right.0));
        format!(
            "{{{}}}",
            entries
                .iter()
                .map(|(key, value)| format!("{}: {}", key.observable_text(), value.observable_text()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

#[cfg(test)]
mod tests;
