//! IR expression definitions.
//!
//! These types represent expressions with resolved types and ownership.
//!
//! ## Enum-based dispatch
//!
//! Built-in functions and known methods are represented as enums (`BuiltinFn`, `MethodKind`) rather than
//! stringly-typed names. This enables:
//!
//! - Compile-time exhaustiveness checking in the emitter
//! - Easier refactoring (rename a variant → compiler shows all call sites)
//! - Clear documentation of supported builtins/methods
//!
//! Unknown methods (e.g., Rust interop) remain string-based via `MethodCall`.

use super::{IrSpan, IrType, Ownership};

/// A typed expression in IR
#[derive(Debug, Clone)]
pub struct TypedExpr {
    /// The expression kind
    pub kind: IrExprKind,
    /// Resolved type
    pub ty: IrType,
    /// Ownership semantics (owned, borrowed, etc.)
    pub ownership: Ownership,
    /// Source span for error reporting
    pub span: IrSpan,
}

impl TypedExpr {
    pub fn new(kind: IrExprKind, ty: IrType) -> Self {
        Self {
            kind,
            ty,
            ownership: Ownership::Owned,
            span: IrSpan::default(),
        }
    }

    pub fn with_ownership(mut self, ownership: Ownership) -> Self {
        self.ownership = ownership;
        self
    }

    pub fn with_span(mut self, span: IrSpan) -> Self {
        self.span = span;
        self
    }
}

/// IR expression (alias for TypedExpr for convenience)
pub type IrExpr = TypedExpr;

/// Expression kinds in IR
#[derive(Debug, Clone)]
pub enum IrExprKind {
    // Literals
    Unit,
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),

    // Variable reference
    Var {
        name: String,
        /// Whether this is a move, borrow, or copy
        access: VarAccess,
    },

    // Binary operations
    BinOp {
        op: BinOp,
        left: Box<IrExpr>,
        right: Box<IrExpr>,
    },

    // Unary operations
    UnaryOp {
        op: UnaryOp,
        operand: Box<IrExpr>,
    },

    // Function call (unknown/user-defined function)
    Call {
        func: Box<IrExpr>,
        args: Vec<IrExpr>,
    },

    /// Built-in function call (enum-dispatched).
    ///
    /// Used for known builtins like `print`, `len`, `range`, etc.
    /// The emitter matches on `BuiltinFn` instead of string names.
    BuiltinCall {
        func: BuiltinFn,
        args: Vec<IrExpr>,
    },

    // Method call (unknown/user-defined method)
    MethodCall {
        receiver: Box<IrExpr>,
        method: String,
        args: Vec<IrExpr>,
    },

    /// Known method call (enum-dispatched).
    ///
    /// Used for known methods like `upper`, `append`, `contains`, etc.
    /// The emitter matches on `MethodKind` instead of string names.
    KnownMethodCall {
        receiver: Box<IrExpr>,
        kind: MethodKind,
        args: Vec<IrExpr>,
    },

    // Field access
    Field {
        object: Box<IrExpr>,
        field: String,
    },

    // Index access (list[i], dict[k])
    Index {
        object: Box<IrExpr>,
        index: Box<IrExpr>,
    },

    // Slice access (list[start:end])
    Slice {
        target: Box<IrExpr>,
        start: Option<Box<IrExpr>>,
        end: Option<Box<IrExpr>>,
    },

    // List comprehension
    ListComp {
        element: Box<IrExpr>,
        variable: String,
        iterable: Box<IrExpr>,
        filter: Option<Box<IrExpr>>,
    },
    DictComp {
        key: Box<IrExpr>,
        value: Box<IrExpr>,
        variable: String,
        iterable: Box<IrExpr>,
        filter: Option<Box<IrExpr>>,
    },

    // List literal
    List(Vec<IrExpr>),

    // Dict literal
    Dict(Vec<(IrExpr, IrExpr)>),

    // Set literal
    Set(Vec<IrExpr>),

    // Tuple literal
    Tuple(Vec<IrExpr>),

    // Struct construction
    Struct {
        name: String,
        fields: Vec<(String, IrExpr)>,
    },

    // If expression
    If {
        condition: Box<IrExpr>,
        then_branch: Box<IrExpr>,
        else_branch: Option<Box<IrExpr>>,
    },

    // Match expression
    Match {
        scrutinee: Box<IrExpr>,
        arms: Vec<MatchArm>,
    },

    // Closure
    Closure {
        params: Vec<(String, IrType)>,
        body: Box<IrExpr>,
        captures: Vec<String>,
    },

    // Block expression (sequence of statements with optional trailing expr)
    Block {
        stmts: Vec<super::IrStmt>,
        value: Option<Box<IrExpr>>,
    },

    // Await expression (async)
    Await(Box<IrExpr>),

    // Try/Propogate expression (i.e. the Rust-like `?` operator)
    Try(Box<IrExpr>),

    // Range (start..end)
    Range {
        start: Option<Box<IrExpr>>,
        end: Option<Box<IrExpr>>,
        inclusive: bool,
    },

    // Cast expression (as Type)
    Cast {
        expr: Box<IrExpr>,
        to_type: IrType,
    },

    // Format string (f-string)
    Format {
        parts: Vec<FormatPart>,
    },

    // Literal value (used for generated code)
    Literal(Literal),

    // List of field names for reflection
    FieldsList(Vec<String>),

    // serde_json::to_string(self).unwrap()
    SerdeToJson,

    // serde_json::from_str(s) - contains the target type name
    SerdeFromJson(String),
}

