//! Type lowering utilities for AST to IR conversion.
//!
//! This module contains helper functions for converting AST types, operators,
//! and performing variable lookups during the lowering pass.

use super::super::expr::BinOp;
use super::super::types::IrType;
use super::AstLowering;
use crate::frontend::ast;
use crate::frontend::symbols::ResolvedType;

impl AstLowering {
    /// Lower an AST type in a `const` context, applying RFC 008 freezing rules.
    ///
    /// Maps container/string annotations to their frozen/static IR equivalents:
    /// - `str` -> `StaticStr`
    /// - `bytes` -> `StaticBytes`
    /// - `List[T]` -> `NamedGeneric("FrozenList", [T])`
    /// - `Dict[K, V]` -> `NamedGeneric("FrozenDict", [K, V])`
    /// - `Set[T]` -> `NamedGeneric("FrozenSet", [T])`
    pub(super) fn lower_const_annotation_type(&self, ty: &ast::Type) -> IrType {
        match ty {
            ast::Type::Simple(name) => match name.as_str() {
                "str" => IrType::StaticStr,
                "bytes" => IrType::StaticBytes,
                // Primitives remain the same
                "int" => IrType::Int,
                "float" => IrType::Float,
                "bool" => IrType::Bool,
                "None" | "Unit" => IrType::Unit,
                _ => {
                    if let Some(enum_ty) = self.enum_names.get(name) {
                        enum_ty.clone()
                    } else {
                        IrType::Struct(name.clone())
                    }
                }
            },
            ast::Type::Generic(base, params) => match base.as_str() {
                "List" | "list" => IrType::NamedGeneric(
                    "FrozenList".to_string(),
                    // For frozen containers, recursively apply const-type lowering to parameters
                    params
                        .iter()
                        .map(|p| self.lower_const_annotation_type(&p.node))
                        .collect(),
                ),
                "Dict" | "dict" | "HashMap" => IrType::NamedGeneric(
                    "FrozenDict".to_string(),
                    params
                        .iter()
                        .map(|p| self.lower_const_annotation_type(&p.node))
                        .collect(),
                ),
                "Set" | "set" => IrType::NamedGeneric(
                    "FrozenSet".to_string(),
                    params
                        .iter()
                        .map(|p| self.lower_const_annotation_type(&p.node))
                        .collect(),
                ),
                // Other generics fall back to regular lowering
                _ => IrType::NamedGeneric(base.clone(), params.iter().map(|p| self.lower_type(&p.node)).collect()),
            },
            // Delegate function/tuple/unit/self handling to regular lowering
            other => self.lower_type(other),
        }
    }
    /// Convert a frontend `ResolvedType` to an IR type.
    ///
    /// This is used when lowering is driven by the typechecker output rather than AST heuristics.
    #[allow(clippy::only_used_in_recursion)]
    pub(super) fn lower_resolved_type(&self, ty: &ResolvedType) -> IrType {
        match ty {
            ResolvedType::Int => IrType::Int,
            ResolvedType::Float => IrType::Float,
            ResolvedType::Bool => IrType::Bool,
            ResolvedType::Str => IrType::String,
            ResolvedType::Bytes => IrType::Unknown,
            ResolvedType::Unit => IrType::Unit,
            ResolvedType::Named(name) => match name.as_str() {
                // RFC 008: Map frozen string/bytes to static forms for const emission
                "FrozenStr" => IrType::StaticStr,
                "FrozenBytes" => IrType::StaticBytes,
                _ => IrType::Struct(name.clone()),
            },
            ResolvedType::Generic(name, args) => match name.as_str() {
                "List" | "list" => IrType::List(Box::new(
                    args.first()
                        .map(|t| self.lower_resolved_type(t))
                        .unwrap_or(IrType::Unknown),
                )),
                "Dict" | "dict" => IrType::Dict(
                    Box::new(
                        args.first()
                            .map(|t| self.lower_resolved_type(t))
                            .unwrap_or(IrType::Unknown),
                    ),
                    Box::new(
                        args.get(1)
                            .map(|t| self.lower_resolved_type(t))
                            .unwrap_or(IrType::Unknown),
                    ),
                ),
                "Set" | "set" => IrType::Set(Box::new(
                    args.first()
                        .map(|t| self.lower_resolved_type(t))
                        .unwrap_or(IrType::Unknown),
                )),
                "Option" | "option" => IrType::Option(Box::new(
                    args.first()
                        .map(|t| self.lower_resolved_type(t))
                        .unwrap_or(IrType::Unknown),
                )),
                "Result" | "result" => IrType::Result(
                    Box::new(
                        args.first()
                            .map(|t| self.lower_resolved_type(t))
                            .unwrap_or(IrType::Unknown),
                    ),
                    Box::new(
                        args.get(1)
                            .map(|t| self.lower_resolved_type(t))
                            .unwrap_or(IrType::Unknown),
                    ),
                ),
                "Tuple" | "tuple" => IrType::Tuple(args.iter().map(|t| self.lower_resolved_type(t)).collect()),
                // RFC 008 frozen types (generic).
                "FrozenList" | "FrozenSet" | "FrozenDict" => {
                    IrType::NamedGeneric(name.clone(), args.iter().map(|t| self.lower_resolved_type(t)).collect())
                }
                _ => IrType::Struct(name.clone()),
            },
            ResolvedType::Function(params, ret) => IrType::Function {
                params: params.iter().map(|p| self.lower_resolved_type(p)).collect(),
                ret: Box::new(self.lower_resolved_type(ret)),
            },
            ResolvedType::Tuple(items) => IrType::Tuple(items.iter().map(|t| self.lower_resolved_type(t)).collect()),
            ResolvedType::TypeVar(name) => IrType::Generic(name.clone()),
            ResolvedType::SelfType => IrType::Unknown,
            ResolvedType::Unknown => IrType::Unknown,
        }
    }

