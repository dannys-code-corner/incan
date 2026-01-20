//! Decorator vocabulary registry.
//!
//! This module centralizes recognized decorator spellings so downstream code
//! doesn't need stringly-typed comparisons.

use crate::lang::registry::{LangItemInfo, RFC, RfcId, Since, Stability};

/// Stable identifier for supported decorators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecoratorId {
    Derive,
    Route,
    Fixture,
    Requires,
}

/// Named argument for `@route(methods=[...])`.
pub const ROUTE_METHODS_ARG: &str = "methods";

/// Named argument for `@fixture(scope=...)`.
pub const FIXTURE_SCOPE_ARG: &str = "scope";

/// Named argument for `@fixture(autouse=...)`.
pub const FIXTURE_AUTOUSE_ARG: &str = "autouse";

/// Fixture scope value: per-function.
pub const FIXTURE_SCOPE_FUNCTION: &str = "function";

/// Fixture scope value: per-module.
pub const FIXTURE_SCOPE_MODULE: &str = "module";

/// Fixture scope value: per-session.
pub const FIXTURE_SCOPE_SESSION: &str = "session";

/// Metadata entry for a decorator.
pub type DecoratorInfo = LangItemInfo<DecoratorId>;

/// Registry of supported decorators.
pub const DECORATORS: &[DecoratorInfo] = &[
    info(
        DecoratorId::Derive,
        "derive",
        &[],
        "Derive common trait implementations.",
        RFC::_000,
        Since(0, 1),
    ),
    info(
        DecoratorId::Route,
        "route",
        &[],
        "Declare a web route handler.",
        RFC::_000,
        Since(0, 1),
    ),
    info(
        DecoratorId::Fixture,
        "fixture",
        &[],
        "Declare a test fixture.",
        RFC::_001,
        Since(0, 1),
    ),
    info(
        DecoratorId::Requires,
        "requires",
        &[],
        "Declare required fields for trait default methods.",
        RFC::_000,
        Since(0, 1),
    ),
];

/// Resolve a decorator name to its stable id.
pub fn from_str(name: &str) -> Option<DecoratorId> {
    if let Some(info) = DECORATORS.iter().find(|d| d.canonical == name) {
        return Some(info.id);
    }
    DECORATORS
        .iter()
        .find(|d| {
            let aliases: &[&str] = d.aliases;
            aliases.contains(&name)
        })
        .map(|d| d.id)
}

/// Return the canonical spelling for a decorator.
pub fn as_str(id: DecoratorId) -> &'static str {
    info_for(id).canonical
}

/// Return the metadata entry for a decorator.
pub fn info_for(id: DecoratorId) -> &'static DecoratorInfo {
    DECORATORS.iter().find(|d| d.id == id).expect("decorator info missing")
}

const fn info(
    id: DecoratorId,
    canonical: &'static str,
    aliases: &'static [&'static str],
    description: &'static str,
    introduced_in_rfc: RfcId,
    since: Since,
) -> DecoratorInfo {
    LangItemInfo {
        id,
        canonical,
        aliases,
        description,
        introduced_in_rfc,
        since,
        stability: Stability::Stable,
        examples: &[],
    }
}