/// Literal values for generated code
#[derive(Debug, Clone)]
pub enum Literal {
    /// Static string literal (&'static str)
    StaticStr(String),
}

/// Part of a format string
#[derive(Debug, Clone)]
pub enum FormatPart {
    /// Literal text
    Literal(String),
    /// Expression to interpolate
    Expr(IrExpr),
}

/// How a variable is accessed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VarAccess {
    /// Move the value (consumes ownership)
    #[default]
    Move,
    /// Borrow immutably (&)
    Borrow,
    /// Borrow mutably (&mut)
    BorrowMut,
    /// Copy the value (for Copy types)
    Copy,
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv, // // (Python-style floor division)
    Mod,
    Pow,

    // Comparison
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,

    // Logical
    And,
    Or,

    // Bitwise
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    Deref,
    Ref,
    RefMut,
}

/// A match arm
#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<IrExpr>,
    pub body: IrExpr,
}

/// Pattern for match expressions
#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard,
    Var(String),
    Literal(IrExpr),
    Tuple(Vec<Pattern>),
    Struct {
        name: String,
        fields: Vec<(String, Pattern)>,
    },
    Enum {
        name: String,
        variant: String,
        fields: Vec<Pattern>,
    },
    Or(Vec<Pattern>),
}

// ============================================================================
// Enum-based dispatch for builtins and methods
// ============================================================================

/// Built-in functions recognized by the Incan compiler.
///
/// These are functions that lower to specific Rust code patterns rather than regular function calls.
/// The emitter matches on this enum instead of string names to avoid stringly-typing.
///
/// ## Adding a new builtin
///
/// 1. Add a variant here
/// 2. Update `BuiltinFn::from_name()` to map the string name
/// 3. Update `emit_builtin_call()` in `expressions/builtins.rs` to emit the Rust code
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinFn {
    /// `print(x)` / `println(x)` → `println!("{}", x)`
    Print,
    /// `len(x)` → `x.len() as i64`
    Len,
    /// `sum(x)` → `x.iter().sum::<i64>()`
    Sum,
    /// `str(x)` → `x.to_string()`
    Str,
    /// `int(x)` → parse or cast to i64
    Int,
    /// `float(x)` → parse or cast to f64
    Float,
    /// `abs(x)` → `x.abs()`
    Abs,
    /// `range(...)` → Rust range expressions
    Range,
    /// `enumerate(x)` → `x.iter().enumerate()`
    Enumerate,
    /// `zip(a, b)` → `a.iter().zip(b.iter())`
    Zip,
    /// `read_file(path)` → `std::fs::read_to_string(path)`
    ReadFile,
    /// `write_file(path, content)` → `std::fs::write(path, content)`
    WriteFile,
    /// `json_stringify(x)` → `serde_json::to_string(&x).unwrap()`
    JsonStringify,
    /// `sleep(secs)` → `tokio::time::sleep(...)`
    Sleep,
}