    /// Lower an AST type to an IR type.
    ///
    /// # Parameters
    ///
    /// * `ty` - The AST type to lower
    ///
    /// # Returns
    ///
    /// The corresponding IR type representation.
    pub(super) fn lower_type(&self, ty: &ast::Type) -> IrType {
        match ty {
            ast::Type::Simple(name) => match name.as_str() {
                "int" => IrType::Int,
                "float" => IrType::Float,
                "str" => IrType::String,
                "bool" => IrType::Bool,
                "None" | "Unit" => IrType::Unit,
                _ => {
                    // Check if this is a known enum
                    if let Some(enum_ty) = self.enum_names.get(name) {
                        enum_ty.clone()
                    } else {
                        // Default to struct
                        IrType::Struct(name.clone())
                    }
                }
            },
            ast::Type::Generic(base, params) => match base.as_str() {
                "List" | "list" => IrType::List(Box::new(
                    params
                        .first()
                        .map(|p| self.lower_type(&p.node))
                        .unwrap_or(IrType::Unknown),
                )),
                "Dict" | "dict" | "HashMap" => IrType::Dict(
                    Box::new(
                        params
                            .first()
                            .map(|p| self.lower_type(&p.node))
                            .unwrap_or(IrType::Unknown),
                    ),
                    Box::new(
                        params
                            .get(1)
                            .map(|p| self.lower_type(&p.node))
                            .unwrap_or(IrType::Unknown),
                    ),
                ),
                "Set" | "set" => IrType::Set(Box::new(
                    params
                        .first()
                        .map(|p| self.lower_type(&p.node))
                        .unwrap_or(IrType::Unknown),
                )),
                "Option" | "option" => IrType::Option(Box::new(
                    params
                        .first()
                        .map(|p| self.lower_type(&p.node))
                        .unwrap_or(IrType::Unknown),
                )),
                "Result" | "result" => IrType::Result(
                    Box::new(
                        params
                            .first()
                            .map(|p| self.lower_type(&p.node))
                            .unwrap_or(IrType::Unknown),
                    ),
                    Box::new(
                        params
                            .get(1)
                            .map(|p| self.lower_type(&p.node))
                            .unwrap_or(IrType::Unknown),
                    ),
                ),
                "Tuple" | "tuple" => IrType::Tuple(params.iter().map(|p| self.lower_type(&p.node)).collect()),
                _ => IrType::NamedGeneric(base.clone(), params.iter().map(|p| self.lower_type(&p.node)).collect()),
            },
            ast::Type::Function(params, ret) => IrType::Function {
                params: params.iter().map(|p| self.lower_type(&p.node)).collect(),
                ret: Box::new(self.lower_type(&ret.node)),
            },
            ast::Type::Unit => IrType::Unit,
            ast::Type::Tuple(items) => IrType::Tuple(items.iter().map(|t| self.lower_type(&t.node)).collect()),
            ast::Type::SelfType => IrType::Unknown,
        }
    }

