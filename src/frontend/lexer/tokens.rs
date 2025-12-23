//! Token types for the Incan lexer

use crate::frontend::ast::Span;
use phf::phf_map;

// ============================================================================
// TOKEN TYPES
// ============================================================================

/// Token types for Incan
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // ========== Keywords ==========
    Def,      // function definition
    Async,    // async function
    Await,    // async await expression
    Class,    // class
    Model,    // model
    Trait,    // trait
    Extends,  // extends statement
    Enum,     // enum
    Type,     // type definition
    Newtype,  // newtype definition
    Const,    // const binding (module-level)
    Import,   // python-like import declaration
    RustKw,   // rust keyword (for rust:: imports)
    As,       // import alias keyword
    Python,   // python import keyword (import python "package")  - TODO: figure out if we really need this at all
    From,     // from import declaration
    With,     // with statement
    Return,   // return statement
    If,       // if statement
    Elif,     // elif statement
    Else,     // if-else statement
    While,    // while loop statement
    For,      // for loop statement
    Break,    // break out of a loop statement
    Continue, // continue to the next iteration of a loop statement
    In,       // in keyword
    Match,    // match-case statement
    Case,     // match-case statement
    And,      // logical and operator
    Or,       // logical or operator
    Not,      // logical not operator
    Is,       // is operator
    True,     // true literal
    False,    // false literal
    None,     // none literal
    Let,      // let binding
    Mut,      // mut binding
    SelfKw,   // self keyword
    Pass,     // pass statement
    Pub,      // pub keyword
    Super,    // super (parent module)
    Crate,    // crate (project root)
    Yield,    // yield (for fixtures/generators)

    // ========== Identifiers and Literals ==========
    Ident(String),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    FString(Vec<FStringPart>),

    // ========== Operators ==========
    Plus,         // +
    Minus,        // -
    Star,         // *
    StarStar,     // ** (power)
    Slash,        // /
    SlashSlash,   // // (floor division)
    Percent,      // % (modulo)
    Eq,           // =
    EqEq,         // ==
    NotEq,        // !=
    PlusEq,       // +=
    MinusEq,      // -=
    StarEq,       // *=
    SlashEq,      // /=
    SlashSlashEq, // //=
    PercentEq,    // %=
    Lt,           // <
    Gt,           // >
    LtEq,         // <=
    GtEq,         // >=
    Arrow,        // ->
    FatArrow,     // =>
    Question,     // ?
    Colon,        // :
    ColonColon,   // ::
    Dot,          // .
    DotDot,       // ..
    DotDotEq,     // ..= (inclusive range)
    Comma,        // ,
    At,           // @

    // ========== Brackets ==========
    LParen,   // (
    RParen,   // )
    LBracket, // [
    RBracket, // ]
    LBrace,   // {
    RBrace,   // }

    // ========== Indentation ==========
    Newline,
    Indent,
    Dedent,

    // ========== Special ==========
    Ellipsis, // ...
    Eof,      // end of file
}

/// Part of an f-string
#[derive(Debug, Clone, PartialEq)]
pub enum FStringPart {
    Literal(String),
    Expr(String), // We store the raw expression string; parser will parse it
}

/// Keyword lookup table using perfect hash map for O(1) lookup.
///
/// This maps source text (e.g., `"def"`, `"async"`) to `TokenKind` variants.
/// When the lexer scans an identifier, it checks this map to determine if the text is a keyword or a regular identifier.
///
/// ```ignore
/// let kind = KEYWORDS.get(name).cloned().unwrap_or(TokenKind::Ident(name));
/// ```
///
/// We use `phf` (Perfect Hash Function) for:
/// - O(1) guaranteed lookup (no hash collisions)
/// - Zero runtime initialization cost (computed at compile time)
///
/// The "duplication" between enum variants and map entries is intentional:
/// - enum variants are Rust identifiers
/// - map keys are Incan source text
pub static KEYWORDS: phf::Map<&'static str, TokenKind> = phf_map! {
    "def" => TokenKind::Def,
    "async" => TokenKind::Async,
    "await" => TokenKind::Await,
    "class" => TokenKind::Class,
    "model" => TokenKind::Model,
    "trait" => TokenKind::Trait,
    "enum" => TokenKind::Enum,
    "type" => TokenKind::Type,
    "newtype" => TokenKind::Newtype,
    "const" => TokenKind::Const,
    "import" => TokenKind::Import,
    "as" => TokenKind::As,
    "python" => TokenKind::Python,
    "from" => TokenKind::From,
    "with" => TokenKind::With,
    "extends" => TokenKind::Extends,
    "return" => TokenKind::Return,
    "if" => TokenKind::If,
    "elif" => TokenKind::Elif,
    "else" => TokenKind::Else,
    "while" => TokenKind::While,
    "for" => TokenKind::For,
    "break" => TokenKind::Break,
    "continue" => TokenKind::Continue,
    "in" => TokenKind::In,
    "match" => TokenKind::Match,
    "case" => TokenKind::Case,
    "and" => TokenKind::And,
    "or" => TokenKind::Or,
    "not" => TokenKind::Not,
    "is" => TokenKind::Is,
    "true" => TokenKind::True,
    "True" => TokenKind::True,
    "false" => TokenKind::False,
    "False" => TokenKind::False,
    "None" => TokenKind::None,
    "let" => TokenKind::Let,
    "mut" => TokenKind::Mut,
    "self" => TokenKind::SelfKw,
    "pass" => TokenKind::Pass,
    "pub" => TokenKind::Pub,
    "super" => TokenKind::Super,
    "crate" => TokenKind::Crate,
    "yield" => TokenKind::Yield,
    "rust" => TokenKind::RustKw,
};

/// A token with its kind and span
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}
