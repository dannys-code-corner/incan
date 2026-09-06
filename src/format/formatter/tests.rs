//! Formatter tests for RFC 081 descriptor-gated embedded fragments (#1022).
//!
//! RFC 081 gives the formatter exactly two modes and forbids a third fallback state: render from the structured
//! artifact, or preserve source verbatim for a DSL that declares itself layout-sensitive. These tests pin both
//! modes and the idempotency the structural mode has to hold.

use std::collections::HashMap;

use incan_syntax::lexer;

use crate::format::FormatConfig;
use crate::format::formatter::Formatter;

type KeywordMap = HashMap<String, Vec<incan_vocab::KeywordRegistration>>;
type SurfaceMap = HashMap<String, Vec<incan_vocab::DslSurface>>;

/// Build keyword and surface maps activating one embedded-fragment descriptor on a declaration body.
fn fixture_maps(
    keyword: &str,
    provider: &str,
    submode: incan_vocab::EmbeddedFragmentSubmode,
    layout_sensitive: bool,
) -> (KeywordMap, SurfaceMap) {
    let namespace = format!("{provider}.{keyword}");
    let mut keyword_map = KeywordMap::new();
    keyword_map.insert(
        provider.to_string(),
        vec![incan_vocab::KeywordRegistration {
            activation: incan_vocab::KeywordActivation::OnImport {
                namespace: namespace.clone(),
            },
            keywords: vec![incan_vocab::KeywordSpec::block(keyword)],
            valid_decorators: Vec::new(),
        }],
    );

    let mut descriptor =
        incan_vocab::EmbeddedFragmentDescriptor::new(&format!("{keyword}.fragment"), submode, "fragment")
            .in_declaration_body(keyword);
    if layout_sensitive {
        descriptor = descriptor.layout_sensitive();
    }

    let mut surface_map = SurfaceMap::new();
    surface_map.insert(
        provider.to_string(),
        vec![
            incan_vocab::DslSurface::on_import(&namespace)
                .with_declaration(incan_vocab::DeclarationSurface::named(keyword))
                .with_embedded_fragment(descriptor),
        ],
    );
    (keyword_map, surface_map)
}

/// Parse through the one entrypoint that produces embedded fragments, then format the result.
///
/// `parse_with_source` is the only parser entrypoint that threads original source through, which is what an
/// embedded fragment's submode re-tokenization needs; the ordinary entrypoints are fragment-blind and would make
/// this test silently assert on a program that has no fragment in it at all.
fn format_fixture(source: &str, keyword_map: &KeywordMap, surface_map: &SurfaceMap) -> Result<String, String> {
    let (tokens, _lex_errors) = lexer::lex_tolerant(source);
    let program = incan_syntax::parser::parse_with_source(&tokens, None, Some(keyword_map), Some(surface_map), source)
        .map_err(|errs| format!("parse errors: {errs:?}"))?;
    Ok(Formatter::new(FormatConfig::default()).format(&program))
}

#[test]
fn a_layout_sensitive_descriptor_keeps_its_fragment_verbatim() -> Result<(), String> {
    // A DSL that declares itself layout-sensitive owns its own whitespace, so the formatter must not restructure
    // it. This is one of RFC 081's two modes, and the descriptor is what selects it.
    let (keyword_map, surface_map) =
        fixture_maps("raw", "textkit", incan_vocab::EmbeddedFragmentSubmode::RawText, true);
    let source =
        "import pub::textkit\n\ndef banner() -> None:\n    raw:\n        line one\n           deliberately indented\n";
    let formatted = format_fixture(source, &keyword_map, &surface_map)?;

    assert!(
        formatted.contains("           deliberately indented"),
        "layout-sensitive source must survive untouched:\n{formatted}"
    );
    Ok(())
}

#[test]
fn a_structural_markup_fragment_is_formatted_from_its_nodes() -> Result<(), String> {
    // The other mode: a descriptor that does not declare layout sensitivity gets rendered from the structural
    // tree. Ragged source indentation is layout, so it is re-derived rather than reproduced.
    let (keyword_map, surface_map) =
        fixture_maps("html", "webkit", incan_vocab::EmbeddedFragmentSubmode::Markup, false);
    let source = "import pub::webkit\n\ndef render(title: str) -> None:\n    html:\n        <section class=\"card\">\n              <h1>{title}</h1>\n        </section>\n";
    let formatted = format_fixture(source, &keyword_map, &surface_map)?;

    assert!(
        formatted.contains("<section class=\"card\">"),
        "the element and its literal attribute should render structurally:\n{formatted}"
    );
    assert!(
        formatted.contains("<h1>{title}</h1>"),
        "an inline child keeps its text and hole on one line:\n{formatted}"
    );
    assert!(
        !formatted.contains("              <h1>"),
        "the source's ragged indentation is layout and should not survive:\n{formatted}"
    );
    Ok(())
}

#[test]
fn structural_fragment_formatting_is_idempotent() -> Result<(), String> {
    // Formatting has to be a fixed point: whitespace the first pass introduces becomes text nodes on re-parse, so
    // a renderer that reproduced them would drift further on every pass. This is the property that makes the
    // structural mode safe to run in `incan fmt --check`.
    let (keyword_map, surface_map) =
        fixture_maps("html", "webkit", incan_vocab::EmbeddedFragmentSubmode::Markup, false);
    let source = "import pub::webkit\n\ndef render(title: str, body: str) -> None:\n    html:\n        <section>\n          <h1>{title}</h1>\n          <p>Body: {body}</p>\n        </section>\n";

    let once = format_fixture(source, &keyword_map, &surface_map)?;
    let twice = format_fixture(&once, &keyword_map, &surface_map)?;
    assert_eq!(
        once, twice,
        "second pass changed the output:\n--- once ---\n{once}\n--- twice ---\n{twice}"
    );
    Ok(())
}

#[test]
fn a_structural_style_fragment_renders_rules_and_declarations() -> Result<(), String> {
    // The style submode nests declarations under a rule rather than children under an element, so it exercises a
    // different branch of the structural renderer than markup does.
    let (keyword_map, surface_map) =
        fixture_maps("css", "styleforge", incan_vocab::EmbeddedFragmentSubmode::Style, false);
    let source = "import pub::styleforge\n\ndef theme(accent: str) -> None:\n    css:\n        .card {\n            color: {accent};\n        }\n";

    let once = format_fixture(source, &keyword_map, &surface_map)?;
    assert!(
        once.contains(".card {"),
        "the selector and block opener should render:\n{once}"
    );
    assert!(
        once.contains("color: {accent};"),
        "a declaration with a hole value should render with its semicolon:\n{once}"
    );

    let twice = format_fixture(&once, &keyword_map, &surface_map)?;
    assert_eq!(once, twice, "style formatting should be a fixed point:\n{once}");
    Ok(())
}
