//! Expression emission for code generation
//!
//! Handles emitting all expression types to Rust.

use crate::frontend::ast::{
    Expr, Literal, FStringPart, CallArg, BinaryOp, UnaryOp,
    IfExpr, ListComp, DictComp, SliceExpr, Spanned,
};
use crate::backend::rust_emitter::{RustEmitter, to_rust_ident};

use super::RustCodegen;

impl RustCodegen<'_> {
    /// Emit an expression
    pub(crate) fn emit_expr(emitter: &mut RustEmitter, expr: &Expr) {
        match expr {
            Expr::Ident(name) => {
                emitter.write(&to_rust_ident(name));
            }
            Expr::Literal(lit) => Self::emit_literal(emitter, lit),
            Expr::SelfExpr => emitter.write("self"),
            Expr::Binary(left, op, right) => {
                Self::emit_binary(emitter, left, *op, right);
            }
            Expr::Unary(op, operand) => {
                match op {
                    UnaryOp::Neg => emitter.write("-"),
                    UnaryOp::Not => emitter.write("!"),
                }
                Self::emit_expr(emitter, &operand.node);
            }
            Expr::Call(callee, args) => {
                Self::emit_call(emitter, callee, args);
            }
            Expr::Index(base, index) => {
                Self::emit_expr(emitter, &base.node);
                emitter.write("[");
                Self::emit_expr(emitter, &index.node);
                emitter.write("].clone()");
            }
            Expr::Slice(base, slice) => {
                Self::emit_slice(emitter, base, slice);
            }
            Expr::Field(base, field) => {
                Self::emit_field(emitter, base, field);
            }
            Expr::MethodCall(base, method, args) => {
                Self::emit_method_call(emitter, base, method, args);
            }
            Expr::Await(inner) => {
                Self::emit_expr(emitter, &inner.node);
                emitter.write(".await");
            }
            Expr::Try(inner) => {
                Self::emit_expr(emitter, &inner.node);
                emitter.write("?");
            }
            Expr::Match(subject, arms) => {
                emitter.write("match ");
                Self::emit_expr(emitter, &subject.node);
                emitter.write(" {\n");
                for arm in arms {
                    Self::emit_match_arm(emitter, &arm.node);
                }
                emitter.write("}");
            }
            Expr::If(if_expr) => {
                Self::emit_if_expr(emitter, if_expr);
            }
            Expr::ListComp(comp) => {
                Self::emit_list_comp(emitter, comp);
            }
            Expr::DictComp(comp) => {
                Self::emit_dict_comp(emitter, comp);
            }
            Expr::Lambda(params, body) => {
                emitter.write("|");
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        emitter.write(", ");
                    }
                    emitter.write(&p.node.name);
                }
                emitter.write("| ");
                Self::emit_expr(emitter, &body.node);
            }
            Expr::Tuple(elems) => {
                emitter.write("(");
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        emitter.write(", ");
                    }
                    Self::emit_expr(emitter, &e.node);
                }
                if elems.len() == 1 {
                    emitter.write(",");
                }
                emitter.write(")");
            }
            Expr::List(elems) => {
                emitter.write("vec![");
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        emitter.write(", ");
                    }
                    Self::emit_expr(emitter, &e.node);
                    if matches!(&e.node, Expr::Literal(Literal::String(_))) {
                        emitter.write(".to_string()");
                    }
                }
                emitter.write("]");
            }
            Expr::Dict(entries) => {
                emitter.write("HashMap::from([");
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        emitter.write(", ");
                    }
                    emitter.write("(");
                    Self::emit_expr(emitter, &k.node);
                    emitter.write(", ");
                    Self::emit_expr(emitter, &v.node);
                    emitter.write(")");
                }
                emitter.write("])");
            }
            Expr::Set(elems) => {
                emitter.write("HashSet::from([");
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        emitter.write(", ");
                    }
                    Self::emit_expr(emitter, &e.node);
                }
                emitter.write("])");
            }
            Expr::Paren(inner) => {
                emitter.write("(");
                Self::emit_expr(emitter, &inner.node);
                emitter.write(")");
            }
            Expr::Constructor(name, args) => {
                Self::emit_constructor(emitter, name, args);
            }
            Expr::FString(parts) => {
                Self::emit_fstring(emitter, parts);
            }
            Expr::Yield(inner) => {
                if let Some(inner) = inner {
                    emitter.write("/* yield */ ");
                    Self::emit_expr(emitter, &inner.node);
                } else {
                    emitter.write("/* yield */");
                }
            }
            Expr::Range { start, end, inclusive } => {
                Self::emit_expr(emitter, &start.node);
                if *inclusive {
                    emitter.write("..=");
                } else {
                    emitter.write("..");
                }
                Self::emit_expr(emitter, &end.node);
            }
        }
    }

    /// Emit a binary expression
    fn emit_binary(emitter: &mut RustEmitter, left: &Spanned<Expr>, op: BinaryOp, right: &Spanned<Expr>) {
        match op {
            BinaryOp::In => {
                emitter.write("(");
                Self::emit_expr(emitter, &right.node);
                emitter.write(".contains(&");
                Self::emit_expr(emitter, &left.node);
                emitter.write("))");
            }
            BinaryOp::NotIn => {
                emitter.write("(!");
                Self::emit_expr(emitter, &right.node);
                emitter.write(".contains(&");
                Self::emit_expr(emitter, &left.node);
                emitter.write("))");
            }
            _ => {
                emitter.write("(");
                Self::emit_expr(emitter, &left.node);
                emitter.write(" ");
                emitter.write(Self::binary_op_to_rust(op));
                emitter.write(" ");
                Self::emit_expr(emitter, &right.node);
                emitter.write(")");
            }
        }
    }

    /// Emit a slice expression
    fn emit_slice(emitter: &mut RustEmitter, base: &Spanned<Expr>, slice: &SliceExpr) {
        Self::emit_expr(emitter, &base.node);
        emitter.write("[");

        match (&slice.start, &slice.end, &slice.step) {
            (Some(start), Some(end), None) => {
                Self::emit_expr(emitter, &start.node);
                emitter.write("..");
                Self::emit_expr(emitter, &end.node);
            }
            (Some(start), None, None) => {
                Self::emit_expr(emitter, &start.node);
                emitter.write("..");
            }
            (None, Some(end), None) => {
                emitter.write("..");
                Self::emit_expr(emitter, &end.node);
            }
            (None, None, None) => {
                emitter.write("..");
            }
            (start, end, Some(_step)) => {
                if let Some(s) = start {
                    Self::emit_expr(emitter, &s.node);
                }
                emitter.write("..");
                if let Some(e) = end {
                    Self::emit_expr(emitter, &e.node);
                }
            }
        }

        emitter.write("].to_vec()");
    }

    /// Emit a field access expression
    fn emit_field(emitter: &mut RustEmitter, base: &Spanned<Expr>, field: &str) {
        if let Expr::Ident(name) = &base.node {
            if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                emitter.write(name);
                emitter.write("::");
                emitter.write(&to_rust_ident(field));
                return;
            }
        }
        Self::emit_expr(emitter, &base.node);
        emitter.write(".");
        emitter.write(&to_rust_ident(field));
    }

    /// Emit an if expression
    fn emit_if_expr(emitter: &mut RustEmitter, if_expr: &IfExpr) {
        emitter.write("if ");
        Self::emit_expr(emitter, &if_expr.condition.node);
        emitter.write(" {\n");
        emitter.indent();
        let then_len = if_expr.then_body.len();
        for (i, stmt) in if_expr.then_body.iter().enumerate() {
            let is_last = i == then_len - 1;
            Self::emit_statement_maybe_return(emitter, &stmt.node, is_last);
        }
        emitter.dedent();
        emitter.write_indent();
        emitter.write("}");
        if let Some(else_body) = &if_expr.else_body {
            emitter.write(" else {\n");
            emitter.indent();
            let else_len = else_body.len();
            for (i, stmt) in else_body.iter().enumerate() {
                let is_last = i == else_len - 1;
                Self::emit_statement_maybe_return(emitter, &stmt.node, is_last);
            }
            emitter.dedent();
            emitter.write_indent();
            emitter.write("}");
        }
    }

    /// Emit a list comprehension
    fn emit_list_comp(emitter: &mut RustEmitter, comp: &ListComp) {
        Self::emit_expr(emitter, &comp.iter.node);
        emitter.write(".iter().cloned()");
        if let Some(filter) = &comp.filter {
            emitter.write(&format!(".filter(|&{}| ", comp.var));
            Self::emit_expr(emitter, &filter.node);
            emitter.write(")");
        }
        emitter.write(&format!(".map(|{}| ", comp.var));
        Self::emit_expr(emitter, &comp.expr.node);
        emitter.write(").collect::<Vec<_>>()");
    }

    /// Emit a dict comprehension
    fn emit_dict_comp(emitter: &mut RustEmitter, comp: &DictComp) {
        Self::emit_expr(emitter, &comp.iter.node);
        emitter.write(".iter().cloned()");
        if let Some(filter) = &comp.filter {
            emitter.write(&format!(".filter(|&{}| ", comp.var));
            Self::emit_expr(emitter, &filter.node);
            emitter.write(")");
        }
        emitter.write(&format!(".map(|{}| {{ let __val = ", comp.var));
        Self::emit_expr(emitter, &comp.value.node);
        emitter.write("; (");
        Self::emit_expr(emitter, &comp.key.node);
        emitter.write(", __val) })");
        emitter.write(".collect::<HashMap<_, _>>()");
    }

    /// Emit a constructor expression
    fn emit_constructor(emitter: &mut RustEmitter, name: &str, args: &[CallArg]) {
        let all_named = args.iter().all(|a| matches!(a, CallArg::Named(_, _)));
        if all_named && !args.is_empty() {
            emitter.write(name);
            emitter.write(" { ");
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    emitter.write(", ");
                }
                if let CallArg::Named(field, value) = arg {
                    emitter.write(&to_rust_ident(field));
                    emitter.write(": ");
                    Self::emit_expr(emitter, &value.node);
                }
            }
            emitter.write(" }");
        } else {
            emitter.write(name);
            if !args.is_empty() {
                emitter.write("(");
                Self::emit_call_args(emitter, args);
                emitter.write(")");
            }
        }
    }

    /// Emit a literal
    pub(crate) fn emit_literal(emitter: &mut RustEmitter, lit: &Literal) {
        match lit {
            Literal::Int(n) => emitter.writef(format_args!("{}", n)),
            Literal::Float(f) => {
                let formatted = format!("{}", f);
                if formatted.contains('.') || formatted.contains('e') || formatted.contains('E') {
                    emitter.write(&formatted);
                } else {
                    emitter.writef(format_args!("{}.0", f));
                }
            }
            Literal::String(s) => emitter.writef(format_args!("{:?}", s)),
            Literal::Bytes(bytes) => {
                emitter.write("vec![");
                for (i, b) in bytes.iter().enumerate() {
                    if i > 0 {
                        emitter.write(", ");
                    }
                    emitter.writef(format_args!("{}", b));
                }
                emitter.write("]");
            }
            Literal::Bool(b) => emitter.write(if *b { "true" } else { "false" }),
            Literal::None => emitter.write("None"),
        }
    }

    /// Emit an f-string
    pub(crate) fn emit_fstring(emitter: &mut RustEmitter, parts: &[FStringPart]) {
        emitter.write("format!(\"");
        let mut args = Vec::new();
        for part in parts {
            match part {
                FStringPart::Literal(s) => {
                    for c in s.chars() {
                        match c {
                            '{' => emitter.write("{{"),
                            '}' => emitter.write("}}"),
                            '"' => emitter.write("\\\""),
                            '\\' => emitter.write("\\\\"),
                            '\n' => emitter.write("\\n"),
                            _ => emitter.writef(format_args!("{}", c)),
                        }
                    }
                }
                FStringPart::Expr(e) => {
                    emitter.write("{}");
                    args.push(e);
                }
            }
        }
        emitter.write("\"");
        for arg in args {
            emitter.write(", ");
            Self::emit_expr(emitter, &arg.node);
        }
        emitter.write(")");
    }

    /// Emit println/print
    pub(crate) fn emit_println(emitter: &mut RustEmitter, args: &[CallArg], newline: bool) {
        let macro_name = if newline { "println!" } else { "print!" };

        if args.is_empty() {
            emitter.write(macro_name);
            emitter.write("()");
            return;
        }

        if let Some(CallArg::Positional(first)) = args.first() {
            if let Expr::FString(parts) = &first.node {
                emitter.write(macro_name);
                emitter.write("(\"");
                let mut fstring_args = Vec::new();
                for part in parts {
                    match part {
                        FStringPart::Literal(s) => {
                            for c in s.chars() {
                                match c {
                                    '{' => emitter.write("{{"),
                                    '}' => emitter.write("}}"),
                                    '"' => emitter.write("\\\""),
                                    '\\' => emitter.write("\\\\"),
                                    '\n' => emitter.write("\\n"),
                                    _ => emitter.writef(format_args!("{}", c)),
                                }
                            }
                        }
                        FStringPart::Expr(e) => {
                            emitter.write("{}");
                            fstring_args.push(e);
                        }
                    }
                }
                emitter.write("\"");
                for arg in fstring_args {
                    emitter.write(", ");
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write(")");
                return;
            }

            if args.len() == 1 {
                if let Expr::Literal(Literal::String(s)) = &first.node {
                    emitter.write(macro_name);
                    emitter.write("(\"{}\"");
                    emitter.write(", ");
                    emitter.writef(format_args!("{:?}", s));
                    emitter.write(")");
                    return;
                }
            }
        }

        emitter.write(macro_name);
        emitter.write("(\"");
        for i in 0..args.len() {
            if i > 0 {
                emitter.write(" ");
            }
            emitter.write("{}");
        }
        emitter.write("\", ");
        Self::emit_call_args(emitter, args);
        emitter.write(")");
    }

    /// Emit call arguments (with auto string conversion and cloning for ownership)
    pub(crate) fn emit_call_args(emitter: &mut RustEmitter, args: &[CallArg]) {
        Self::emit_call_args_inner(emitter, args, true, true);
    }

    /// Emit method arguments (no auto string conversion, no auto cloning)
    pub(crate) fn emit_method_args(emitter: &mut RustEmitter, args: &[CallArg]) {
        Self::emit_call_args_inner(emitter, args, false, false);
    }

    /// Check if an expression needs cloning when passed to a function
    /// This prevents ownership issues when variables/fields are used after being passed
    fn needs_clone_for_call(expr: &Expr) -> bool {
        match expr {
            // Field accesses like user.email, product.name need cloning
            Expr::Field(_, _) => true,
            // Variables of complex types need cloning (clone all to be safe)
            Expr::Ident(_) => true,
            // Calls return owned values, no need to clone
            Expr::Call(_, _) | Expr::MethodCall(_, _, _) => false,
            // Literals are Copy or create owned values
            Expr::Literal(_) => false,
            // Constructors return owned values
            Expr::Constructor(_, _) => false,
            // Index already clones
            Expr::Index(_, _) => false,
            // Other expressions don't need clone
            _ => false,
        }
    }

    /// Emit call arguments with optional auto string conversion and cloning
    fn emit_call_args_inner(emitter: &mut RustEmitter, args: &[CallArg], auto_string: bool, auto_clone: bool) {
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                emitter.write(", ");
            }
            match arg {
                CallArg::Positional(e) => {
                    Self::emit_expr(emitter, &e.node);
                    if auto_string && matches!(&e.node, Expr::Literal(Literal::String(_))) {
                        emitter.write(".to_string()");
                    } else if auto_clone && Self::needs_clone_for_call(&e.node) {
                        emitter.write(".clone()");
                    }
                }
                CallArg::Named(_name, e) => {
                    Self::emit_expr(emitter, &e.node);
                    if auto_string && matches!(&e.node, Expr::Literal(Literal::String(_))) {
                        emitter.write(".to_string()");
                    } else if auto_clone && Self::needs_clone_for_call(&e.node) {
                        emitter.write(".clone()");
                    }
                }
            }
        }
    }

    /// Emit a function call
    fn emit_call(emitter: &mut RustEmitter, callee: &Spanned<Expr>, args: &[CallArg]) {
        if let Expr::Ident(name) = &callee.node {
            if Self::emit_builtin_call(emitter, name, args) {
                return;
            }
        }
        Self::emit_expr(emitter, &callee.node);
        emitter.write("(");
        Self::emit_call_args(emitter, args);
        emitter.write(")");
    }

    /// Try to emit a builtin call, returns true if handled
    fn emit_builtin_call(emitter: &mut RustEmitter, name: &str, args: &[CallArg]) -> bool {
        match name {
            "println" => { Self::emit_println(emitter, args, true); true }
            "print" => { Self::emit_println(emitter, args, false); true }
            "assert" => {
                emitter.write("assert!(");
                if let Some(CallArg::Positional(cond)) = args.first() {
                    Self::emit_expr(emitter, &cond.node);
                }
                if args.len() >= 2 {
                    if let CallArg::Positional(msg) = &args[1] {
                        emitter.write(", ");
                        Self::emit_expr(emitter, &msg.node);
                    }
                }
                emitter.write(")");
                true
            }
            "assert_eq" => {
                emitter.write("assert_eq!(");
                if args.len() >= 2 {
                    if let (CallArg::Positional(left), CallArg::Positional(right)) = (&args[0], &args[1]) {
                        Self::emit_expr(emitter, &left.node);
                        emitter.write(", ");
                        Self::emit_expr(emitter, &right.node);
                    }
                }
                if args.len() >= 3 {
                    if let CallArg::Positional(msg) = &args[2] {
                        emitter.write(", ");
                        Self::emit_expr(emitter, &msg.node);
                    }
                }
                emitter.write(")");
                true
            }
            "assert_ne" => {
                emitter.write("assert_ne!(");
                if args.len() >= 2 {
                    if let (CallArg::Positional(left), CallArg::Positional(right)) = (&args[0], &args[1]) {
                        Self::emit_expr(emitter, &left.node);
                        emitter.write(", ");
                        Self::emit_expr(emitter, &right.node);
                    }
                }
                emitter.write(")");
                true
            }
            "assert_true" => {
                emitter.write("assert!(");
                if let Some(CallArg::Positional(cond)) = args.first() {
                    Self::emit_expr(emitter, &cond.node);
                }
                emitter.write(")");
                true
            }
            "assert_false" => {
                emitter.write("assert!(!");
                if let Some(CallArg::Positional(cond)) = args.first() {
                    Self::emit_expr(emitter, &cond.node);
                }
                emitter.write(")");
                true
            }
            "fail" => {
                emitter.write("panic!(");
                if let Some(CallArg::Positional(msg)) = args.first() {
                    Self::emit_expr(emitter, &msg.node);
                } else {
                    emitter.write("\"test failed\"");
                }
                emitter.write(")");
                true
            }
            "len" => {
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                    emitter.write(".len()");
                    return true;
                }
                false
            }
            "range" => {
                match args.len() {
                    1 => {
                        emitter.write("(");
                        if let CallArg::Positional(arg) = &args[0] {
                            if let Expr::Range { start, end, inclusive } = &arg.node {
                                Self::emit_expr(emitter, &start.node);
                                if *inclusive {
                                    emitter.write("..=");
                                } else {
                                    emitter.write("..");
                                }
                                Self::emit_expr(emitter, &end.node);
                            } else {
                                emitter.write("0..");
                                Self::emit_expr(emitter, &arg.node);
                            }
                        }
                        emitter.write(")");
                        true
                    }
                    2 => {
                        emitter.write("(");
                        if let CallArg::Positional(a) = &args[0] {
                            Self::emit_expr(emitter, &a.node);
                        }
                        emitter.write("..");
                        if let CallArg::Positional(b) = &args[1] {
                            Self::emit_expr(emitter, &b.node);
                        }
                        emitter.write(")");
                        true
                    }
                    3 => {
                        emitter.write("(");
                        if let CallArg::Positional(a) = &args[0] {
                            Self::emit_expr(emitter, &a.node);
                        }
                        emitter.write("..");
                        if let CallArg::Positional(b) = &args[1] {
                            Self::emit_expr(emitter, &b.node);
                        }
                        emitter.write(").step_by(");
                        if let CallArg::Positional(step) = &args[2] {
                            Self::emit_expr(emitter, &step.node);
                        }
                        emitter.write(" as usize)");
                        true
                    }
                    _ => false, // fall through to regular call emission
                }
            }
            "dict" => {
                if args.is_empty() {
                    emitter.write("std::collections::HashMap::new()");
                } else if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                    emitter.write(".clone().into_iter().collect::<std::collections::HashMap<_, _>>()");
                }
                true
            }
            "list" => {
                if args.is_empty() {
                    emitter.write("Vec::new()");
                } else if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                    emitter.write(".into_iter().collect::<Vec<_>>()");
                }
                true
            }
            "set" => {
                if args.is_empty() {
                    emitter.write("std::collections::HashSet::new()");
                } else if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                    emitter.write(".into_iter().collect::<std::collections::HashSet<_>>()");
                }
                true
            }
            "enumerate" => {
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                    emitter.write(".iter().enumerate().map(|(i, x)| (i as i64, x.clone())).collect::<Vec<_>>()");
                    return true;
                }
                false
            }
            "zip" => {
                if args.len() >= 2 {
                    if let (CallArg::Positional(arg1), CallArg::Positional(arg2)) = (&args[0], &args[1]) {
                        Self::emit_expr(emitter, &arg1.node);
                        emitter.write(".iter().zip(");
                        Self::emit_expr(emitter, &arg2.node);
                        emitter.write(".iter()).map(|(a, b)| (a.clone(), b.clone())).collect::<Vec<_>>()");
                        return true;
                    }
                }
                false
            }
            "read_file" => {
                emitter.write("std::fs::read_to_string(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write(").map_err(|e| e.to_string())");
                true
            }
            "write_file" => {
                emitter.write("std::fs::write(");
                if args.len() >= 2 {
                    if let CallArg::Positional(path) = &args[0] {
                        Self::emit_expr(emitter, &path.node);
                    }
                    emitter.write(", ");
                    if let CallArg::Positional(content) = &args[1] {
                        Self::emit_expr(emitter, &content.node);
                    }
                }
                emitter.write(").map_err(|e| e.to_string())");
                true
            }
            "int" => {
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                    emitter.write(".to_string().parse::<i64>().unwrap()");
                    return true;
                }
                false
            }
            "str" => {
                if let Some(CallArg::Positional(arg)) = args.first() {
                    emitter.write("format!(\"{}\", ");
                    Self::emit_expr(emitter, &arg.node);
                    emitter.write(")");
                    return true;
                }
                false
            }
            "float" => {
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                    emitter.write(".to_string().parse::<f64>().unwrap()");
                    return true;
                }
                false
            }
            "json_stringify" => {
                emitter.write("serde_json::to_string(&");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write(").unwrap()");
                true
            }
            "json_parse" => {
                emitter.write("serde_json::from_str(&");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write(").map_err(|e| e.to_string())");
                true
            }
            // Async primitives
            "sleep" => {
                emitter.write("tokio::time::sleep(tokio::time::Duration::from_secs_f64(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write("))");
                true
            }
            "sleep_ms" => {
                emitter.write("tokio::time::sleep(tokio::time::Duration::from_millis(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write(" as u64))");
                true
            }
            "timeout" => {
                emitter.write("tokio::time::timeout(tokio::time::Duration::from_secs_f64(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write("), ");
                if args.len() >= 2 {
                    if let CallArg::Positional(task) = &args[1] {
                        Self::emit_expr(emitter, &task.node);
                        emitter.write("()");
                    }
                }
                emitter.write(").await.map_err(|_| \"timeout\".to_string())");
                true
            }
            "timeout_ms" => {
                emitter.write("tokio::time::timeout(tokio::time::Duration::from_millis(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write(" as u64), ");
                if args.len() >= 2 {
                    if let CallArg::Positional(task) = &args[1] {
                        Self::emit_expr(emitter, &task.node);
                        emitter.write("()");
                    }
                }
                emitter.write(").await.map_err(|_| \"timeout\".to_string())");
                true
            }
            "spawn" => {
                emitter.write("tokio::spawn(");
                if let Some(CallArg::Positional(task)) = args.first() {
                    Self::emit_expr(emitter, &task.node);
                    // Only add () if the expression isn't already a call
                    if !matches!(&task.node, Expr::Call(_, _) | Expr::MethodCall(_, _, _)) {
                        emitter.write("()");
                    }
                }
                emitter.write(")");
                true
            }
            "spawn_blocking" => {
                emitter.write("tokio::task::spawn_blocking(|| ");
                if let Some(CallArg::Positional(f)) = args.first() {
                    Self::emit_expr(emitter, &f.node);
                    // Only add () if the expression isn't already a call
                    if !matches!(&f.node, Expr::Call(_, _) | Expr::MethodCall(_, _, _)) {
                        emitter.write("()");
                    }
                }
                emitter.write(")");
                true
            }
            "yield_now" => {
                emitter.write("tokio::task::yield_now()");
                true
            }
            "channel" => {
                emitter.write("tokio::sync::mpsc::channel(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                } else {
                    emitter.write("32");
                }
                emitter.write(" as usize)");
                true
            }
            "unbounded_channel" => {
                emitter.write("tokio::sync::mpsc::unbounded_channel()");
                true
            }
            "oneshot" => {
                emitter.write("tokio::sync::oneshot::channel()");
                true
            }
            "Mutex" => {
                // Wrap in Arc for sharing between tasks
                emitter.write("std::sync::Arc::new(tokio::sync::Mutex::new(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write("))");
                true
            }
            "RwLock" => {
                // Wrap in Arc for sharing between tasks
                emitter.write("std::sync::Arc::new(tokio::sync::RwLock::new(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write("))");
                true
            }
            "Semaphore" => {
                // Wrap in Arc for sharing between tasks
                emitter.write("std::sync::Arc::new(tokio::sync::Semaphore::new(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write(" as usize))");
                true
            }
            "Barrier" => {
                // Wrap in Arc for sharing between tasks
                emitter.write("std::sync::Arc::new(tokio::sync::Barrier::new(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write(" as usize))");
                true
            }
            "select2" => {
                emitter.write("{\n");
                emitter.write("    #[allow(dead_code)]\n");
                emitter.write("    enum Either<L, R> { Left(L), Right(R) }\n");
                emitter.write("    tokio::select! {\n");
                emitter.write("        biased;\n");
                emitter.write("        result = ");
                if let Some(CallArg::Positional(a)) = args.first() {
                    Self::emit_expr(emitter, &a.node);
                }
                emitter.write("() => Either::Left(result),\n");
                emitter.write("        result = ");
                if args.len() >= 2 {
                    if let CallArg::Positional(b) = &args[1] {
                        Self::emit_expr(emitter, &b.node);
                    }
                }
                emitter.write("() => Either::Right(result),\n");
                emitter.write("    }\n");
                emitter.write("}");
                true
            }
            "select3" => {
                emitter.write("{\n");
                emitter.write("    #[allow(dead_code)]\n");
                emitter.write("    enum Either3<A, B, C> { First(A), Second(B), Third(C) }\n");
                emitter.write("    tokio::select! {\n");
                emitter.write("        biased;\n");
                emitter.write("        result = ");
                if let Some(CallArg::Positional(a)) = args.first() {
                    Self::emit_expr(emitter, &a.node);
                }
                emitter.write("() => Either3::First(result),\n");
                emitter.write("        result = ");
                if args.len() >= 2 {
                    if let CallArg::Positional(b) = &args[1] {
                        Self::emit_expr(emitter, &b.node);
                    }
                }
                emitter.write("() => Either3::Second(result),\n");
                emitter.write("        result = ");
                if args.len() >= 3 {
                    if let CallArg::Positional(c) = &args[2] {
                        Self::emit_expr(emitter, &c.node);
                    }
                }
                emitter.write("() => Either3::Third(result),\n");
                emitter.write("    }\n");
                emitter.write("}");
                true
            }
            "select_timeout" => {
                emitter.write("tokio::time::timeout(tokio::time::Duration::from_secs_f64(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write("), ");
                if args.len() >= 2 {
                    if let CallArg::Positional(task) = &args[1] {
                        Self::emit_expr(emitter, &task.node);
                        emitter.write("()");
                    }
                }
                emitter.write(").await.ok()");
                true
            }
            _ => {
                // Check if this looks like struct construction or type constructor
                let is_type_name = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                let all_named = !args.is_empty() && args.iter().all(|a| matches!(a, CallArg::Named(_, _)));

                if is_type_name && all_named {
                    // Struct literal: User { name: value, ... }
                    emitter.write(name);
                    emitter.write(" { ");
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            emitter.write(", ");
                        }
                        if let CallArg::Named(field, value) = arg {
                            emitter.write(&to_rust_ident(field));
                            emitter.write(": ");
                            Self::emit_expr(emitter, &value.node);
                            if matches!(&value.node, Expr::Literal(Literal::String(_))) {
                                emitter.write(".to_string()");
                            }
                        }
                    }
                    emitter.write(" }");
                    true
                } else if is_type_name {
                    // Type constructor: e.g. Email("foo@bar.baz") or UserId(42)
                    emitter.write(name);
                    emitter.write("(");
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            emitter.write(", ");
                        }
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => {
                                Self::emit_expr(emitter, &e.node);
                                if matches!(&e.node, Expr::Literal(Literal::String(_))) {
                                    emitter.write(".to_string()");
                                }
                            }
                        }
                    }
                    emitter.write(")");
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Emit a method call
    fn emit_method_call(emitter: &mut RustEmitter, base: &Spanned<Expr>, method: &str, args: &[CallArg]) {
        if let Expr::Ident(name) = &base.node {
            // Handle Response builder methods
            if name == "Response" {
                if Self::emit_response_method(emitter, method, args) {
                    return;
                }
            }

            // Handle enum variant constructors and static methods on types
            if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                emitter.write(name);
                emitter.write("::");
                emitter.write(&to_rust_ident(method));
                emitter.write("(");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        emitter.write(", ");
                    }
                    match arg {
                        CallArg::Positional(e) | CallArg::Named(_, e) => {
                            // For from_json, borrow the string argument
                            let needs_borrow = method == "from_json" && !matches!(&e.node, Expr::Literal(Literal::String(_)));
                            if needs_borrow {
                                emitter.write("&");
                            }
                            Self::emit_expr(emitter, &e.node);
                            if matches!(&e.node, Expr::Literal(Literal::String(_))) {
                                // String literals are already &str, no conversion needed for from_json
                                if method != "from_json" {
                                    emitter.write(".to_string()");
                                }
                            }
                        }
                    }
                }
                emitter.write(")");
                return;
            }
        }

        // Handle Python-style string methods
        if Self::emit_string_method(emitter, base, method, args) {
            return;
        }

        // Handle MutexGuard/RwLockGuard methods: get() and set()
        match method {
            "get" if args.is_empty() => {
                // guard.get() -> dereference the guard to get the value
                emitter.write("(*");
                Self::emit_expr(emitter, &base.node);
                emitter.write(").clone()");
                return;
            }
            "set" if args.len() == 1 => {
                // guard.set(value) -> assign through the guard
                emitter.write("*");
                Self::emit_expr(emitter, &base.node);
                emitter.write(" = ");
                if let Some(CallArg::Positional(e)) = args.first() {
                    Self::emit_expr(emitter, &e.node);
                }
                return;
            }
            _ => {}
        }

        // Default method call
        Self::emit_expr(emitter, &base.node);
        emitter.write(".");
        emitter.write(&to_rust_ident(method));
        emitter.write("(");
        Self::emit_method_args(emitter, args);
        emitter.write(")");
    }

    /// Emit Response builder methods
    fn emit_response_method(emitter: &mut RustEmitter, method: &str, args: &[CallArg]) -> bool {
        match method {
            "html" => {
                emitter.write("Html(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                    emitter.write(".to_string()");
                }
                emitter.write(")");
                true
            }
            "text" => {
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                    emitter.write(".to_string()");
                }
                true
            }
            "ok" => { emitter.write("axum::http::StatusCode::OK"); true }
            "created" => { emitter.write("axum::http::StatusCode::CREATED"); true }
            "no_content" => { emitter.write("axum::http::StatusCode::NO_CONTENT"); true }
            "bad_request" => {
                emitter.write("(axum::http::StatusCode::BAD_REQUEST, ");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                } else {
                    emitter.write("\"Bad Request\"");
                }
                emitter.write(")");
                true
            }
            "not_found" => {
                emitter.write("(axum::http::StatusCode::NOT_FOUND, ");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                } else {
                    emitter.write("\"Not Found\"");
                }
                emitter.write(")");
                true
            }
            "internal_error" => {
                emitter.write("(axum::http::StatusCode::INTERNAL_SERVER_ERROR, ");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                } else {
                    emitter.write("\"Internal Server Error\"");
                }
                emitter.write(")");
                true
            }
            _ => false
        }
    }

    /// Emit Python-style string methods
    fn emit_string_method(emitter: &mut RustEmitter, base: &Spanned<Expr>, method: &str, args: &[CallArg]) -> bool {
        match method {
            "upper" => {
                Self::emit_expr(emitter, &base.node);
                emitter.write(".to_uppercase()");
                true
            }
            "lower" => {
                Self::emit_expr(emitter, &base.node);
                emitter.write(".to_lowercase()");
                true
            }
            "strip" => {
                Self::emit_expr(emitter, &base.node);
                emitter.write(".trim().to_string()");
                true
            }
            "split" => {
                Self::emit_expr(emitter, &base.node);
                emitter.write(".split(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write(").map(|s| s.to_string()).collect::<Vec<_>>()");
                true
            }
            "join" => {
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                    emitter.write(".join(");
                    Self::emit_expr(emitter, &base.node);
                    emitter.write(")");
                    return true;
                }
                false
            }
            "contains" => {
                Self::emit_expr(emitter, &base.node);
                emitter.write(".contains(");
                if let Some(CallArg::Positional(arg)) = args.first() {
                    Self::emit_expr(emitter, &arg.node);
                }
                emitter.write(")");
                true
            }
            "replace" => {
                Self::emit_expr(emitter, &base.node);
                emitter.write(".replace(");
                Self::emit_method_args(emitter, args);
                emitter.write(")");
                true
            }
            _ => false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::ast::{Param, Span, Type};

    fn make_spanned<T>(node: T) -> Spanned<T> {
        Spanned { node, span: Span::default() }
    }

    fn ident_expr(name: &str) -> Expr {
        Expr::Ident(name.to_string())
    }

    fn int_lit(n: i64) -> Expr {
        Expr::Literal(Literal::Int(n))
    }

    fn str_lit(s: &str) -> Expr {
        Expr::Literal(Literal::String(s.to_string()))
    }

    fn bool_lit(b: bool) -> Expr {
        Expr::Literal(Literal::Bool(b))
    }

    // ========================================
    // Literal emission tests
    // ========================================

    #[test]
    fn test_emit_literal_int() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_literal(&mut emitter, &Literal::Int(42));
        assert_eq!(emitter.finish(), "42");
    }

    #[test]
    fn test_emit_literal_negative_int() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_literal(&mut emitter, &Literal::Int(-100));
        assert_eq!(emitter.finish(), "-100");
    }

    #[test]
    fn test_emit_literal_float() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_literal(&mut emitter, &Literal::Float(3.14159));
        let output = emitter.finish();
        assert!(output.starts_with("3.14"));
    }

    #[test]
    fn test_emit_literal_string() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_literal(&mut emitter, &Literal::String("hello world".to_string()));
        assert_eq!(emitter.finish(), "\"hello world\"");
    }

    #[test]
    fn test_emit_literal_string_with_escapes() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_literal(&mut emitter, &Literal::String("line1\nline2".to_string()));
        // Strings are escaped in the output
        let output = emitter.finish();
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
    }

    #[test]
    fn test_emit_literal_bool_true() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_literal(&mut emitter, &Literal::Bool(true));
        assert_eq!(emitter.finish(), "true");
    }

    #[test]
    fn test_emit_literal_bool_false() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_literal(&mut emitter, &Literal::Bool(false));
        assert_eq!(emitter.finish(), "false");
    }

    #[test]
    fn test_emit_literal_none() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_literal(&mut emitter, &Literal::None);
        assert_eq!(emitter.finish(), "None");
    }

    #[test]
    fn test_emit_literal_bytes() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_literal(&mut emitter, &Literal::Bytes(vec![65, 66, 67]));
        let output = emitter.finish();
        assert!(output.contains("vec!"));
        assert!(output.contains("65"));
    }

    // ========================================
    // Identifier emission tests
    // ========================================

    #[test]
    fn test_emit_expr_ident_simple() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &ident_expr("foo"));
        assert_eq!(emitter.finish(), "foo");
    }

    #[test]
    fn test_emit_expr_ident_reserved() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &ident_expr("type"));
        assert_eq!(emitter.finish(), "r#type");
    }

    #[test]
    fn test_emit_expr_self() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::SelfExpr);
        assert_eq!(emitter.finish(), "self");
    }

    // ========================================
    // Binary operator tests
    // ========================================

    #[test]
    fn test_emit_binary_add() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(int_lit(1))),
            BinaryOp::Add,
            Box::new(make_spanned(int_lit(2))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        // Binary ops are parenthesized in the codegen
        assert_eq!(emitter.finish(), "(1 + 2)");
    }

    #[test]
    fn test_emit_binary_sub() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(int_lit(10))),
            BinaryOp::Sub,
            Box::new(make_spanned(int_lit(5))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "(10 - 5)");
    }

    #[test]
    fn test_emit_binary_mul() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(int_lit(3))),
            BinaryOp::Mul,
            Box::new(make_spanned(int_lit(4))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "(3 * 4)");
    }

    #[test]
    fn test_emit_binary_div() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(int_lit(10))),
            BinaryOp::Div,
            Box::new(make_spanned(int_lit(2))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "(10 / 2)");
    }

    #[test]
    fn test_emit_binary_mod() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(int_lit(10))),
            BinaryOp::Mod,
            Box::new(make_spanned(int_lit(3))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "(10 % 3)");
    }

    #[test]
    fn test_emit_binary_eq() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(ident_expr("x"))),
            BinaryOp::Eq,
            Box::new(make_spanned(int_lit(5))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "(x == 5)");
    }

    #[test]
    fn test_emit_binary_not_eq() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(ident_expr("x"))),
            BinaryOp::NotEq,
            Box::new(make_spanned(int_lit(5))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "(x != 5)");
    }

    #[test]
    fn test_emit_binary_lt() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(ident_expr("x"))),
            BinaryOp::Lt,
            Box::new(make_spanned(int_lit(10))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "(x < 10)");
    }

    #[test]
    fn test_emit_binary_gt() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(ident_expr("x"))),
            BinaryOp::Gt,
            Box::new(make_spanned(int_lit(0))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "(x > 0)");
    }

    #[test]
    fn test_emit_binary_and() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(bool_lit(true))),
            BinaryOp::And,
            Box::new(make_spanned(bool_lit(false))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "(true && false)");
    }

    #[test]
    fn test_emit_binary_or() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(bool_lit(true))),
            BinaryOp::Or,
            Box::new(make_spanned(bool_lit(false))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "(true || false)");
    }

    #[test]
    fn test_emit_binary_in() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Binary(
            Box::new(make_spanned(int_lit(1))),
            BinaryOp::In,
            Box::new(make_spanned(ident_expr("items"))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("contains"));
    }

    // ========================================
    // Unary operator tests
    // ========================================

    #[test]
    fn test_emit_unary_neg() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Unary(
            UnaryOp::Neg,
            Box::new(make_spanned(int_lit(5))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "-5");
    }

    #[test]
    fn test_emit_unary_not() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Unary(
            UnaryOp::Not,
            Box::new(make_spanned(bool_lit(true))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "!true");
    }

    // ========================================
    // Collection tests
    // ========================================

    #[test]
    fn test_emit_list_empty() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::List(vec![]));
        assert_eq!(emitter.finish(), "vec![]");
    }

    #[test]
    fn test_emit_list_single() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::List(vec![make_spanned(int_lit(1))]));
        assert_eq!(emitter.finish(), "vec![1]");
    }

    #[test]
    fn test_emit_list_multiple() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::List(vec![
            make_spanned(int_lit(1)),
            make_spanned(int_lit(2)),
            make_spanned(int_lit(3)),
        ]));
        assert_eq!(emitter.finish(), "vec![1, 2, 3]");
    }

    #[test]
    fn test_emit_list_strings_get_to_string() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::List(vec![
            make_spanned(str_lit("hello")),
        ]));
        let output = emitter.finish();
        assert!(output.contains(".to_string()"));
    }

    #[test]
    fn test_emit_dict_empty() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::Dict(vec![]));
        assert_eq!(emitter.finish(), "HashMap::from([])");
    }

    #[test]
    fn test_emit_dict_single() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::Dict(vec![
            (make_spanned(str_lit("key")), make_spanned(int_lit(42))),
        ]));
        let output = emitter.finish();
        assert!(output.contains("HashMap::from"));
        assert!(output.contains("\"key\""));
        assert!(output.contains("42"));
    }

    #[test]
    fn test_emit_dict_multiple() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::Dict(vec![
            (make_spanned(str_lit("a")), make_spanned(int_lit(1))),
            (make_spanned(str_lit("b")), make_spanned(int_lit(2))),
        ]));
        let output = emitter.finish();
        assert!(output.contains("\"a\""));
        assert!(output.contains("\"b\""));
    }

    #[test]
    fn test_emit_set_empty() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::Set(vec![]));
        assert_eq!(emitter.finish(), "HashSet::from([])");
    }

    #[test]
    fn test_emit_set_multiple() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::Set(vec![
            make_spanned(int_lit(1)),
            make_spanned(int_lit(2)),
            make_spanned(int_lit(3)),
        ]));
        let output = emitter.finish();
        assert!(output.contains("HashSet::from"));
        assert!(output.contains("1"));
        assert!(output.contains("2"));
    }

    // ========================================
    // Tuple tests
    // ========================================

    #[test]
    fn test_emit_tuple_empty() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::Tuple(vec![]));
        assert_eq!(emitter.finish(), "()");
    }

    #[test]
    fn test_emit_tuple_single_has_trailing_comma() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::Tuple(vec![make_spanned(int_lit(42))]));
        assert_eq!(emitter.finish(), "(42,)");
    }

    #[test]
    fn test_emit_tuple_multiple() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_expr(&mut emitter, &Expr::Tuple(vec![
            make_spanned(int_lit(1)),
            make_spanned(int_lit(2)),
            make_spanned(int_lit(3)),
        ]));
        assert_eq!(emitter.finish(), "(1, 2, 3)");
    }

    // ========================================
    // Index and slice tests
    // ========================================

    #[test]
    fn test_emit_index() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Index(
            Box::new(make_spanned(ident_expr("arr"))),
            Box::new(make_spanned(int_lit(0))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "arr[0].clone()");
    }

    #[test]
    fn test_emit_slice_full() {
        let mut emitter = RustEmitter::new();
        let slice = SliceExpr {
            start: Some(Box::new(make_spanned(int_lit(1)))),
            end: Some(Box::new(make_spanned(int_lit(3)))),
            step: None,
        };
        let expr = Expr::Slice(
            Box::new(make_spanned(ident_expr("arr"))),
            slice,
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("arr"));
        assert!(output.contains("1"));
        assert!(output.contains("3"));
    }

    #[test]
    fn test_emit_slice_start_only() {
        let mut emitter = RustEmitter::new();
        let slice = SliceExpr {
            start: Some(Box::new(make_spanned(int_lit(2)))),
            end: None,
            step: None,
        };
        let expr = Expr::Slice(
            Box::new(make_spanned(ident_expr("arr"))),
            slice,
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("arr"));
    }

    // ========================================
    // Paren tests
    // ========================================

    #[test]
    fn test_emit_paren() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Paren(Box::new(make_spanned(int_lit(42))));
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "(42)");
    }

    #[test]
    fn test_emit_paren_complex() {
        let mut emitter = RustEmitter::new();
        let inner = Expr::Binary(
            Box::new(make_spanned(int_lit(1))),
            BinaryOp::Add,
            Box::new(make_spanned(int_lit(2))),
        );
        let expr = Expr::Paren(Box::new(make_spanned(inner)));
        RustCodegen::emit_expr(&mut emitter, &expr);
        // Double parens: Paren wraps Binary which also adds parens
        assert_eq!(emitter.finish(), "((1 + 2))");
    }

    // ========================================
    // Await and Try tests
    // ========================================

    #[test]
    fn test_emit_await() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Await(Box::new(make_spanned(ident_expr("future"))));
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "future.await");
    }

    #[test]
    fn test_emit_try() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Try(Box::new(make_spanned(ident_expr("result"))));
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "result?");
    }

    // ========================================
    // Lambda tests
    // ========================================

    #[test]
    fn test_emit_lambda_no_params() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Lambda(vec![], Box::new(make_spanned(int_lit(42))));
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "|| 42");
    }

    #[test]
    fn test_emit_lambda_one_param() {
        let mut emitter = RustEmitter::new();
        let param = Param {
            name: "x".to_string(),
            ty: make_spanned(Type::Simple("int".to_string())),
            default: None,
        };
        let expr = Expr::Lambda(
            vec![make_spanned(param)],
            Box::new(make_spanned(ident_expr("x"))),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "|x| x");
    }

    #[test]
    fn test_emit_lambda_multiple_params() {
        let mut emitter = RustEmitter::new();
        let params = vec![
            make_spanned(Param { name: "x".to_string(), ty: make_spanned(Type::Simple("int".to_string())), default: None }),
            make_spanned(Param { name: "y".to_string(), ty: make_spanned(Type::Simple("int".to_string())), default: None }),
        ];
        let body = Expr::Binary(
            Box::new(make_spanned(ident_expr("x"))),
            BinaryOp::Add,
            Box::new(make_spanned(ident_expr("y"))),
        );
        let expr = Expr::Lambda(params, Box::new(make_spanned(body)));
        RustCodegen::emit_expr(&mut emitter, &expr);
        // Binary adds parens
        assert_eq!(emitter.finish(), "|x, y| (x + y)");
    }

    // ========================================
    // F-string tests
    // ========================================

    #[test]
    fn test_emit_fstring_text_only() {
        let mut emitter = RustEmitter::new();
        let parts = vec![FStringPart::Literal("hello world".to_string())];
        RustCodegen::emit_fstring(&mut emitter, &parts);
        let output = emitter.finish();
        assert!(output.contains("format!"));
        assert!(output.contains("hello world"));
    }

    #[test]
    fn test_emit_fstring_with_interpolation() {
        let mut emitter = RustEmitter::new();
        let parts = vec![
            FStringPart::Literal("Hello, ".to_string()),
            FStringPart::Expr(make_spanned(ident_expr("name"))),
            FStringPart::Literal("!".to_string()),
        ];
        RustCodegen::emit_fstring(&mut emitter, &parts);
        let output = emitter.finish();
        assert!(output.contains("format!"));
        assert!(output.contains("{}"));
        assert!(output.contains("name"));
    }

    // ========================================
    // Field access tests
    // ========================================

    #[test]
    fn test_emit_field() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Field(
            Box::new(make_spanned(ident_expr("obj"))),
            "field".to_string(),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "obj.field");
    }

    #[test]
    fn test_emit_field_nested() {
        let mut emitter = RustEmitter::new();
        let inner = Expr::Field(
            Box::new(make_spanned(ident_expr("obj"))),
            "inner".to_string(),
        );
        let expr = Expr::Field(
            Box::new(make_spanned(inner)),
            "value".to_string(),
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "obj.inner.value");
    }

    // ========================================
    // Constructor tests
    // ========================================

    #[test]
    fn test_emit_constructor_no_args() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Constructor("None".to_string(), vec![]);
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("None"));
    }

    #[test]
    fn test_emit_constructor_some() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Constructor(
            "Some".to_string(),
            vec![CallArg::Positional(make_spanned(int_lit(42)))],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("Some"));
        assert!(output.contains("42"));
    }

    #[test]
    fn test_emit_constructor_ok() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Constructor(
            "Ok".to_string(),
            vec![CallArg::Positional(make_spanned(str_lit("success")))],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("Ok"));
    }

    #[test]
    fn test_emit_constructor_err() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Constructor(
            "Err".to_string(),
            vec![CallArg::Positional(make_spanned(str_lit("error")))],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("Err"));
    }

    // ========================================
    // Call tests
    // ========================================

    #[test]
    fn test_emit_call_no_args() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Call(
            Box::new(make_spanned(ident_expr("foo"))),
            vec![],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "foo()");
    }

    #[test]
    fn test_emit_call_with_args() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Call(
            Box::new(make_spanned(ident_expr("add"))),
            vec![
                CallArg::Positional(make_spanned(int_lit(1))),
                CallArg::Positional(make_spanned(int_lit(2))),
            ],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "add(1, 2)");
    }

    #[test]
    fn test_emit_call_println() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Call(
            Box::new(make_spanned(ident_expr("println"))),
            vec![CallArg::Positional(make_spanned(str_lit("hello")))],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("println!"));
    }

    #[test]
    fn test_emit_call_print() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Call(
            Box::new(make_spanned(ident_expr("print"))),
            vec![CallArg::Positional(make_spanned(str_lit("hello")))],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("print!"));
    }

    #[test]
    fn test_emit_call_len() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Call(
            Box::new(make_spanned(ident_expr("len"))),
            vec![CallArg::Positional(make_spanned(ident_expr("items")))],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("len()"));
    }

    #[test]
    fn test_emit_call_assert() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Call(
            Box::new(make_spanned(ident_expr("assert"))),
            vec![CallArg::Positional(make_spanned(bool_lit(true)))],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("assert!"));
    }

    #[test]
    fn test_emit_call_assert_eq() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Call(
            Box::new(make_spanned(ident_expr("assert_eq"))),
            vec![
                CallArg::Positional(make_spanned(int_lit(1))),
                CallArg::Positional(make_spanned(int_lit(1))),
            ],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("assert_eq!"));
    }

    #[test]
    fn test_emit_call_list() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Call(
            Box::new(make_spanned(ident_expr("list"))),
            vec![],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("Vec::new"));
    }

    #[test]
    fn test_emit_call_dict() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Call(
            Box::new(make_spanned(ident_expr("dict"))),
            vec![],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("HashMap::new"));
    }

    #[test]
    fn test_emit_call_set() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Call(
            Box::new(make_spanned(ident_expr("set"))),
            vec![],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("HashSet::new"));
    }

    // ========================================
    // Method call tests
    // ========================================

    #[test]
    fn test_emit_method_call_basic() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::MethodCall(
            Box::new(make_spanned(ident_expr("obj"))),
            "method".to_string(),
            vec![],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        assert_eq!(emitter.finish(), "obj.method()");
    }

    #[test]
    fn test_emit_method_call_with_args() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::MethodCall(
            Box::new(make_spanned(ident_expr("obj"))),
            "method".to_string(),
            vec![CallArg::Positional(make_spanned(int_lit(42)))],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("obj.method("));
        assert!(output.contains("42"));
    }

    #[test]
    fn test_emit_method_call_upper() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::MethodCall(
            Box::new(make_spanned(ident_expr("s"))),
            "upper".to_string(),
            vec![],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("to_uppercase"));
    }

    #[test]
    fn test_emit_method_call_lower() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::MethodCall(
            Box::new(make_spanned(ident_expr("s"))),
            "lower".to_string(),
            vec![],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("to_lowercase"));
    }

    #[test]
    fn test_emit_method_call_strip() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::MethodCall(
            Box::new(make_spanned(ident_expr("s"))),
            "strip".to_string(),
            vec![],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("trim"));
    }

    #[test]
    fn test_emit_method_call_append() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::MethodCall(
            Box::new(make_spanned(ident_expr("list"))),
            "append".to_string(),
            vec![CallArg::Positional(make_spanned(int_lit(42)))],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        // append may be translated or kept as-is depending on context
        assert!(output.contains("append") || output.contains("push"));
    }

    #[test]
    fn test_emit_method_call_pop() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::MethodCall(
            Box::new(make_spanned(ident_expr("list"))),
            "pop".to_string(),
            vec![],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("pop()"));
    }

    #[test]
    fn test_emit_method_call_contains() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::MethodCall(
            Box::new(make_spanned(ident_expr("s"))),
            "contains".to_string(),
            vec![CallArg::Positional(make_spanned(str_lit("test")))],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("contains"));
    }

    #[test]
    fn test_emit_method_call_replace() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::MethodCall(
            Box::new(make_spanned(ident_expr("s"))),
            "replace".to_string(),
            vec![
                CallArg::Positional(make_spanned(str_lit("old"))),
                CallArg::Positional(make_spanned(str_lit("new"))),
            ],
        );
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("replace"));
    }

    // ========================================
    // If expression tests
    // ========================================

    #[test]
    fn test_emit_if_expr_simple() {
        let mut emitter = RustEmitter::new();
        let if_expr = IfExpr {
            condition: make_spanned(bool_lit(true)),
            then_body: vec![make_spanned(crate::frontend::ast::Statement::Expr(make_spanned(int_lit(1))))],
            else_body: Some(vec![make_spanned(crate::frontend::ast::Statement::Expr(make_spanned(int_lit(0))))]),
        };
        let expr = Expr::If(Box::new(if_expr));
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("if"));
    }

    // ========================================
    // Match expression tests
    // ========================================

    #[test]
    fn test_emit_match_simple() {
        let mut emitter = RustEmitter::new();
        let arms = vec![
            make_spanned(crate::frontend::ast::MatchArm {
                pattern: make_spanned(crate::frontend::ast::Pattern::Wildcard),
                guard: None,
                body: crate::frontend::ast::MatchBody::Expr(make_spanned(int_lit(0))),
            }),
        ];
        let expr = Expr::Match(Box::new(make_spanned(ident_expr("x"))), arms);
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("match x"));
        assert!(output.contains("_"));
    }

    // ========================================
    // List comprehension tests
    // ========================================

    #[test]
    fn test_emit_list_comp_simple() {
        let mut emitter = RustEmitter::new();
        let comp = ListComp {
            expr: make_spanned(ident_expr("x")),
            var: "x".to_string(),
            iter: make_spanned(ident_expr("items")),
            filter: None,
        };
        let expr = Expr::ListComp(Box::new(comp));
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("iter()"));
        assert!(output.contains("map"));
        assert!(output.contains("collect"));
    }

    #[test]
    fn test_emit_list_comp_with_filter() {
        let mut emitter = RustEmitter::new();
        let comp = ListComp {
            expr: make_spanned(ident_expr("x")),
            var: "x".to_string(),
            iter: make_spanned(ident_expr("items")),
            filter: Some(make_spanned(Expr::Binary(
                Box::new(make_spanned(ident_expr("x"))),
                BinaryOp::Gt,
                Box::new(make_spanned(int_lit(0))),
            ))),
        };
        let expr = Expr::ListComp(Box::new(comp));
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("filter"));
    }

    // ========================================
    // Dict comprehension tests
    // ========================================

    #[test]
    fn test_emit_dict_comp_simple() {
        let mut emitter = RustEmitter::new();
        let comp = DictComp {
            key: make_spanned(ident_expr("k")),
            value: make_spanned(ident_expr("v")),
            var: "item".to_string(),
            iter: make_spanned(ident_expr("items")),
            filter: None,
        };
        let expr = Expr::DictComp(Box::new(comp));
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("iter()"));
        assert!(output.contains("collect"));
    }

    // ========================================
    // Yield tests
    // ========================================

    #[test]
    fn test_emit_yield_none() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Yield(None);
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("yield"));
    }

    #[test]
    fn test_emit_yield_value() {
        let mut emitter = RustEmitter::new();
        let expr = Expr::Yield(Some(Box::new(make_spanned(int_lit(42)))));
        RustCodegen::emit_expr(&mut emitter, &expr);
        let output = emitter.finish();
        assert!(output.contains("yield"));
        assert!(output.contains("42"));
    }
}
