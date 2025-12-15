//! Helper functions for code generation
//!
//! Shared utilities used by multiple codegen modules.

use crate::frontend::ast::*;

/// Check if a variable is used in an expression
/// 
/// Recursively traverses the expression tree to determine if the given variable name
/// appears anywhere in the expression. This is used to suppress unused variable warnings
/// when a loop/comprehension variable is actually used in the body/filter/map expression.
pub(super) fn is_var_used_in_expr(var_name: &str, expr: &Expr) -> bool {
    match expr {
        // Simple identifier: direct string comparison
        Expr::Ident(name) => name == var_name,
        
        // Binary operations (e.g., x + y, a * b): check both operands
        Expr::Binary(left, _, right) => {
            is_var_used_in_expr(var_name, &left.node) ||
            is_var_used_in_expr(var_name, &right.node)
        }
        
        // Unary operations (e.g., -x, not y): check the inner expression
        Expr::Unary(_, e) => is_var_used_in_expr(var_name, &e.node),
        
        // Function calls (e.g., foo(x, y)): check the callee and all arguments
        Expr::Call(callee, args) => {
            is_var_used_in_expr(var_name, &callee.node) ||
            args.iter().any(|arg| match arg {
                CallArg::Positional(e) => is_var_used_in_expr(var_name, &e.node),
                CallArg::Named(_, e) => is_var_used_in_expr(var_name, &e.node),
            })
        }
        
        // Method calls (e.g., obj.method(x)): check the object and all arguments
        Expr::MethodCall(obj, _, args) => {
            is_var_used_in_expr(var_name, &obj.node) ||
            args.iter().any(|arg| match arg {
                CallArg::Positional(e) => is_var_used_in_expr(var_name, &e.node),
                CallArg::Named(_, e) => is_var_used_in_expr(var_name, &e.node),
            })
        }
        
        // Index access (e.g., arr[i]): check both the base and the index expression
        Expr::Index(base, index) => {
            is_var_used_in_expr(var_name, &base.node) ||
            is_var_used_in_expr(var_name, &index.node)
        }
        
        // Field access (e.g., obj.field): only check the base object
        Expr::Field(base, _) => is_var_used_in_expr(var_name, &base.node),
        
        // List literals (e.g., [x, y, z]): check if variable appears in any element
        Expr::List(items) => items.iter().any(|e| is_var_used_in_expr(var_name, &e.node)),
        
        // Dict literals (e.g., {k: v}): check both keys and values
        Expr::Dict(pairs) => pairs.iter().any(|(k, v)| {
            is_var_used_in_expr(var_name, &k.node) ||
            is_var_used_in_expr(var_name, &v.node)
        }),
        
        // Range expressions (e.g., start..end): check both bounds
        Expr::Range { start, end, inclusive: _ } => {
            is_var_used_in_expr(var_name, &start.node) ||
            is_var_used_in_expr(var_name, &end.node)
        }
        
        // F-strings (e.g., f"hello {x}"): check interpolated expressions
        Expr::FString(parts) => parts.iter().any(|part| match part {
            FStringPart::Literal(_) => false,
            FStringPart::Expr(e) => is_var_used_in_expr(var_name, &e.node),
        }),
        
        // If expressions: check condition, conservatively assume variable is used in branches
        // (branches contain statements, which would require more complex analysis)
        Expr::If(if_expr) => {
            is_var_used_in_expr(var_name, &if_expr.condition.node) || true
        }
        
        // Literals (numbers, strings, booleans, None) don't reference variables
        _ => false,
    }
}
