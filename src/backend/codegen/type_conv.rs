//! Type conversion utilities for code generation
//!
//! Handles converting Incan types to Rust types and formatting parameters.

use crate::frontend::ast::*;
use crate::backend::rust_emitter::{incan_type_to_rust, to_rust_ident};

use super::RustCodegen;

impl RustCodegen<'_> {
    /// Convert an Incan type to Rust type string
    pub(crate) fn type_to_rust(&self, ty: &Type) -> String {
        Self::type_to_rust_static(ty)
    }

    /// Convert an Incan type to Rust type string (static version)
    pub(crate) fn type_to_rust_static(ty: &Type) -> String {
        match ty {
            Type::Simple(name) => incan_type_to_rust(name),
            Type::Generic(name, args) => {
                let rust_args: Vec<String> = args
                    .iter()
                    .map(|a| Self::type_to_rust_static(&a.node))
                    .collect();
                let base = incan_type_to_rust(name);
                // Handle special cases
                match name.as_str() {
                    "List" => format!("Vec<{}>", rust_args.join(", ")),
                    "Dict" => format!("HashMap<{}>", rust_args.join(", ")),
                    "Set" => format!("HashSet<{}>", rust_args.join(", ")),
                    // Sync primitives are Arc-wrapped for sharing between tasks
                    "Mutex" => format!("std::sync::Arc<tokio::sync::Mutex<{}>>", rust_args.join(", ")),
                    "RwLock" => format!("std::sync::Arc<tokio::sync::RwLock<{}>>", rust_args.join(", ")),
                    _ => format!("{}<{}>", base, rust_args.join(", ")),
                }
            }
            Type::Function(params, ret) => {
                let param_types: Vec<String> = params
                    .iter()
                    .map(|p| Self::type_to_rust_static(&p.node))
                    .collect();
                let ret_type = Self::type_to_rust_static(&ret.node);
                format!("fn({}) -> {}", param_types.join(", "), ret_type)
            }
            Type::Unit => "()".to_string(),
            Type::Tuple(elems) => {
                let elem_types: Vec<String> = elems
                    .iter()
                    .map(|e| Self::type_to_rust_static(&e.node))
                    .collect();
                format!("({})", elem_types.join(", "))
            }
            Type::SelfType => "Self".to_string(),
        }
    }

    /// Convert binary operator to Rust equivalent
    pub(crate) fn binary_op_to_rust(op: BinaryOp) -> &'static str {
        match op {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Eq => "==",
            BinaryOp::NotEq => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Gt => ">",
            BinaryOp::LtEq => "<=",
            BinaryOp::GtEq => ">=",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
            BinaryOp::In => ".contains(&", // Special handling needed
            BinaryOp::NotIn => "!.contains(&", // Special handling needed
            BinaryOp::Is => "==", // Simplified
        }
    }

    /// Format method parameters (with optional receiver)
    pub(crate) fn format_params(receiver: &Option<Receiver>, params: &[Spanned<Param>]) -> String {
        let mut parts = Vec::new();

        if let Some(recv) = receiver {
            match recv {
                Receiver::Immutable => parts.push("&self".to_string()),
                Receiver::Mutable => parts.push("&mut self".to_string()),
            }
        }

        for p in params {
            let ty = Self::type_to_rust_static(&p.node.ty.node);
            parts.push(format!("{}: {}", to_rust_ident(&p.node.name), ty));
        }

        parts.join(", ")
    }

    /// Format function parameters (no receiver)
    pub(crate) fn format_function_params(params: &[Spanned<Param>]) -> String {
        params
            .iter()
            .map(|p| {
                let ty = Self::type_to_rust_static(&p.node.ty.node);
                format!("{}: {}", to_rust_ident(&p.node.name), ty)
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Convert Incan derive name to Rust derives, returning all required derives
    pub(crate) fn derive_to_rust_vec(&self, name: &str) -> Vec<&'static str> {
        match name {
            // String representation
            "Debug" => vec!["Debug"],
            "Display" => vec!["Display"],

            // Comparison - Eq requires PartialEq, Ord requires all four
            "Eq" => vec!["Eq", "PartialEq"],
            "Ord" => vec!["Ord", "PartialOrd", "Eq", "PartialEq"],

            // Hashing
            "Hash" => vec!["Hash"],

            // Copying - Copy requires Clone
            "Clone" => vec!["Clone"],
            "Copy" => vec!["Copy", "Clone"],

            // Default values
            "Default" => vec!["Default"],

            // Serialization (JSON support)
            "Serialize" => vec!["serde::Serialize"],
            "Deserialize" => vec!["serde::Deserialize"],

            // Legacy/special
            "Validate" => vec!["Debug"],

            _ => vec![], // Unknown derives are handled separately
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::codegen::RustCodegen;

    fn make_spanned<T>(node: T) -> Spanned<T> {
        Spanned { node, span: Span::default() }
    }

    // ========================================
    // type_to_rust_static tests
    // ========================================

    #[test]
    fn test_type_simple_int() {
        let ty = Type::Simple("int".to_string());
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "i64");
    }

    #[test]
    fn test_type_simple_str() {
        let ty = Type::Simple("str".to_string());
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "String");
    }

    #[test]
    fn test_type_simple_bool() {
        let ty = Type::Simple("bool".to_string());
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "bool");
    }

    #[test]
    fn test_type_simple_float() {
        let ty = Type::Simple("float".to_string());
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "f64");
    }

    #[test]
    fn test_type_simple_bytes() {
        let ty = Type::Simple("bytes".to_string());
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "Vec<u8>");
    }

    #[test]
    fn test_type_simple_custom() {
        let ty = Type::Simple("MyType".to_string());
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "MyType");
    }

    #[test]
    fn test_type_generic_list() {
        let ty = Type::Generic(
            "List".to_string(),
            vec![make_spanned(Type::Simple("int".to_string()))],
        );
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "Vec<i64>");
    }

    #[test]
    fn test_type_generic_dict() {
        let ty = Type::Generic(
            "Dict".to_string(),
            vec![
                make_spanned(Type::Simple("str".to_string())),
                make_spanned(Type::Simple("int".to_string())),
            ],
        );
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "HashMap<String, i64>");
    }

    #[test]
    fn test_type_generic_set() {
        let ty = Type::Generic(
            "Set".to_string(),
            vec![make_spanned(Type::Simple("int".to_string()))],
        );
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "HashSet<i64>");
    }

    #[test]
    fn test_type_generic_option() {
        let ty = Type::Generic(
            "Option".to_string(),
            vec![make_spanned(Type::Simple("str".to_string()))],
        );
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "Option<String>");
    }

    #[test]
    fn test_type_generic_result() {
        let ty = Type::Generic(
            "Result".to_string(),
            vec![
                make_spanned(Type::Simple("int".to_string())),
                make_spanned(Type::Simple("str".to_string())),
            ],
        );
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "Result<i64, String>");
    }

    #[test]
    fn test_type_function() {
        let ty = Type::Function(
            vec![
                make_spanned(Type::Simple("int".to_string())),
                make_spanned(Type::Simple("str".to_string())),
            ],
            Box::new(make_spanned(Type::Simple("bool".to_string()))),
        );
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "fn(i64, String) -> bool");
    }

    #[test]
    fn test_type_function_no_params() {
        let ty = Type::Function(
            vec![],
            Box::new(make_spanned(Type::Unit)),
        );
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "fn() -> ()");
    }

    #[test]
    fn test_type_unit() {
        let ty = Type::Unit;
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "()");
    }

    #[test]
    fn test_type_tuple() {
        let ty = Type::Tuple(vec![
            make_spanned(Type::Simple("int".to_string())),
            make_spanned(Type::Simple("str".to_string())),
        ]);
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "(i64, String)");
    }

    #[test]
    fn test_type_tuple_single() {
        let ty = Type::Tuple(vec![make_spanned(Type::Simple("int".to_string()))]);
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "(i64)");
    }

    #[test]
    fn test_type_self() {
        let ty = Type::SelfType;
        assert_eq!(RustCodegen::type_to_rust_static(&ty), "Self");
    }

    // ========================================
    // binary_op_to_rust tests
    // ========================================

    #[test]
    fn test_binary_op_add() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::Add), "+");
    }

    #[test]
    fn test_binary_op_sub() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::Sub), "-");
    }

    #[test]
    fn test_binary_op_mul() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::Mul), "*");
    }

    #[test]
    fn test_binary_op_div() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::Div), "/");
    }

    #[test]
    fn test_binary_op_mod() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::Mod), "%");
    }

    #[test]
    fn test_binary_op_eq() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::Eq), "==");
    }

    #[test]
    fn test_binary_op_not_eq() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::NotEq), "!=");
    }

    #[test]
    fn test_binary_op_lt() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::Lt), "<");
    }

    #[test]
    fn test_binary_op_gt() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::Gt), ">");
    }

    #[test]
    fn test_binary_op_lt_eq() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::LtEq), "<=");
    }

    #[test]
    fn test_binary_op_gt_eq() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::GtEq), ">=");
    }

    #[test]
    fn test_binary_op_and() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::And), "&&");
    }

    #[test]
    fn test_binary_op_or() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::Or), "||");
    }

    #[test]
    fn test_binary_op_in() {
        let op = RustCodegen::binary_op_to_rust(BinaryOp::In);
        assert!(op.contains("contains"));
    }

    #[test]
    fn test_binary_op_not_in() {
        let op = RustCodegen::binary_op_to_rust(BinaryOp::NotIn);
        assert!(op.contains("contains"));
    }

    #[test]
    fn test_binary_op_is() {
        assert_eq!(RustCodegen::binary_op_to_rust(BinaryOp::Is), "==");
    }

    // ========================================
    // format_params tests
    // ========================================

    #[test]
    fn test_format_params_no_receiver_no_params() {
        let params: Vec<Spanned<Param>> = vec![];
        let result = RustCodegen::format_params(&None, &params);
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_params_immutable_receiver() {
        let params: Vec<Spanned<Param>> = vec![];
        let result = RustCodegen::format_params(&Some(Receiver::Immutable), &params);
        assert_eq!(result, "&self");
    }

    #[test]
    fn test_format_params_mutable_receiver() {
        let params: Vec<Spanned<Param>> = vec![];
        let result = RustCodegen::format_params(&Some(Receiver::Mutable), &params);
        assert_eq!(result, "&mut self");
    }

    #[test]
    fn test_format_params_with_params() {
        let params = vec![
            make_spanned(Param {
                name: "x".to_string(),
                ty: make_spanned(Type::Simple("int".to_string())),
                default: None,
            }),
            make_spanned(Param {
                name: "y".to_string(),
                ty: make_spanned(Type::Simple("str".to_string())),
                default: None,
            }),
        ];
        let result = RustCodegen::format_params(&None, &params);
        assert_eq!(result, "x: i64, y: String");
    }

    #[test]
    fn test_format_params_receiver_with_params() {
        let params = vec![
            make_spanned(Param {
                name: "value".to_string(),
                ty: make_spanned(Type::Simple("int".to_string())),
                default: None,
            }),
        ];
        let result = RustCodegen::format_params(&Some(Receiver::Immutable), &params);
        assert_eq!(result, "&self, value: i64");
    }

    // ========================================
    // format_function_params tests
    // ========================================

    #[test]
    fn test_format_function_params_empty() {
        let params: Vec<Spanned<Param>> = vec![];
        let result = RustCodegen::format_function_params(&params);
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_function_params_single() {
        let params = vec![
            make_spanned(Param {
                name: "n".to_string(),
                ty: make_spanned(Type::Simple("int".to_string())),
                default: None,
            }),
        ];
        let result = RustCodegen::format_function_params(&params);
        assert_eq!(result, "n: i64");
    }

    #[test]
    fn test_format_function_params_multiple() {
        let params = vec![
            make_spanned(Param {
                name: "a".to_string(),
                ty: make_spanned(Type::Simple("int".to_string())),
                default: None,
            }),
            make_spanned(Param {
                name: "b".to_string(),
                ty: make_spanned(Type::Simple("float".to_string())),
                default: None,
            }),
            make_spanned(Param {
                name: "c".to_string(),
                ty: make_spanned(Type::Simple("bool".to_string())),
                default: None,
            }),
        ];
        let result = RustCodegen::format_function_params(&params);
        assert_eq!(result, "a: i64, b: f64, c: bool");
    }

    #[test]
    fn test_format_function_params_reserved_name() {
        let params = vec![
            make_spanned(Param {
                name: "type".to_string(),
                ty: make_spanned(Type::Simple("str".to_string())),
                default: None,
            }),
        ];
        let result = RustCodegen::format_function_params(&params);
        assert_eq!(result, "r#type: String");
    }

    // ========================================
    // derive_to_rust_vec tests (more cases)
    // ========================================

    #[test]
    fn test_derive_debug() {
        let codegen = RustCodegen::new();
        assert_eq!(codegen.derive_to_rust_vec("Debug"), vec!["Debug"]);
    }

    #[test]
    fn test_derive_display() {
        let codegen = RustCodegen::new();
        assert_eq!(codegen.derive_to_rust_vec("Display"), vec!["Display"]);
    }

    #[test]
    fn test_derive_clone() {
        let codegen = RustCodegen::new();
        assert_eq!(codegen.derive_to_rust_vec("Clone"), vec!["Clone"]);
    }

    #[test]
    fn test_derive_copy() {
        let codegen = RustCodegen::new();
        let derives = codegen.derive_to_rust_vec("Copy");
        assert!(derives.contains(&"Copy"));
        assert!(derives.contains(&"Clone"));
    }

    #[test]
    fn test_derive_validate() {
        let codegen = RustCodegen::new();
        assert_eq!(codegen.derive_to_rust_vec("Validate"), vec!["Debug"]);
    }

    #[test]
    fn test_derive_unknown() {
        let codegen = RustCodegen::new();
        assert!(codegen.derive_to_rust_vec("UnknownDerive").is_empty());
    }
}
