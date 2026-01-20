//! Rust keyword vocabulary (for codegen identifier escaping).

/// Reserved + strict keywords in Rust.
pub const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "super", "trait", "true",
    "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final",
    "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
];

/// Check whether an identifier is a Rust keyword.
pub fn is_keyword(name: &str) -> bool {
    RUST_KEYWORDS.contains(&name)
}