impl BuiltinFn {
    /// Try to resolve a function name to a known builtin.
    ///
    /// Returns `None` for unknown functions (which are treated as user-defined).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "print" | "println" => Some(Self::Print),
            "len" => Some(Self::Len),
            "sum" => Some(Self::Sum),
            "str" => Some(Self::Str),
            "int" => Some(Self::Int),
            "float" => Some(Self::Float),
            "abs" => Some(Self::Abs),
            "range" => Some(Self::Range),
            "enumerate" => Some(Self::Enumerate),
            "zip" => Some(Self::Zip),
            "read_file" => Some(Self::ReadFile),
            "write_file" => Some(Self::WriteFile),
            "json_stringify" => Some(Self::JsonStringify),
            "sleep" => Some(Self::Sleep),
            _ => None,
        }
    }
}

/// Known method kinds recognized by the Incan compiler.
///
/// These are methods that have special lowering or emit behavior. The emitter
/// matches on this enum instead of string names.
///
/// ## Adding a new method
///
/// 1. Add a variant here
/// 2. Update `MethodKind::from_name()` to map the string name
/// 3. Update `emit_known_method_call()` in `expressions/methods.rs` to emit the Rust code
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodKind {
    // ---- String methods ----
    /// `s.upper()` → `s.to_uppercase()`
    Upper,
    /// `s.lower()` → `s.to_lowercase()`
    Lower,
    /// `s.strip()` → `s.trim().to_string()`
    Strip,
    /// `s.split(sep)` → `s.split(sep).map(...).collect()`
    Split,
    /// `s.replace(old, new)` → `s.replace(old, new)`
    Replace,
    /// `sep.join(items)` → `items.join(sep)`
    Join,
    /// `s.startswith(prefix)` → `s.starts_with(prefix)`
    StartsWith,
    /// `s.endswith(suffix)` → `s.ends_with(suffix)`
    EndsWith,

    // ---- Collection methods ----
    /// `x.contains(item)` → varies by type
    Contains,
    /// `x.get(key)` → `x.get(key)`
    Get,
    /// `x.insert(k, v)` → `x.insert(k, v)`
    Insert,
    /// `x.remove(key)` → `x.remove(key)`
    Remove,

    // ---- List methods ----
    /// `list.append(item)` → `list.push(item)`
    Append,
    /// `list.pop()` → `list.pop().unwrap_or(default)`
    Pop,
    /// `list.swap(i, j)` → `list.swap(i as usize, j as usize)`
    Swap,
    /// `list.reserve(n)` → `list.reserve(n as usize)`
    Reserve,
    /// `list.reserve_exact(n)` → `list.reserve_exact(n as usize)`
    ReserveExact,

    // ---- Internal/special methods ----
    /// `x.__slice__(start, end)` → `x[start..end]`
    Slice,
}

impl MethodKind {
    /// Try to resolve a method name to a known method kind.
    ///
    /// Returns `None` for unknown methods (which pass through as regular method calls).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            // String methods
            "upper" => Some(Self::Upper),
            "lower" => Some(Self::Lower),
            "strip" => Some(Self::Strip),
            "split" => Some(Self::Split),
            "replace" => Some(Self::Replace),
            "join" => Some(Self::Join),
            "startswith" => Some(Self::StartsWith),
            "endswith" => Some(Self::EndsWith),
            // Collection methods
            "contains" => Some(Self::Contains),
            "get" => Some(Self::Get),
            "insert" => Some(Self::Insert),
            "remove" => Some(Self::Remove),
            // List methods
            "append" => Some(Self::Append),
            "pop" => Some(Self::Pop),
            "swap" => Some(Self::Swap),
            "reserve" => Some(Self::Reserve),
            "reserve_exact" => Some(Self::ReserveExact),
            // Internal
            "__slice__" => Some(Self::Slice),
            _ => None,
        }
    }
}