    /// Lower a binary operator from AST to IR.
    ///
    /// # Parameters
    ///
    /// * `op` - The AST binary operator
    ///
    /// # Returns
    ///
    /// The corresponding IR binary operator.
    pub(super) fn lower_binop(&self, op: &ast::BinaryOp) -> BinOp {
        match op {
            ast::BinaryOp::Add => BinOp::Add,
            ast::BinaryOp::Sub => BinOp::Sub,
            ast::BinaryOp::Mul => BinOp::Mul,
            ast::BinaryOp::Div => BinOp::Div,
            ast::BinaryOp::Mod => BinOp::Mod,
            ast::BinaryOp::Pow => BinOp::Pow,
            ast::BinaryOp::Eq => BinOp::Eq,
            ast::BinaryOp::NotEq => BinOp::Ne,
            ast::BinaryOp::Lt => BinOp::Lt,
            ast::BinaryOp::LtEq => BinOp::Le,
            ast::BinaryOp::Gt => BinOp::Gt,
            ast::BinaryOp::GtEq => BinOp::Ge,
            ast::BinaryOp::And => BinOp::And,
            ast::BinaryOp::Or => BinOp::Or,
            ast::BinaryOp::In | ast::BinaryOp::NotIn | ast::BinaryOp::Is => BinOp::Eq,
        }
    }

    /// Lower a compound assignment operator.
    ///
    /// # Parameters
    ///
    /// * `op` - The AST compound assignment operator
    ///
    /// # Returns
    ///
    /// The corresponding IR binary operator (for the operation part).
    pub(super) fn lower_compound_op(&self, op: &ast::CompoundOp) -> BinOp {
        match op {
            ast::CompoundOp::Add => BinOp::Add,
            ast::CompoundOp::Sub => BinOp::Sub,
            ast::CompoundOp::Mul => BinOp::Mul,
            ast::CompoundOp::Div => BinOp::Div,
            ast::CompoundOp::Mod => BinOp::Mod,
        }
    }

    /// Determine the result type of a binary operation.
    ///
    /// # Parameters
    ///
    /// * `left` - The type of the left operand
    /// * `right` - The type of the right operand
    /// * `op` - The binary operator
    ///
    /// # Returns
    ///
    /// The result type of the operation.
    pub(super) fn binary_result_type(&self, left: &IrType, right: &IrType, op: &ast::BinaryOp) -> IrType {
        match op {
            ast::BinaryOp::Eq
            | ast::BinaryOp::NotEq
            | ast::BinaryOp::Lt
            | ast::BinaryOp::LtEq
            | ast::BinaryOp::Gt
            | ast::BinaryOp::GtEq
            | ast::BinaryOp::And
            | ast::BinaryOp::Or
            | ast::BinaryOp::In
            | ast::BinaryOp::NotIn
            | ast::BinaryOp::Is => IrType::Bool,
            ast::BinaryOp::Add
            | ast::BinaryOp::Sub
            | ast::BinaryOp::Mul
            | ast::BinaryOp::Div
            | ast::BinaryOp::Mod
            | ast::BinaryOp::Pow => {
                if matches!(left, IrType::Float) || matches!(right, IrType::Float) {
                    IrType::Float
                } else if matches!(left, IrType::Int) && matches!(right, IrType::Int) {
                    IrType::Int
                } else {
                    left.clone()
                }
            }
        }
    }

    /// Look up a variable type in the current scope chain.
    ///
    /// Searches from innermost to outermost scope.
    ///
    /// # Parameters
    ///
    /// * `name` - The variable name to look up
    ///
    /// # Returns
    ///
    /// The type of the variable, or `IrType::Unknown` if not found.
    pub(super) fn lookup_var(&self, name: &str) -> IrType {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return ty.clone();
            }
        }
        IrType::Unknown
    }
}
