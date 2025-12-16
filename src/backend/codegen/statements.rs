//! Statement emission for code generation
//!
//! Handles emitting all statement types to Rust.

use crate::frontend::ast::*;
use crate::backend::rust_emitter::{RustEmitter, to_rust_ident, CollectionKind};

use super::RustCodegen;

impl RustCodegen<'_> {
    /// Emit a statement
    pub(crate) fn emit_statement(emitter: &mut RustEmitter, stmt: &Statement) {
        Self::emit_statement_maybe_return(emitter, stmt, false);
    }

    /// Emit a statement, optionally as an implicit return
    pub(crate) fn emit_statement_maybe_return(emitter: &mut RustEmitter, stmt: &Statement, is_implicit_return: bool) {
        match stmt {
            Statement::Assignment(assign) => Self::emit_assignment(emitter, assign),
            Statement::FieldAssignment(field_assign) => Self::emit_field_assignment(emitter, field_assign),
            Statement::IndexAssignment(index_assign) => Self::emit_index_assignment(emitter, index_assign),
            Statement::Return(expr) => Self::emit_return(emitter, expr.as_ref(), is_implicit_return),
            Statement::If(if_stmt) => Self::emit_if(emitter, if_stmt),
            Statement::While(while_stmt) => Self::emit_while(emitter, while_stmt),
            Statement::For(for_stmt) => Self::emit_for(emitter, for_stmt),
            Statement::Expr(expr) => {
                emitter.write_indent();
                Self::emit_expr(emitter, &expr.node);
                // If this is the last statement and we have a return type, don't add semicolon
                if is_implicit_return {
                    emitter.write("\n");
                } else {
                    emitter.write(";\n");
                }
            }
            Statement::Pass => {
                // Empty statement in Rust is just nothing, or we can use ()
            }
            Statement::Break => {
                emitter.line("break;");
            }
            Statement::Continue => {
                emitter.line("continue;");
            }
            Statement::CompoundAssignment(compound) => {
                emitter.write_indent();
                emitter.write(&to_rust_ident(&compound.name));
                emitter.write(match compound.op {
                    CompoundOp::Add => " += ",
                    CompoundOp::Sub => " -= ",
                    CompoundOp::Mul => " *= ",
                    CompoundOp::Div => " /= ",
                    CompoundOp::Mod => " %= ",
                });
                Self::emit_expr(emitter, &compound.value.node);
                emitter.write(";\n");
            }
            Statement::TupleUnpack(unpack) => {
                emitter.write_indent();
                // Emit: let (a, b, c) = expr;
                if matches!(unpack.binding, BindingKind::Mutable) {
                    emitter.write("let (");
                    for (i, name) in unpack.names.iter().enumerate() {
                        if i > 0 {
                            emitter.write(", ");
                        }
                        emitter.write("mut ");
                        emitter.write(&to_rust_ident(name));
                    }
                    emitter.write(")");
                } else {
                    emitter.write("let (");
                    for (i, name) in unpack.names.iter().enumerate() {
                        if i > 0 {
                            emitter.write(", ");
                        }
                        emitter.write(&to_rust_ident(name));
                    }
                    emitter.write(")");
                }
                emitter.write(" = ");
                Self::emit_expr(emitter, &unpack.value.node);
                emitter.write(";\n");
                // Declare the variables in scope
                for name in &unpack.names {
                    emitter.declare_var(name);
                }
            }
            Statement::TupleAssign(assign) => {
                // Tuple assignment to lvalue expressions: arr[i], arr[j] = arr[j], arr[i]
                // Strategy: evaluate RHS into temps first, then assign to targets
                // This ensures correct swap semantics
                
                // If value is a tuple, unpack it into temp variables
                if let Expr::Tuple(rhs_elems) = &assign.value.node {
                    // Generate unique temp names
                    let temp_names: Vec<String> = (0..rhs_elems.len())
                        .map(|i| format!("_swap_tmp_{}", i))
                        .collect();
                    
                    // Step 1: Evaluate each RHS element into temps
                    for (i, (temp_name, rhs_elem)) in temp_names.iter().zip(rhs_elems.iter()).enumerate() {
                        emitter.write_indent();
                        // Use .clone() for the temp if we need the value again
                        emitter.write(&format!("let {} = ", temp_name));
                        Self::emit_expr(emitter, &rhs_elem.node);
                        // Clone if it's an index expression to avoid borrow conflicts
                        if matches!(rhs_elem.node, Expr::Index(_, _)) {
                            emitter.write(".clone()");
                        }
                        emitter.write(";\n");
                        let _ = i; // suppress unused warning
                    }
                    
                    // Step 2: Assign temps to LHS targets
                    for (target, temp_name) in assign.targets.iter().zip(temp_names.iter()) {
                        emitter.write_indent();
                        Self::emit_lvalue(emitter, &target.node);
                        emitter.write(&format!(" = {};\n", temp_name));
                    }
                } else {
                    // RHS is not a tuple literal - evaluate the whole thing first
                    emitter.write_indent();
                    emitter.write("let _swap_rhs = ");
                    Self::emit_expr(emitter, &assign.value.node);
                    emitter.write(";\n");
                    
                    // Assign each target from the tuple
                    for (i, target) in assign.targets.iter().enumerate() {
                        emitter.write_indent();
                        Self::emit_lvalue(emitter, &target.node);
                        emitter.write(&format!(" = _swap_rhs.{};\n", i));
                    }
                }
            }
        }
    }

    /// Check if an expression is an await on a lock/read/write method (returns a mutable guard)
    fn is_guard_expression(expr: &Expr) -> bool {
        if let Expr::Await(inner) = expr {
            if let Expr::MethodCall(_, method, _) = &inner.node {
                return matches!(method.as_str(), "lock" | "read" | "write");
            }
        }
        false
    }

    /// Emit an assignment statement
    pub(crate) fn emit_assignment(emitter: &mut RustEmitter, assign: &AssignmentStmt) {
        emitter.write_indent();

        // Determine if this is a declaration or reassignment
        let is_reassign = match assign.binding {
            BindingKind::Reassign => true,
            BindingKind::Inferred => {
                // For Inferred bindings, check if variable is already declared
                emitter.is_var_declared(&assign.name)
            }
            _ => false,
        };

        // Check if we need a mutable binding (for mutex guards)
        let needs_mut = matches!(assign.binding, BindingKind::Mutable) 
            || Self::is_guard_expression(&assign.value.node);

        if is_reassign {
            // Reassignment - no let keyword
        } else {
            // Declaration - emit let or let mut
            if needs_mut {
                emitter.write("let mut ");
                // If this is a mutable collection, register it for &mut passing at call sites
                let is_collection = if let Some(ty) = &assign.ty {
                    Self::is_collection_type(&ty.node)
                } else {
                    // Infer from value expression
                    Self::expr_is_collection(&assign.value.node)
                };
                if is_collection {
                    emitter.register_mut_collection_var(&assign.name);
                }
            } else {
                emitter.write("let ");
            }
            // Track the variable as declared
            emitter.declare_var(&assign.name);
        }

        emitter.write(&to_rust_ident(&assign.name));

        if let Some(ty) = &assign.ty {
            emitter.write(": ");
            emitter.write(&Self::type_to_rust_static(&ty.node));
            // Track known collection kind from explicit type annotation
            if let Type::Generic(name, _args) = &ty.node {
                match name.as_str() {
                    "List" => emitter.register_collection_kind(&assign.name, CollectionKind::List),
                    "Dict" => emitter.register_collection_kind(&assign.name, CollectionKind::Dict),
                    "Set" => emitter.register_collection_kind(&assign.name, CollectionKind::Set),
                    _ => {}
                }
            } else if let Type::Simple(name) = &ty.node {
                if name == "str" || name == "String" {
                    emitter.register_string_var(&assign.name);
                }
            }
        }

        emitter.write(" = ");
        Self::emit_expr(emitter, &assign.value.node);
        // Auto-convert string literals to owned String
        if matches!(&assign.value.node, Expr::Literal(Literal::String(_))) {
            emitter.write(".to_string()");
            emitter.register_string_var(&assign.name);
        } else {
            // Infer collection kind (and basic string-ness) from the assigned expression for unannotated vars
            match &assign.value.node {
                Expr::List(_) | Expr::ListComp(_) => {
                    emitter.register_collection_kind(&assign.name, CollectionKind::List);
                }
                Expr::Dict(_) | Expr::DictComp(_) => {
                    emitter.register_collection_kind(&assign.name, CollectionKind::Dict);
                }
                Expr::Set(_) => {
                    emitter.register_collection_kind(&assign.name, CollectionKind::Set);
                }
                Expr::FString(_) => {
                    emitter.register_string_var(&assign.name);
                }
                _ => {}
            }
        }
        emitter.write(";\n");
    }

    /// Emit a field assignment statement
    pub(crate) fn emit_field_assignment(emitter: &mut RustEmitter, field_assign: &FieldAssignmentStmt) {
        emitter.write_indent();
        // Use emit_lvalue for the object to avoid .clone() on indexed access
        Self::emit_lvalue(emitter, &field_assign.object.node);
        emitter.write(".");
        emitter.write(&to_rust_ident(&field_assign.field));
        emitter.write(" = ");
        Self::emit_expr(emitter, &field_assign.value.node);
        emitter.write(";\n");
    }

    /// Emit an index assignment statement
    pub(crate) fn emit_index_assignment(emitter: &mut RustEmitter, index_assign: &IndexAssignmentStmt) {
        emitter.write_indent();
        // For vectors/lists, use bracket notation: vec[index] = value
        // For HashMaps with string keys, use .insert()
        let is_string_key = matches!(&index_assign.index.node, Expr::Literal(Literal::String(_)));
        
        if is_string_key {
            // HashMap string key: dict.insert("key", value)
            Self::emit_expr(emitter, &index_assign.object.node);
            emitter.write(".insert(");
            Self::emit_expr(emitter, &index_assign.index.node);
            emitter.write(".to_string()");
            emitter.write(", ");
            Self::emit_expr(emitter, &index_assign.value.node);
            emitter.write(");\n");
        } else {
            // Vector/list integer index: vec[index as usize] = value
            Self::emit_expr(emitter, &index_assign.object.node);
            emitter.write("[(");
            Self::emit_expr(emitter, &index_assign.index.node);
            emitter.write(" as usize)] = ");
            Self::emit_expr(emitter, &index_assign.value.node);
            emitter.write(";\n");
        }
    }

    /// Emit a return statement
    pub(crate) fn emit_return(emitter: &mut RustEmitter, expr: Option<&Spanned<Expr>>, is_implicit: bool) {
        emitter.write_indent();
        if let Some(e) = expr {
            // Optimization: in a return position, we can safely move distinct identifier arguments
            // into a call (no need to clone) as long as the callee does not expect &mut params.
            //
            // This avoids pathological deep clones like `merge(left.clone(), right.clone())`
            // in recursive algorithms (e.g. mergesort benchmark).
            if let Expr::Call(callee, args) = &e.node {
                if let Expr::Ident(func_name) = &callee.node {
                    // IMPORTANT: don't apply this optimization to builtins.
                    // Builtins like `sum(x)` lower to Rust expressions and are not real Rust functions.
                    // If we bypass normal expression emission here, we end up emitting `sum(x)` literally,
                    // which fails to compile (e.g. primes benchmark).
                    if matches!(
                        func_name.as_str(),
                        "len"
                            | "sum"
                            | "range"
                            | "print"
                            | "println"
                            | "assert"
                            | "assert_eq"
                            | "assert_ne"
                            | "assert_true"
                            | "assert_false"
                            | "fail"
                            | "int"
                            | "float"
                            | "str"
                            | "json_stringify"
                            | "json_parse"
                            | "read_file"
                            | "write_file"
                            | "sleep"
                            | "sleep_ms"
                            | "timeout"
                    ) {
                        // Fall back to the regular path so the builtin lowering runs.
                    } else {
                    let mut seen = std::collections::HashSet::new();
                    let mut all_distinct_idents = true;
                    let mut any_mut_param = false;

                    for (idx, arg) in args.iter().enumerate() {
                        if emitter.function_param_is_mut(func_name, idx) {
                            any_mut_param = true;
                            break;
                        }
                        match arg {
                            CallArg::Positional(a) => {
                                if let Expr::Ident(name) = &a.node {
                                    if !seen.insert(name.clone()) {
                                        all_distinct_idents = false;
                                        break;
                                    }
                                } else {
                                    all_distinct_idents = false;
                                    break;
                                }
                            }
                            // Named args are more complex; fall back to the regular path.
                            CallArg::Named(_, _) => {
                                all_distinct_idents = false;
                                break;
                            }
                        }
                    }

                    if all_distinct_idents && !any_mut_param {
                        if !is_implicit {
                            emitter.write("return ");
                        }
                        Self::emit_expr(emitter, &callee.node);
                        emitter.write("(");
                        for (i, arg) in args.iter().enumerate() {
                            if i > 0 {
                                emitter.write(", ");
                            }
                            if let CallArg::Positional(a) = arg {
                                // Safe: distinct identifier args in return position; move into the call.
                                Self::emit_expr(emitter, &a.node);
                            }
                        }
                        emitter.write(")");
                        if !is_implicit {
                            emitter.write(";\n");
                        } else {
                            emitter.write("\n");
                        }
                        return;
                    }
                    }
                }
            }

            // For non-implicit returns (inside if/while/etc), use explicit return
            if !is_implicit {
                emitter.write("return ");
                Self::emit_expr(emitter, &e.node);
                // Add .to_string() for string literals
                if matches!(&e.node, Expr::Literal(Literal::String(_))) {
                    emitter.write(".to_string()");
                }
                emitter.write(";\n");
            } else {
                // Implicit return at end of function (no semicolon)
                Self::emit_expr(emitter, &e.node);
                // Add .to_string() for string literals
                if matches!(&e.node, Expr::Literal(Literal::String(_))) {
                    emitter.write(".to_string()");
                }
                emitter.write("\n");
            }
        } else {
            if is_implicit {
                emitter.write("()\n"); // Return unit
            } else {
                emitter.write("return;\n");
            }
        }
    }

    /// Emit an if statement
    pub(crate) fn emit_if(emitter: &mut RustEmitter, if_stmt: &IfStmt) {
        emitter.write_indent();
        emitter.write("if ");
        Self::emit_expr(emitter, &if_stmt.condition.node);
        emitter.write(" {\n");
        emitter.indent();
        emitter.push_scope();
        for stmt in &if_stmt.then_body {
            Self::emit_statement(emitter, &stmt.node);
        }
        emitter.pop_scope();
        emitter.dedent();
        emitter.write_indent();
        emitter.write("}");

        if let Some(else_body) = &if_stmt.else_body {
            emitter.write(" else {\n");
            emitter.indent();
            emitter.push_scope();
            for stmt in else_body {
                Self::emit_statement(emitter, &stmt.node);
            }
            emitter.pop_scope();
            emitter.dedent();
            emitter.write_indent();
            emitter.write("}");
        }
        emitter.write("\n");
    }

    /// Emit a while statement
    pub(crate) fn emit_while(emitter: &mut RustEmitter, while_stmt: &WhileStmt) {
        emitter.write_indent();
        emitter.write("while ");
        Self::emit_expr(emitter, &while_stmt.condition.node);
        emitter.write(" {\n");
        emitter.indent();
        emitter.push_scope();
        for stmt in &while_stmt.body {
            Self::emit_statement(emitter, &stmt.node);
        }
        emitter.pop_scope();
        emitter.dedent();
        emitter.line("}");
    }

    /// Whether the loop variable is mutated inside the loop body.
    ///
    /// Used to decide between `.iter().cloned()` (read-only iteration) and `.iter_mut()` (mutating elements through the loop variable).
    fn loop_var_mutated_in_body(loop_var: &str, body: &[Spanned<Statement>]) -> bool {
        fn expr_is_or_starts_with_var(loop_var: &str, e: &Expr) -> bool {
            match e {
                Expr::Ident(name) => name == loop_var,
                Expr::Field(base, _) => expr_is_or_starts_with_var(loop_var, &base.node),
                Expr::Index(base, _) => expr_is_or_starts_with_var(loop_var, &base.node),
                Expr::Slice(base, _) => expr_is_or_starts_with_var(loop_var, &base.node),
                _ => false,
            }
        }

        for stmt in body {
            match &stmt.node {
                // Mutating through loop var: x.field = ..., x[i] = ...
                Statement::FieldAssignment(s) => {
                    if expr_is_or_starts_with_var(loop_var, &s.object.node) {
                        return true;
                    }
                }
                Statement::IndexAssignment(s) => {
                    if expr_is_or_starts_with_var(loop_var, &s.object.node) {
                        return true;
                    }
                }
                Statement::TupleAssign(s) => {
                    // (x.field, ...) = ... or (x[i], ...) = ...
                    if s.targets
                        .iter()
                        .any(|t| expr_is_or_starts_with_var(loop_var, &t.node))
                    {
                        return true;
                    }
                }

                // Reassigning / compound-assigning the loop var itself
                Statement::Assignment(s) => {
                    if s.name == loop_var && matches!(s.binding, BindingKind::Reassign) {
                        return true;
                    }
                }
                Statement::CompoundAssignment(s) => {
                    if s.name == loop_var {
                        return true;
                    }
                }

                // Recurse into nested control flow
                Statement::If(s) => {
                    if Self::loop_var_mutated_in_body(loop_var, &s.then_body) {
                        return true;
                    }
                    if let Some(else_body) = &s.else_body {
                        if Self::loop_var_mutated_in_body(loop_var, else_body) {
                            return true;
                        }
                    }
                }
                Statement::While(s) => {
                    if Self::loop_var_mutated_in_body(loop_var, &s.body) {
                        return true;
                    }
                }
                Statement::For(s) => {
                    if Self::loop_var_mutated_in_body(loop_var, &s.body) {
                        return true;
                    }
                }
                _ => {}
            }
        }

        false
    }

    /// Emit a for statement
    pub(crate) fn emit_for(emitter: &mut RustEmitter, for_stmt: &ForStmt) {
        emitter.write_indent();
        emitter.write("for ");
        
        let loop_var = to_rust_ident(&for_stmt.var);
        
        // Check if the iterator is a call to range() and emit Rust range syntax
        if let Expr::Call(callee, args) = &for_stmt.iter.node {
            if let Expr::Ident(name) = &callee.node {
                if name == "range" {
                    // Range loops: loop variable is typically immutable (just a counter)
                    emitter.write(&loop_var);
                    emitter.write(" in ");
                    Self::emit_range_call(emitter, args);
                    emitter.write(" {\n");
                    emitter.indent();
                    Self::emit_for_body(emitter, &for_stmt.var, &for_stmt.body);
                    emitter.dedent();
                    emitter.line("}");
                    return;
                }
            }
        }

        // For-each loops
        emitter.write(&loop_var);
        emitter.write(" in ");

        // Determine how to iterate based on the expression type
        Self::emit_expr(emitter, &for_stmt.iter.node);

        // Method calls (like str.split_whitespace()) return iterators directly
        // Range expressions are already iterators
        // Lists/temporary collections can use .into_iter() for owned values.
        // Identifiers holding collections should only use `.iter_mut()` if the loop body actually mutates elements through the loop variable. Otherwise prefer
        // `.iter().cloned()` so read-only loops work even when the collection binding is immutable.
        match &for_stmt.iter.node {
            Expr::MethodCall(_, _, _) => {
                // Method calls typically return iterators, no suffix needed
            }
            Expr::Range { .. } => {
                // Range expressions (0..n, start..end) are already iterators
            }
            Expr::List(_) => {
                // List literals - use .into_iter() for owned values
                emitter.write(".into_iter()");
            }
            Expr::Call(_, _) => {
                // Calls like enumerate(...) / zip(...) often lower to temporary Vecs.
                // Use .into_iter() so the loop variable owns the element (avoids borrow/move issues).
                emitter.write(".into_iter()");
            }
            Expr::Ident(_name) => {
                let needs_mut = Self::loop_var_mutated_in_body(&for_stmt.var, &for_stmt.body);
                if needs_mut {
                    emitter.write(".iter_mut()");
                } else {
                    emitter.write(".iter().cloned()");
                }
            }
            _ => {
                emitter.write(".iter().cloned()");
            }
        }

        emitter.write(" {\n");
        emitter.indent();
        Self::emit_for_body(emitter, &for_stmt.var, &for_stmt.body);
        emitter.dedent();
        emitter.line("}");
    }

    /// Emit the body of a for loop
    pub(crate) fn emit_for_body(emitter: &mut RustEmitter, loop_var: &str, body: &[Spanned<Statement>]) {
        // Enter for loop scope and declare the loop variable
        emitter.push_scope();
        emitter.declare_var(loop_var);

        for stmt in body {
            Self::emit_statement(emitter, &stmt.node);
        }

        emitter.pop_scope();
    }

    /// Emit a range() call as Rust range syntax
    pub(crate) fn emit_range_call(emitter: &mut RustEmitter, args: &[CallArg]) {
        // range(n) -> 0..n
        // range(a, b) -> a..b
        // range(a, b, step) -> (a..b).step_by(step)
        // range(start..end) -> start..end
        match args.len() {
            1 => {
                // range(n) -> 0..n
                // range(start..end) -> start..end
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
            }
            2 => {
                // range(a, b) -> a..b
                if let CallArg::Positional(a) = &args[0] {
                    Self::emit_expr(emitter, &a.node);
                }
                emitter.write("..");
                if let CallArg::Positional(b) = &args[1] {
                    Self::emit_expr(emitter, &b.node);
                }
            }
            3 => {
                // range(a, b, step) -> (a..b).step_by(step as usize)
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
            }
            _ => {
                // Fallback - just emit as function call
                emitter.write("range(");
                Self::emit_call_args(emitter, args);
                emitter.write(")");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rust_emitter::RustEmitter;

    fn make_spanned<T>(node: T) -> Spanned<T> {
        Spanned { node, span: Span::default() }
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

    fn ident(s: &str) -> Expr {
        Expr::Ident(s.to_string())
    }

    // ========================================
    // Assignment statement tests
    // ========================================

    #[test]
    fn test_emit_assignment_let() {
        let mut emitter = RustEmitter::new();
        let assign = AssignmentStmt {
            binding: BindingKind::Let,
            name: "x".to_string(),
            ty: None,
            value: make_spanned(int_lit(42)),
        };
        RustCodegen::emit_assignment(&mut emitter, &assign);
        let output = emitter.finish();
        assert!(output.contains("let x = 42"));
    }

    #[test]
    fn test_emit_assignment_let_mut() {
        let mut emitter = RustEmitter::new();
        let assign = AssignmentStmt {
            binding: BindingKind::Mutable,
            name: "x".to_string(),
            ty: None,
            value: make_spanned(int_lit(0)),
        };
        RustCodegen::emit_assignment(&mut emitter, &assign);
        let output = emitter.finish();
        assert!(output.contains("let mut x = 0"));
    }

    #[test]
    fn test_emit_assignment_with_type() {
        let mut emitter = RustEmitter::new();
        let assign = AssignmentStmt {
            binding: BindingKind::Let,
            name: "count".to_string(),
            ty: Some(make_spanned(Type::Simple("int".to_string()))),
            value: make_spanned(int_lit(0)),
        };
        RustCodegen::emit_assignment(&mut emitter, &assign);
        let output = emitter.finish();
        assert!(output.contains("let count"));
        assert!(output.contains("i64"));
    }

    #[test]
    fn test_emit_assignment_string_adds_to_string() {
        let mut emitter = RustEmitter::new();
        let assign = AssignmentStmt {
            binding: BindingKind::Let,
            name: "s".to_string(),
            ty: None,
            value: make_spanned(str_lit("hello")),
        };
        RustCodegen::emit_assignment(&mut emitter, &assign);
        let output = emitter.finish();
        assert!(output.contains(".to_string()"));
    }

    #[test]
    fn test_emit_assignment_reassign() {
        let mut emitter = RustEmitter::new();
        emitter.push_scope();
        emitter.declare_var("x");
        let assign = AssignmentStmt {
            binding: BindingKind::Reassign,
            name: "x".to_string(),
            ty: None,
            value: make_spanned(int_lit(100)),
        };
        RustCodegen::emit_assignment(&mut emitter, &assign);
        let output = emitter.finish();
        // No let keyword for reassignment
        assert!(!output.contains("let"));
        assert!(output.contains("x = 100"));
    }

    #[test]
    fn test_emit_assignment_inferred_new() {
        let mut emitter = RustEmitter::new();
        emitter.push_scope();
        let assign = AssignmentStmt {
            binding: BindingKind::Inferred,
            name: "y".to_string(),
            ty: None,
            value: make_spanned(int_lit(5)),
        };
        RustCodegen::emit_assignment(&mut emitter, &assign);
        let output = emitter.finish();
        // New variable with inferred gets let
        assert!(output.contains("let y = 5"));
    }

    #[test]
    fn test_emit_assignment_inferred_existing() {
        let mut emitter = RustEmitter::new();
        emitter.push_scope();
        emitter.declare_var("y");
        let assign = AssignmentStmt {
            binding: BindingKind::Inferred,
            name: "y".to_string(),
            ty: None,
            value: make_spanned(int_lit(10)),
        };
        RustCodegen::emit_assignment(&mut emitter, &assign);
        let output = emitter.finish();
        // Existing variable - no let
        assert!(!output.contains("let"));
    }

    #[test]
    fn test_emit_assignment_reserved_name() {
        let mut emitter = RustEmitter::new();
        let assign = AssignmentStmt {
            binding: BindingKind::Let,
            name: "type".to_string(),
            ty: None,
            value: make_spanned(int_lit(1)),
        };
        RustCodegen::emit_assignment(&mut emitter, &assign);
        let output = emitter.finish();
        assert!(output.contains("r#type"));
    }

    // ========================================
    // Field assignment tests
    // ========================================

    #[test]
    fn test_emit_field_assignment() {
        let mut emitter = RustEmitter::new();
        let field_assign = FieldAssignmentStmt {
            object: make_spanned(ident("obj")),
            field: "value".to_string(),
            value: make_spanned(int_lit(42)),
        };
        RustCodegen::emit_field_assignment(&mut emitter, &field_assign);
        let output = emitter.finish();
        assert!(output.contains("obj.value = 42"));
    }

    #[test]
    fn test_emit_field_assignment_nested() {
        let mut emitter = RustEmitter::new();
        let inner = Expr::Field(
            Box::new(make_spanned(ident("self"))),
            "inner".to_string(),
        );
        let field_assign = FieldAssignmentStmt {
            object: make_spanned(inner),
            field: "count".to_string(),
            value: make_spanned(int_lit(0)),
        };
        RustCodegen::emit_field_assignment(&mut emitter, &field_assign);
        let output = emitter.finish();
        assert!(output.contains("self.inner.count = 0"));
    }

    // ========================================
    // Index assignment tests
    // ========================================

    #[test]
    fn test_emit_index_assignment() {
        let mut emitter = RustEmitter::new();
        let index_assign = IndexAssignmentStmt {
            object: make_spanned(ident("dict")),
            index: make_spanned(str_lit("key")),
            value: make_spanned(int_lit(42)),
        };
        RustCodegen::emit_index_assignment(&mut emitter, &index_assign);
        let output = emitter.finish();
        assert!(output.contains("dict.insert"));
        assert!(output.contains(".to_string()"));
    }

    #[test]
    fn test_emit_index_assignment_int_key() {
        let mut emitter = RustEmitter::new();
        let index_assign = IndexAssignmentStmt {
            object: make_spanned(ident("arr")),
            index: make_spanned(int_lit(0)),
            value: make_spanned(str_lit("value")),
        };
        RustCodegen::emit_index_assignment(&mut emitter, &index_assign);
        let output = emitter.finish();
        // Integer indices use bracket notation with usize cast
        assert!(output.contains("arr[(0 as usize)] = "));
    }

    // ========================================
    // Return statement tests
    // ========================================

    #[test]
    fn test_emit_return_explicit() {
        let mut emitter = RustEmitter::new();
        let expr = make_spanned(int_lit(42));
        RustCodegen::emit_return(&mut emitter, Some(&expr), false);
        let output = emitter.finish();
        assert!(output.contains("return 42;"));
    }

    #[test]
    fn test_emit_return_implicit() {
        let mut emitter = RustEmitter::new();
        let expr = make_spanned(int_lit(42));
        RustCodegen::emit_return(&mut emitter, Some(&expr), true);
        let output = emitter.finish();
        // Implicit return - no return keyword or semicolon
        assert!(!output.contains("return"));
        assert!(!output.contains(";"));
        assert!(output.contains("42"));
    }

    #[test]
    fn test_emit_return_none_explicit() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_return(&mut emitter, None, false);
        let output = emitter.finish();
        assert!(output.contains("return;"));
    }

    #[test]
    fn test_emit_return_none_implicit() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_return(&mut emitter, None, true);
        let output = emitter.finish();
        assert!(output.contains("()"));
    }

    #[test]
    fn test_emit_return_string_adds_to_string() {
        let mut emitter = RustEmitter::new();
        let expr = make_spanned(str_lit("hello"));
        RustCodegen::emit_return(&mut emitter, Some(&expr), false);
        let output = emitter.finish();
        assert!(output.contains(".to_string()"));
    }

    #[test]
    fn test_emit_return_builtin_sum_is_lowered() {
        let mut emitter = RustEmitter::new();
        let expr = make_spanned(Expr::Call(
            Box::new(make_spanned(ident("sum"))),
            vec![CallArg::Positional(make_spanned(ident("values")))],
        ));
        RustCodegen::emit_return(&mut emitter, Some(&expr), false);
        let output = emitter.finish();
        // Should lower `sum(values)` instead of emitting a raw `sum(...)` Rust call.
        assert!(!output.contains("sum("));
        assert!(output.contains("values.iter()"));
    }

    // ========================================
    // If statement tests
    // ========================================

    #[test]
    fn test_emit_if_simple() {
        let mut emitter = RustEmitter::new();
        let if_stmt = IfStmt {
            condition: make_spanned(bool_lit(true)),
            then_body: vec![make_spanned(Statement::Pass)],
            else_body: None,
        };
        RustCodegen::emit_if(&mut emitter, &if_stmt);
        let output = emitter.finish();
        assert!(output.contains("if true"));
        assert!(output.contains("{"));
        assert!(output.contains("}"));
    }

    #[test]
    fn test_emit_if_with_else() {
        let mut emitter = RustEmitter::new();
        let if_stmt = IfStmt {
            condition: make_spanned(ident("cond")),
            then_body: vec![make_spanned(Statement::Break)],
            else_body: Some(vec![make_spanned(Statement::Continue)]),
        };
        RustCodegen::emit_if(&mut emitter, &if_stmt);
        let output = emitter.finish();
        assert!(output.contains("if cond"));
        assert!(output.contains("break;"));
        assert!(output.contains("else"));
        assert!(output.contains("continue;"));
    }

    #[test]
    fn test_emit_if_with_complex_condition() {
        let mut emitter = RustEmitter::new();
        let cond = Expr::Binary(
            Box::new(make_spanned(ident("x"))),
            BinaryOp::Gt,
            Box::new(make_spanned(int_lit(0))),
        );
        let if_stmt = IfStmt {
            condition: make_spanned(cond),
            then_body: vec![],
            else_body: None,
        };
        RustCodegen::emit_if(&mut emitter, &if_stmt);
        let output = emitter.finish();
        assert!(output.contains("if"));
        assert!(output.contains("x > 0"));
    }

    // ========================================
    // While statement tests
    // ========================================

    #[test]
    fn test_emit_while_simple() {
        let mut emitter = RustEmitter::new();
        let while_stmt = WhileStmt {
            condition: make_spanned(bool_lit(true)),
            body: vec![make_spanned(Statement::Break)],
        };
        RustCodegen::emit_while(&mut emitter, &while_stmt);
        let output = emitter.finish();
        assert!(output.contains("while true"));
        assert!(output.contains("break;"));
    }

    #[test]
    fn test_emit_while_with_condition() {
        let mut emitter = RustEmitter::new();
        let cond = Expr::Binary(
            Box::new(make_spanned(ident("i"))),
            BinaryOp::Lt,
            Box::new(make_spanned(int_lit(10))),
        );
        let while_stmt = WhileStmt {
            condition: make_spanned(cond),
            body: vec![],
        };
        RustCodegen::emit_while(&mut emitter, &while_stmt);
        let output = emitter.finish();
        assert!(output.contains("while"));
        assert!(output.contains("i < 10"));
    }

    // ========================================
    // For statement tests
    // ========================================

    #[test]
    fn test_emit_for_simple() {
        let mut emitter = RustEmitter::new();
        let for_stmt = ForStmt {
            var: "x".to_string(),
            iter: make_spanned(ident("items")),
            body: vec![],
        };
        RustCodegen::emit_for(&mut emitter, &for_stmt);
        let output = emitter.finish();
        assert!(output.contains("for x in"));
        assert!(output.contains("items.iter().cloned()"));
    }

    #[test]
    fn test_emit_for_uses_iter_mut_when_loop_var_mutated() {
        let mut emitter = RustEmitter::new();
        let for_stmt = ForStmt {
            var: "item".to_string(),
            iter: make_spanned(ident("items")),
            body: vec![make_spanned(Statement::FieldAssignment(FieldAssignmentStmt {
                object: make_spanned(ident("item")),
                field: "x".to_string(),
                value: make_spanned(int_lit(1)),
            }))],
        };
        RustCodegen::emit_for(&mut emitter, &for_stmt);
        let output = emitter.finish();
        assert!(output.contains("for item in"));
        assert!(output.contains("items.iter_mut()"));
    }

    #[test]
    fn test_emit_for_with_list_literal() {
        let mut emitter = RustEmitter::new();
        let for_stmt = ForStmt {
            var: "i".to_string(),
            iter: make_spanned(Expr::List(vec![
                make_spanned(int_lit(1)),
                make_spanned(int_lit(2)),
                make_spanned(int_lit(3)),
            ])),
            body: vec![],
        };
        RustCodegen::emit_for(&mut emitter, &for_stmt);
        let output = emitter.finish();
        assert!(output.contains("for i in"));
        assert!(output.contains("vec![1, 2, 3]"));
        assert!(output.contains(".into_iter()"));
    }

    #[test]
    fn test_emit_for_with_range_one_arg() {
        let mut emitter = RustEmitter::new();
        let for_stmt = ForStmt {
            var: "i".to_string(),
            iter: make_spanned(Expr::Call(
                Box::new(make_spanned(ident("range"))),
                vec![CallArg::Positional(make_spanned(int_lit(10)))],
            )),
            body: vec![],
        };
        RustCodegen::emit_for(&mut emitter, &for_stmt);
        let output = emitter.finish();
        assert!(output.contains("for i in 0..10"));
    }

    #[test]
    fn test_emit_for_with_range_two_args() {
        let mut emitter = RustEmitter::new();
        let for_stmt = ForStmt {
            var: "i".to_string(),
            iter: make_spanned(Expr::Call(
                Box::new(make_spanned(ident("range"))),
                vec![
                    CallArg::Positional(make_spanned(int_lit(5))),
                    CallArg::Positional(make_spanned(int_lit(15))),
                ],
            )),
            body: vec![],
        };
        RustCodegen::emit_for(&mut emitter, &for_stmt);
        let output = emitter.finish();
        assert!(output.contains("for i in 5..15"));
    }

    #[test]
    fn test_emit_for_with_range_three_args() {
        let mut emitter = RustEmitter::new();
        let for_stmt = ForStmt {
            var: "i".to_string(),
            iter: make_spanned(Expr::Call(
                Box::new(make_spanned(ident("range"))),
                vec![
                    CallArg::Positional(make_spanned(int_lit(0))),
                    CallArg::Positional(make_spanned(int_lit(10))),
                    CallArg::Positional(make_spanned(int_lit(2))),
                ],
            )),
            body: vec![],
        };
        RustCodegen::emit_for(&mut emitter, &for_stmt);
        let output = emitter.finish();
        assert!(output.contains("step_by"));
    }

    #[test]
    fn test_emit_for_with_used_variable() {
        let mut emitter = RustEmitter::new();
        // Create a for loop where the variable is actually used
        let for_stmt = ForStmt {
            var: "x".to_string(),
            iter: make_spanned(Expr::Call(
                Box::new(make_spanned(ident("range"))),
                vec![CallArg::Positional(make_spanned(int_lit(5)))],
            )),
            body: vec![make_spanned(Statement::Expr(make_spanned(ident("x"))))],
        };
        RustCodegen::emit_for(&mut emitter, &for_stmt);
        let output = emitter.finish();
        // Used variable should NOT have _ prefix
        assert!(output.contains("for x in 0..5"));
        assert!(!output.contains("for _x in"));
    }

    // ========================================
    // Simple statement tests
    // ========================================

    #[test]
    fn test_emit_statement_pass() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_statement(&mut emitter, &Statement::Pass);
        let output = emitter.finish();
        // Pass is empty in Rust
        assert!(output.is_empty() || output.trim().is_empty());
    }

    #[test]
    fn test_emit_statement_break() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_statement(&mut emitter, &Statement::Break);
        let output = emitter.finish();
        assert!(output.contains("break;"));
    }

    #[test]
    fn test_emit_statement_continue() {
        let mut emitter = RustEmitter::new();
        RustCodegen::emit_statement(&mut emitter, &Statement::Continue);
        let output = emitter.finish();
        assert!(output.contains("continue;"));
    }

    #[test]
    fn test_emit_statement_expr() {
        let mut emitter = RustEmitter::new();
        let stmt = Statement::Expr(make_spanned(Expr::Call(
            Box::new(make_spanned(ident("do_something"))),
            vec![],
        )));
        RustCodegen::emit_statement(&mut emitter, &stmt);
        let output = emitter.finish();
        assert!(output.contains("do_something()"));
        assert!(output.contains(";"));
    }

    #[test]
    fn test_emit_statement_expr_implicit_return() {
        let mut emitter = RustEmitter::new();
        let stmt = Statement::Expr(make_spanned(int_lit(42)));
        RustCodegen::emit_statement_maybe_return(&mut emitter, &stmt, true);
        let output = emitter.finish();
        // No semicolon for implicit return
        assert!(!output.contains(";"));
    }

    // ========================================
    // Compound assignment tests
    // ========================================

    #[test]
    fn test_emit_compound_add() {
        let mut emitter = RustEmitter::new();
        let stmt = Statement::CompoundAssignment(CompoundAssignmentStmt {
            name: "x".to_string(),
            op: CompoundOp::Add,
            value: make_spanned(int_lit(5)),
        });
        RustCodegen::emit_statement(&mut emitter, &stmt);
        let output = emitter.finish();
        assert!(output.contains("x += 5"));
    }

    #[test]
    fn test_emit_compound_sub() {
        let mut emitter = RustEmitter::new();
        let stmt = Statement::CompoundAssignment(CompoundAssignmentStmt {
            name: "counter".to_string(),
            op: CompoundOp::Sub,
            value: make_spanned(int_lit(1)),
        });
        RustCodegen::emit_statement(&mut emitter, &stmt);
        let output = emitter.finish();
        assert!(output.contains("counter -= 1"));
    }

    #[test]
    fn test_emit_compound_mul() {
        let mut emitter = RustEmitter::new();
        let stmt = Statement::CompoundAssignment(CompoundAssignmentStmt {
            name: "total".to_string(),
            op: CompoundOp::Mul,
            value: make_spanned(int_lit(2)),
        });
        RustCodegen::emit_statement(&mut emitter, &stmt);
        let output = emitter.finish();
        assert!(output.contains("total *= 2"));
    }

    #[test]
    fn test_emit_compound_div() {
        let mut emitter = RustEmitter::new();
        let stmt = Statement::CompoundAssignment(CompoundAssignmentStmt {
            name: "value".to_string(),
            op: CompoundOp::Div,
            value: make_spanned(int_lit(10)),
        });
        RustCodegen::emit_statement(&mut emitter, &stmt);
        let output = emitter.finish();
        assert!(output.contains("value /= 10"));
    }

    #[test]
    fn test_emit_compound_mod() {
        let mut emitter = RustEmitter::new();
        let stmt = Statement::CompoundAssignment(CompoundAssignmentStmt {
            name: "n".to_string(),
            op: CompoundOp::Mod,
            value: make_spanned(int_lit(3)),
        });
        RustCodegen::emit_statement(&mut emitter, &stmt);
        let output = emitter.finish();
        assert!(output.contains("n %= 3"));
    }

    // ========================================
    // Range call tests
    // ========================================

    #[test]
    fn test_emit_range_call_one_arg() {
        let mut emitter = RustEmitter::new();
        let args = vec![CallArg::Positional(make_spanned(int_lit(5)))];
        RustCodegen::emit_range_call(&mut emitter, &args);
        let output = emitter.finish();
        assert_eq!(output, "0..5");
    }

    #[test]
    fn test_emit_range_call_two_args() {
        let mut emitter = RustEmitter::new();
        let args = vec![
            CallArg::Positional(make_spanned(int_lit(1))),
            CallArg::Positional(make_spanned(int_lit(10))),
        ];
        RustCodegen::emit_range_call(&mut emitter, &args);
        let output = emitter.finish();
        assert_eq!(output, "1..10");
    }

    #[test]
    fn test_emit_range_call_three_args() {
        let mut emitter = RustEmitter::new();
        let args = vec![
            CallArg::Positional(make_spanned(int_lit(0))),
            CallArg::Positional(make_spanned(int_lit(100))),
            CallArg::Positional(make_spanned(int_lit(10))),
        ];
        RustCodegen::emit_range_call(&mut emitter, &args);
        let output = emitter.finish();
        assert!(output.contains("0..100"));
        assert!(output.contains("step_by"));
        assert!(output.contains("10"));
    }
}
