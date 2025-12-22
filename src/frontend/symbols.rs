//! Symbol table and scope management for Incan
//!
//! Tracks all named entities (types, functions, variables, traits) and their scopes.

use std::collections::HashMap;

use crate::frontend::ast::{Receiver, Span, Type};

/// Unique identifier for symbols
pub type SymbolId = usize;

/// Symbol table managing all named entities
#[derive(Debug, Default)]
pub struct SymbolTable {
    symbols: Vec<Symbol>,
    scopes: Vec<Scope>,
    current_scope: usize,
}

impl SymbolTable {
    pub fn new() -> Self {
        let mut table = Self {
            symbols: Vec::new(),
            scopes: vec![Scope::new(None, ScopeKind::Module)],
            current_scope: 0,
        };

        // Add builtin types
        table.add_builtins();
        table
    }

    fn add_builtins(&mut self) {
        // Builtin types
        let builtin_types = [
            "int", "float", "bool", "str", "bytes", "List", "Dict", "Set", "Tuple", "Option",
            "Result", "Unit",
        ];

        for name in builtin_types {
            self.define(Symbol {
                name: name.to_string(),
                kind: SymbolKind::Type(TypeInfo::Builtin),
                span: Span::default(),
                scope: 0,
            });
        }

        // Builtin traits
        let builtin_traits = [
            "Debug",
            "Display",
            "Eq",
            "PartialEq",
            "Ord",
            "PartialOrd",
            "Hash",
            "Clone",
            "Default",
            "From",
            "Into",
            "TryFrom",
            "TryInto",
            "Iterator",
            "IntoIterator",
            "Error",
        ];

        for name in builtin_traits {
            self.define(Symbol {
                name: name.to_string(),
                kind: SymbolKind::Trait(TraitInfo {
                    type_params: vec![],
                    methods: HashMap::new(),
                    requires: vec![],
                }),
                span: Span::default(),
                scope: 0,
            });
        }

        // Builtin variants for Result and Option
        // Ok(T) and Err(E) for Result
        self.define(Symbol {
            name: "Ok".to_string(),
            kind: SymbolKind::Variant(VariantInfo {
                enum_name: "Result".to_string(),
                fields: vec![ResolvedType::TypeVar("T".to_string())],
            }),
            span: Span::default(),
            scope: 0,
        });
        self.define(Symbol {
            name: "Err".to_string(),
            kind: SymbolKind::Variant(VariantInfo {
                enum_name: "Result".to_string(),
                fields: vec![ResolvedType::TypeVar("E".to_string())],
            }),
            span: Span::default(),
            scope: 0,
        });
        // Some(T) and None for Option
        self.define(Symbol {
            name: "Some".to_string(),
            kind: SymbolKind::Variant(VariantInfo {
                enum_name: "Option".to_string(),
                fields: vec![ResolvedType::TypeVar("T".to_string())],
            }),
            span: Span::default(),
            scope: 0,
        });
        self.define(Symbol {
            name: "None".to_string(),
            kind: SymbolKind::Variant(VariantInfo {
                enum_name: "Option".to_string(),
                fields: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });

        // Builtin functions
        self.define(Symbol {
            name: "println".to_string(),
            kind: SymbolKind::Function(FunctionInfo {
                params: vec![("msg".to_string(), ResolvedType::Str)],
                return_type: ResolvedType::Unit,
                is_async: false,
                type_params: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });
        self.define(Symbol {
            name: "print".to_string(),
            kind: SymbolKind::Function(FunctionInfo {
                params: vec![("msg".to_string(), ResolvedType::Str)],
                return_type: ResolvedType::Unit,
                is_async: false,
                type_params: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });
        self.define(Symbol {
            name: "len".to_string(),
            kind: SymbolKind::Function(FunctionInfo {
                params: vec![("collection".to_string(), ResolvedType::Unknown)],
                return_type: ResolvedType::Int,
                is_async: false,
                type_params: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });
        // Async primitives (exposed as builtins)
        self.define(Symbol {
            name: "sleep".to_string(),
            kind: SymbolKind::Function(FunctionInfo {
                params: vec![("seconds".to_string(), ResolvedType::Float)],
                return_type: ResolvedType::Unit,
                is_async: true,
                type_params: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });
        self.define(Symbol {
            name: "sleep_ms".to_string(),
            kind: SymbolKind::Function(FunctionInfo {
                params: vec![("millis".to_string(), ResolvedType::Int)],
                return_type: ResolvedType::Unit,
                is_async: true,
                type_params: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });
        self.define(Symbol {
            name: "yield_now".to_string(),
            kind: SymbolKind::Function(FunctionInfo {
                params: vec![],
                return_type: ResolvedType::Unit,
                is_async: true,
                type_params: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });
        self.define(Symbol {
            name: "timeout".to_string(),
            kind: SymbolKind::Function(FunctionInfo {
                params: vec![
                    ("seconds".to_string(), ResolvedType::Float),
                    ("task".to_string(), ResolvedType::Unknown),
                ],
                return_type: ResolvedType::Unknown,
                is_async: true,
                type_params: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });
        self.define(Symbol {
            name: "timeout_ms".to_string(),
            kind: SymbolKind::Function(FunctionInfo {
                params: vec![
                    ("millis".to_string(), ResolvedType::Int),
                    ("task".to_string(), ResolvedType::Unknown),
                ],
                return_type: ResolvedType::Unknown,
                is_async: true,
                type_params: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });
        self.define(Symbol {
            name: "spawn".to_string(),
            kind: SymbolKind::Function(FunctionInfo {
                params: vec![("task".to_string(), ResolvedType::Unknown)],
                return_type: ResolvedType::Unknown,
                is_async: true,
                type_params: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });
        self.define(Symbol {
            name: "spawn_blocking".to_string(),
            kind: SymbolKind::Function(FunctionInfo {
                params: vec![("task".to_string(), ResolvedType::Unknown)],
                return_type: ResolvedType::Unknown,
                is_async: true,
                type_params: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });

        // range() builtin - returns an iterator
        self.define(Symbol {
            name: "range".to_string(),
            kind: SymbolKind::Function(FunctionInfo {
                params: vec![("n".to_string(), ResolvedType::Int)],
                return_type: ResolvedType::Named("Range".to_string()), // Iterator-like
                is_async: false,
                type_params: vec![],
            }),
            span: Span::default(),
            scope: 0,
        });
    }

    /// Enter a new scope
    pub fn enter_scope(&mut self, kind: ScopeKind) {
        let new_scope = Scope::new(Some(self.current_scope), kind);
        self.scopes.push(new_scope);
        self.current_scope = self.scopes.len() - 1;
    }

    /// Exit the current scope
    pub fn exit_scope(&mut self) {
        if let Some(parent) = self.scopes[self.current_scope].parent {
            self.current_scope = parent;
        }
    }

    /// Define a new symbol in the current scope
    pub fn define(&mut self, mut symbol: Symbol) -> SymbolId {
        symbol.scope = self.current_scope;
        let id = self.symbols.len();
        self.scopes[self.current_scope]
            .symbols
            .insert(symbol.name.clone(), id);
        self.symbols.push(symbol);
        id
    }

    /// Look up a symbol by name in the current scope chain
    pub fn lookup(&self, name: &str) -> Option<SymbolId> {
        let mut scope_idx = self.current_scope;
        loop {
            if let Some(&id) = self.scopes[scope_idx].symbols.get(name) {
                return Some(id);
            }
            if let Some(parent) = self.scopes[scope_idx].parent {
                scope_idx = parent;
            } else {
                break;
            }
        }
        None
    }

    /// Look up a symbol only in the current scope (no parent lookup)
    pub fn lookup_local(&self, name: &str) -> Option<SymbolId> {
        self.scopes[self.current_scope].symbols.get(name).copied()
    }

    /// Get a symbol by ID
    pub fn get(&self, id: SymbolId) -> Option<&Symbol> {
        self.symbols.get(id)
    }

    /// Get a mutable symbol by ID
    pub fn get_mut(&mut self, id: SymbolId) -> Option<&mut Symbol> {
        self.symbols.get_mut(id)
    }

    /// Get the current scope kind
    pub fn current_scope_kind(&self) -> ScopeKind {
        self.scopes[self.current_scope].kind
    }

    /// Check if we're inside a function/method
    pub fn in_function(&self) -> bool {
        let mut scope_idx = self.current_scope;
        loop {
            match self.scopes[scope_idx].kind {
                ScopeKind::Function | ScopeKind::Method { .. } => return true,
                _ => {}
            }
            if let Some(parent) = self.scopes[scope_idx].parent {
                scope_idx = parent;
            } else {
                break;
            }
        }
        false
    }

    /// Get the current function's return type (if in a function)
    pub fn current_return_type(&self) -> Option<&ResolvedType> {
        let mut scope_idx = self.current_scope;
        loop {
            match &self.scopes[scope_idx].kind {
                ScopeKind::Function | ScopeKind::Method { .. } => {
                    return self.scopes[scope_idx].return_type.as_ref();
                }
                _ => {}
            }
            if let Some(parent) = self.scopes[scope_idx].parent {
                scope_idx = parent;
            } else {
                break;
            }
        }
        None
    }

    /// Set the return type for the current function scope
    pub fn set_return_type(&mut self, ty: ResolvedType) {
        self.scopes[self.current_scope].return_type = Some(ty);
    }
}

/// A scope containing symbol definitions
#[derive(Debug)]
pub struct Scope {
    pub parent: Option<usize>,
    pub kind: ScopeKind,
    pub symbols: HashMap<String, SymbolId>,
    pub return_type: Option<ResolvedType>,
}

impl Scope {
    pub fn new(parent: Option<usize>, kind: ScopeKind) -> Self {
        Self {
            parent,
            kind,
            symbols: HashMap::new(),
            return_type: None,
        }
    }
}

/// Kind of scope
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Module,
    Function,
    Method { receiver: Option<Receiver> },
    Class,
    Model,
    Trait,
    Block,
}

/// A symbol in the symbol table
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub span: Span,
    pub scope: usize,
}

/// Kind of symbol
#[derive(Debug, Clone)]
pub enum SymbolKind {
    /// Variable/binding
    Variable(VariableInfo),
    /// Function
    Function(FunctionInfo),
    /// Type (class, model, newtype, enum, builtin)
    Type(TypeInfo),
    /// Trait
    Trait(TraitInfo),
    /// Module/import
    Module(ModuleInfo),
    /// Enum variant
    Variant(VariantInfo),
    /// Field
    Field(FieldInfo),
    /// Rust crate import (import rust::...)
    RustModule { crate_name: String, path: String },
}

/// Variable information
#[derive(Debug, Clone)]
pub struct VariableInfo {
    pub ty: ResolvedType,
    pub is_mutable: bool,
    pub is_used: bool,
}

/// Function information
#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub params: Vec<(String, ResolvedType)>,
    pub return_type: ResolvedType,
    pub is_async: bool,
    pub type_params: Vec<String>,
}

/// Type information
#[derive(Debug, Clone)]
pub enum TypeInfo {
    Builtin,
    Class(ClassInfo),
    Model(ModelInfo),
    Newtype(NewtypeInfo),
    Enum(EnumInfo),
}

/// Class information
#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub type_params: Vec<String>,
    pub extends: Option<String>,
    pub traits: Vec<String>,
    pub fields: HashMap<String, FieldInfo>,
    pub methods: HashMap<String, MethodInfo>,
}

/// Model information
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub type_params: Vec<String>,
    pub traits: Vec<String>,
    pub fields: HashMap<String, FieldInfo>,
    pub methods: HashMap<String, MethodInfo>,
}

/// Newtype information
#[derive(Debug, Clone)]
pub struct NewtypeInfo {
    pub underlying: ResolvedType,
    pub methods: HashMap<String, MethodInfo>,
}

/// Enum information
#[derive(Debug, Clone)]
pub struct EnumInfo {
    pub type_params: Vec<String>,
    pub variants: Vec<String>,
}

/// Trait information
#[derive(Debug, Clone)]
pub struct TraitInfo {
    pub type_params: Vec<String>,
    pub methods: HashMap<String, MethodInfo>,
    pub requires: Vec<(String, ResolvedType)>, // Required fields
}

/// Module/import information
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub path: Vec<String>,
    pub is_python: bool,
}

/// Variant information
#[derive(Debug, Clone)]
pub struct VariantInfo {
    pub enum_name: String,
    pub fields: Vec<ResolvedType>,
}

/// Field information
#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub ty: ResolvedType,
    pub has_default: bool,
}

/// Method information
#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub receiver: Option<Receiver>,
    pub params: Vec<(String, ResolvedType)>,
    pub return_type: ResolvedType,
    pub is_async: bool,
    pub has_body: bool, // false for abstract methods (...)
}

/// Resolved type (after type checking)
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedType {
    /// Primitive types
    Int,
    Float,
    Bool,
    Str,
    Bytes,
    /// Unit type
    Unit,
    /// Named type (class, model, newtype, enum)
    Named(String),
    /// Generic type with arguments
    Generic(String, Vec<ResolvedType>),
    /// Function type
    Function(Vec<ResolvedType>, Box<ResolvedType>),
    /// Tuple type
    Tuple(Vec<ResolvedType>),
    /// Type variable (for generics)
    TypeVar(String),
    /// Self type (resolved to the implementing type in traits)
    SelfType,
    /// Unknown/error type
    Unknown,
}

impl ResolvedType {
    /// Check if this is a Result type
    pub fn is_result(&self) -> bool {
        matches!(self, ResolvedType::Generic(name, _) if name == "Result")
    }

    /// Check if this is an Option type
    pub fn is_option(&self) -> bool {
        matches!(self, ResolvedType::Generic(name, _) if name == "Option")
    }

    /// Get the Ok type from Result[T, E]
    pub fn result_ok_type(&self) -> Option<&ResolvedType> {
        match self {
            ResolvedType::Generic(name, args) if name == "Result" && !args.is_empty() => {
                Some(&args[0])
            }
            _ => None,
        }
    }

    /// Get the Err type from Result[T, E]
    pub fn result_err_type(&self) -> Option<&ResolvedType> {
        match self {
            ResolvedType::Generic(name, args) if name == "Result" && args.len() >= 2 => {
                Some(&args[1])
            }
            _ => None,
        }
    }

    /// Get the inner type from Option[T]
    pub fn option_inner_type(&self) -> Option<&ResolvedType> {
        match self {
            ResolvedType::Generic(name, args) if name == "Option" && !args.is_empty() => {
                Some(&args[0])
            }
            _ => None,
        }
    }
}

impl std::fmt::Display for ResolvedType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolvedType::Int => write!(f, "int"),
            ResolvedType::Float => write!(f, "float"),
            ResolvedType::Bool => write!(f, "bool"),
            ResolvedType::Str => write!(f, "str"),
            ResolvedType::Bytes => write!(f, "bytes"),
            ResolvedType::Unit => write!(f, "Unit"),
            ResolvedType::Named(name) => write!(f, "{}", name),
            ResolvedType::Generic(name, args) => {
                write!(f, "{}[", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, "]")
            }
            ResolvedType::Function(params, ret) => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            ResolvedType::Tuple(elems) => {
                write!(f, "(")?;
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", e)?;
                }
                write!(f, ")")
            }
            ResolvedType::TypeVar(name) => write!(f, "{}", name),
            ResolvedType::SelfType => write!(f, "Self"),
            ResolvedType::Unknown => write!(f, "?"),
        }
    }
}

/// Convert AST Type to ResolvedType
/// Normalize type name to canonical form (uppercase for built-in generics)
fn normalize_type_name(name: &str) -> String {
    match name {
        // Python-style lowercase → Rust-style uppercase
        "list" => "List".to_string(),
        "dict" => "Dict".to_string(),
        "set" => "Set".to_string(),
        "tuple" => "Tuple".to_string(),
        "option" => "Option".to_string(),
        "result" => "Result".to_string(),
        // Already uppercase or custom types
        _ => name.to_string(),
    }
}

pub fn resolve_type(ty: &Type, symbols: &SymbolTable) -> ResolvedType {
    match ty {
        Type::Simple(name) => match name.as_str() {
            "int" => ResolvedType::Int,
            "float" => ResolvedType::Float,
            "bool" => ResolvedType::Bool,
            "str" => ResolvedType::Str,
            "bytes" => ResolvedType::Bytes,
            "Unit" => ResolvedType::Unit,
            _ => {
                // Check if it's a known type
                if symbols.lookup(name).is_some() {
                    ResolvedType::Named(name.clone())
                } else {
                    // Could be a type variable
                    ResolvedType::TypeVar(name.clone())
                }
            }
        },
        Type::Generic(name, args) => {
            let resolved_args: Vec<_> = args
                .iter()
                .map(|a| resolve_type(&a.node, symbols))
                .collect();
            // Normalize type name for built-in generics (list → List, dict → Dict, etc.)
            let normalized_name = normalize_type_name(name);
            ResolvedType::Generic(normalized_name, resolved_args)
        }
        Type::Function(params, ret) => {
            let resolved_params: Vec<_> = params
                .iter()
                .map(|p| resolve_type(&p.node, symbols))
                .collect();
            let resolved_ret = resolve_type(&ret.node, symbols);
            ResolvedType::Function(resolved_params, Box::new(resolved_ret))
        }
        Type::Unit => ResolvedType::Unit,
        Type::Tuple(elems) => {
            let resolved_elems: Vec<_> = elems
                .iter()
                .map(|e| resolve_type(&e.node, symbols))
                .collect();
            ResolvedType::Tuple(resolved_elems)
        }
        Type::SelfType => ResolvedType::SelfType,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_lookup() {
        let mut table = SymbolTable::new();

        // Define in global scope
        table.define(Symbol {
            name: "x".to_string(),
            kind: SymbolKind::Variable(VariableInfo {
                ty: ResolvedType::Int,
                is_mutable: false,
                is_used: false,
            }),
            span: Span::default(),
            scope: 0,
        });

        // Enter a new scope
        table.enter_scope(ScopeKind::Function);

        // Should still find x
        assert!(table.lookup("x").is_some());

        // Define y in inner scope
        table.define(Symbol {
            name: "y".to_string(),
            kind: SymbolKind::Variable(VariableInfo {
                ty: ResolvedType::Int,
                is_mutable: false,
                is_used: false,
            }),
            span: Span::default(),
            scope: 0,
        });

        assert!(table.lookup("y").is_some());

        // Exit scope
        table.exit_scope();

        // x still visible, y not
        assert!(table.lookup("x").is_some());
        assert!(table.lookup("y").is_none());
    }

    #[test]
    fn test_result_type_helpers() {
        let result_type = ResolvedType::Generic(
            "Result".to_string(),
            vec![
                ResolvedType::Int,
                ResolvedType::Named("AppError".to_string()),
            ],
        );

        assert!(result_type.is_result());
        assert_eq!(result_type.result_ok_type(), Some(&ResolvedType::Int));
        assert_eq!(
            result_type.result_err_type(),
            Some(&ResolvedType::Named("AppError".to_string()))
        );
    }
}
