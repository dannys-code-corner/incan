//! Pattern matching emission for code generation
//!
//! Handles emitting match arms and patterns to Rust.

use crate::frontend::ast::*;
use crate::backend::rust_emitter::RustEmitter;

use super::RustCodegen;

impl RustCodegen<'_> {
    /// Emit a match arm
    pub(crate) fn emit_match_arm(emitter: &mut RustEmitter, arm: &MatchArm) {
        emitter.write("    ");
        Self::emit_pattern(emitter, &arm.pattern.node);

        // Emit guard if present: `pattern if guard =>`
        if let Some(guard) = &arm.guard {
            emitter.write(" if ");
            Self::emit_expr(emitter, &guard.node);
        }

        emitter.write(" => ");
        match &arm.body {
            MatchBody::Expr(e) => {
                Self::emit_expr(emitter, &e.node);
                emitter.write(",\n");
            }
            MatchBody::Block(stmts) => {
                emitter.write("{\n");
                for stmt in stmts {
                    emitter.write("        ");
                    Self::emit_statement(emitter, &stmt.node);
                }
                emitter.write("    },\n");
            }
        }
    }

    /// Emit a pattern
    pub(crate) fn emit_pattern(emitter: &mut RustEmitter, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard => emitter.write("_"),
            Pattern::Binding(name) => emitter.write(&crate::backend::rust_emitter::to_rust_ident(name)),
            Pattern::Literal(lit) => Self::emit_literal(emitter, lit),
            Pattern::Constructor(name, pats) => {
                emitter.write(name);
                if !pats.is_empty() {
                    emitter.write("(");
                    for (i, p) in pats.iter().enumerate() {
                        if i > 0 {
                            emitter.write(", ");
                        }
                        Self::emit_pattern(emitter, &p.node);
                    }
                    emitter.write(")");
                }
            }
            Pattern::Tuple(pats) => {
                emitter.write("(");
                for (i, p) in pats.iter().enumerate() {
                    if i > 0 {
                        emitter.write(", ");
                    }
                    Self::emit_pattern(emitter, &p.node);
                }
                emitter.write(")");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rust_emitter::RustEmitter;
    use crate::frontend::ast::{BinaryOp, Expr, Literal, MatchArm, MatchBody, Pattern, Span, Spanned, Statement};

    fn make_spanned<T>(node: T) -> Spanned<T> {
        Spanned { node, span: Span::default() }
    }

    // ========================================
    // Pattern tests
    // ========================================

    #[test]
    fn test_emit_pattern_wildcard() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Wildcard);
        assert_eq!(emitter.finish(), "_");
    }

    #[test]
    fn test_emit_pattern_binding_simple() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Binding("x".to_string()));
        assert_eq!(emitter.finish(), "x");
    }

    #[test]
    fn test_emit_pattern_binding_reserved_word() {
        let mut emitter = RustEmitter::new();
        // "type" is a reserved word in Rust
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Binding("type".to_string()));
        assert_eq!(emitter.finish(), "r#type");
    }

    #[test]
    fn test_emit_pattern_literal_int() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Literal(Literal::Int(42)));
        assert_eq!(emitter.finish(), "42");
    }

    #[test]
    fn test_emit_pattern_literal_string() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Literal(Literal::String("hello".to_string())));
        assert_eq!(emitter.finish(), "\"hello\"");
    }

    #[test]
    fn test_emit_pattern_literal_bool_true() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Literal(Literal::Bool(true)));
        assert_eq!(emitter.finish(), "true");
    }

    #[test]
    fn test_emit_pattern_literal_bool_false() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Literal(Literal::Bool(false)));
        assert_eq!(emitter.finish(), "false");
    }

    #[test]
    fn test_emit_pattern_literal_none() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Literal(Literal::None));
        assert_eq!(emitter.finish(), "None");
    }

    #[test]
    fn test_emit_pattern_literal_float() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Literal(Literal::Float(3.14)));
        let output = emitter.finish();
        assert!(output.starts_with("3.14"));
    }

    #[test]
    fn test_emit_pattern_constructor_without_args() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Constructor("None".to_string(), vec![]));
        assert_eq!(emitter.finish(), "None");
    }

    #[test]
    fn test_emit_pattern_constructor_with_one_arg() {
        let mut emitter = RustEmitter::new();
        let inner = make_spanned(Pattern::Binding("x".to_string()));
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Constructor("Some".to_string(), vec![inner]));
        assert_eq!(emitter.finish(), "Some(x)");
    }

    #[test]
    fn test_emit_pattern_constructor_with_multiple_args() {
        let mut emitter = RustEmitter::new();
        let arg1 = make_spanned(Pattern::Binding("a".to_string()));
        let arg2 = make_spanned(Pattern::Binding("b".to_string()));
        let arg3 = make_spanned(Pattern::Literal(Literal::Int(42)));
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Constructor("MyVariant".to_string(), vec![arg1, arg2, arg3]));
        assert_eq!(emitter.finish(), "MyVariant(a, b, 42)");
    }

    #[test]
    fn test_emit_pattern_constructor_nested() {
        let mut emitter = RustEmitter::new();
        let inner_pattern = make_spanned(Pattern::Constructor("Inner".to_string(), vec![
            make_spanned(Pattern::Binding("x".to_string()))
        ]));
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Constructor("Outer".to_string(), vec![inner_pattern]));
        assert_eq!(emitter.finish(), "Outer(Inner(x))");
    }

    #[test]
    fn test_emit_pattern_tuple_empty() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Tuple(vec![]));
        assert_eq!(emitter.finish(), "()");
    }

    #[test]
    fn test_emit_pattern_tuple_single() {
        let mut emitter = RustEmitter::new();
        let elem = make_spanned(Pattern::Binding("x".to_string()));
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Tuple(vec![elem]));
        assert_eq!(emitter.finish(), "(x)");
    }

    #[test]
    fn test_emit_pattern_tuple_multiple() {
        let mut emitter = RustEmitter::new();
        let elem1 = make_spanned(Pattern::Binding("a".to_string()));
        let elem2 = make_spanned(Pattern::Binding("b".to_string()));
        let elem3 = make_spanned(Pattern::Wildcard);
        RustCodegen::emit_pattern(&mut emitter, &Pattern::Tuple(vec![elem1, elem2, elem3]));
        assert_eq!(emitter.finish(), "(a, b, _)");
    }

    #[test]
    fn test_emit_pattern_tuple_nested() {
        let mut emitter = RustEmitter::new();
        let inner_tuple = make_spanned(Pattern::Tuple(vec![
            make_spanned(Pattern::Binding("x".to_string())),
            make_spanned(Pattern::Binding("y".to_string())),
        ]));
        let outer = Pattern::Tuple(vec![inner_tuple, make_spanned(Pattern::Wildcard)]);
        RustCodegen::emit_pattern(&mut emitter, &outer);
        assert_eq!(emitter.finish(), "((x, y), _)");
    }

    #[test]
    fn test_emit_pattern_complex_mixed() {
        // Test a complex pattern: (Some(x), _, 42)
        let mut emitter = RustEmitter::new();
        let some_pattern = make_spanned(Pattern::Constructor(
            "Some".to_string(),
            vec![make_spanned(Pattern::Binding("x".to_string()))]
        ));
        let tuple = Pattern::Tuple(vec![
            some_pattern,
            make_spanned(Pattern::Wildcard),
            make_spanned(Pattern::Literal(Literal::Int(42))),
        ]);
        RustCodegen::emit_pattern(&mut emitter, &tuple);
        assert_eq!(emitter.finish(), "(Some(x), _, 42)");
    }

    // ========================================
    // Match arm tests
    // ========================================

    #[test]
    fn test_emit_match_arm_simple_expr() {
        let mut emitter = RustEmitter::new();
        let arm = MatchArm {
            pattern: make_spanned(Pattern::Binding("x".to_string())),
            guard: None,
            body: MatchBody::Expr(make_spanned(Expr::Ident("x".to_string()))),
        };
        RustCodegen::emit_match_arm(&mut emitter, &arm);
        let output = emitter.finish();
        assert!(output.contains("x => x,"));
    }

    #[test]
    fn test_emit_match_arm_wildcard() {
        let mut emitter = RustEmitter::new();
        let arm = MatchArm {
            pattern: make_spanned(Pattern::Wildcard),
            guard: None,
            body: MatchBody::Expr(make_spanned(Expr::Literal(Literal::Int(0)))),
        };
        RustCodegen::emit_match_arm(&mut emitter, &arm);
        let output = emitter.finish();
        assert!(output.contains("_ => 0,"));
    }

    #[test]
    fn test_emit_match_arm_with_guard() {
        let mut emitter = RustEmitter::new();
        let arm = MatchArm {
            pattern: make_spanned(Pattern::Binding("x".to_string())),
            guard: Some(make_spanned(Expr::Binary(
                Box::new(make_spanned(Expr::Ident("x".to_string()))),
                BinaryOp::Gt,
                Box::new(make_spanned(Expr::Literal(Literal::Int(0)))),
            ))),
            body: MatchBody::Expr(make_spanned(Expr::Ident("x".to_string()))),
        };
        RustCodegen::emit_match_arm(&mut emitter, &arm);
        let output = emitter.finish();
        assert!(output.contains("if"));
        assert!(output.contains("x > 0"));
    }

    #[test]
    fn test_emit_match_arm_with_block_body() {
        let mut emitter = RustEmitter::new();
        let arm = MatchArm {
            pattern: make_spanned(Pattern::Binding("x".to_string())),
            guard: None,
            body: MatchBody::Block(vec![
                make_spanned(Statement::Expr(
                    make_spanned(Expr::Ident("x".to_string()))
                ))
            ]),
        };
        RustCodegen::emit_match_arm(&mut emitter, &arm);
        let output = emitter.finish();
        assert!(output.contains("{"));
        assert!(output.contains("},"));
    }

    #[test]
    fn test_emit_match_arm_constructor_pattern() {
        let mut emitter = RustEmitter::new();
        let arm = MatchArm {
            pattern: make_spanned(Pattern::Constructor(
                "Some".to_string(),
                vec![make_spanned(Pattern::Binding("val".to_string()))]
            )),
            guard: None,
            body: MatchBody::Expr(make_spanned(Expr::Ident("val".to_string()))),
        };
        RustCodegen::emit_match_arm(&mut emitter, &arm);
        let output = emitter.finish();
        assert!(output.contains("Some(val) => val,"));
    }

    #[test]
    fn test_emit_match_arm_literal_pattern() {
        let mut emitter = RustEmitter::new();
        let arm = MatchArm {
            pattern: make_spanned(Pattern::Literal(Literal::Int(42))),
            guard: None,
            body: MatchBody::Expr(make_spanned(Expr::Literal(Literal::String("answer".to_string())))),
        };
        RustCodegen::emit_match_arm(&mut emitter, &arm);
        let output = emitter.finish();
        assert!(output.contains("42 => \"answer\","));
    }
}
