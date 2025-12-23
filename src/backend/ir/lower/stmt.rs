//! Statement lowering for AST to IR conversion.
//!
//! This module handles lowering of all statement types: let bindings,
//! assignments, control flow (if/while/for), and returns.

use std::collections::HashMap;

use super::super::expr::{IrExprKind, Pattern, VarAccess};
use super::super::stmt::{AssignTarget, IrStmt, IrStmtKind};
use super::super::types::IrType;
use super::super::{IrSpan, Mutability, TypedExpr};
use super::AstLowering;
use super::errors::LoweringError;
use crate::frontend::ast::{self, Spanned};

impl AstLowering {
    /// Lower a list of statements to IR.
    ///
    /// # Parameters
    ///
    /// * `stmts` - The AST statements to lower
    ///
    /// # Returns
    ///
    /// A vector of IR statements.
    ///
    /// # Errors
    ///
    /// Returns `LoweringError` if any statement cannot be lowered.
    pub(super) fn lower_statements(
        &mut self,
        stmts: &[Spanned<ast::Statement>],
    ) -> Result<Vec<IrStmt>, LoweringError> {
        let mut result = Vec::new();
        for s in stmts {
            let stmt = self.lower_statement(&s.node)?;
            result.push(stmt);
        }
        Ok(result)
    }

    /// Lower a single statement to IR.
    ///
    /// Handles all statement types including:
    /// - Expression statements
    /// - Let bindings (mutable and immutable)
    /// - Assignments (variable, field, index)
    /// - Control flow (if/elif/else, while, for)
    /// - Returns, break, continue, pass
    /// - Compound assignments (+=, -=, etc.)
    /// - Tuple unpacking
    /// - Chained assignments
    ///
    /// # Parameters
    ///
    /// * `stmt` - The AST statement to lower
    ///
    /// # Returns
    ///
    /// The corresponding IR statement.
    ///
    /// # Errors
    ///
    /// Returns `LoweringError` if the statement cannot be lowered.
    pub(super) fn lower_statement(
        &mut self,
        stmt: &ast::Statement,
    ) -> Result<IrStmt, LoweringError> {
        let kind = match stmt {
            ast::Statement::Expr(e) => IrStmtKind::Expr(self.lower_expr_spanned(e)?),

            ast::Statement::Assignment(a) => {
                let value = self.lower_expr_spanned(&a.value)?;
                let ty =
                    a.ty.as_ref()
                        .map(|t| self.lower_type(&t.node))
                        .unwrap_or_else(|| value.ty.clone());

                match a.binding {
                    ast::BindingKind::Reassign => {
                        return Ok(IrStmt::new(IrStmtKind::Assign {
                            target: AssignTarget::Var(a.name.clone()),
                            value,
                        }));
                    }
                    ast::BindingKind::Inferred => {
                        // Check if the variable exists in ANY scope (innermost to outermost).
                        // This allows reassignment of outer scope variables from nested scopes.
                        let var_exists_in_scope =
                            self.scopes.iter().rev().any(|s| s.contains_key(&a.name));

                        if var_exists_in_scope {
                            let is_mut = self.mutable_vars.get(&a.name).copied().unwrap_or(false);
                            if is_mut {
                                return Ok(IrStmt::new(IrStmtKind::Assign {
                                    target: AssignTarget::Var(a.name.clone()),
                                    value,
                                }));
                            } else {
                                return Err(LoweringError {
                                    message: format!(
                                        "Cannot reassign immutable variable '{}'",
                                        a.name
                                    ),
                                    span: IrSpan::default(),
                                });
                            }
                        }
                        // Otherwise, create a new immutable binding in the current scope.
                        if let Some(scope) = self.scopes.last_mut() {
                            scope.insert(a.name.clone(), ty.clone());
                        }
                        IrStmtKind::Let {
                            name: a.name.clone(),
                            ty,
                            mutability: Mutability::Immutable,
                            value,
                        }
                    }
                    ast::BindingKind::Mutable => {
                        // New mutable binding
                        self.mutable_vars.insert(a.name.clone(), true);
                        if let Some(scope) = self.scopes.last_mut() {
                            scope.insert(a.name.clone(), ty.clone());
                        }
                        IrStmtKind::Let {
                            name: a.name.clone(),
                            ty,
                            mutability: Mutability::Mutable,
                            value,
                        }
                    }
                    ast::BindingKind::Let => {
                        // New immutable binding
                        if let Some(scope) = self.scopes.last_mut() {
                            scope.insert(a.name.clone(), ty.clone());
                        }
                        IrStmtKind::Let {
                            name: a.name.clone(),
                            ty,
                            mutability: Mutability::Immutable,
                            value,
                        }
                    }
                }
            }

            ast::Statement::FieldAssignment(fa) => IrStmtKind::Assign {
                target: AssignTarget::Field {
                    object: Box::new(self.lower_expr_spanned(&fa.object)?),
                    field: fa.field.clone(),
                },
                value: self.lower_expr_spanned(&fa.value)?,
            },

            ast::Statement::IndexAssignment(ia) => IrStmtKind::Assign {
                target: AssignTarget::Index {
                    object: Box::new(self.lower_expr_spanned(&ia.object)?),
                    index: Box::new(self.lower_expr_spanned(&ia.index)?),
                },
                value: self.lower_expr_spanned(&ia.value)?,
            },

            ast::Statement::Return(opt) => IrStmtKind::Return(
                opt.as_ref()
                    .map(|e| self.lower_expr_spanned(e))
                    .transpose()?,
            ),

            ast::Statement::If(i) => {
                // Lower elif branches as nested if-else in the else branch
                // Each branch gets its own scope
                let mut else_branch = i
                    .else_body
                    .as_ref()
                    .map(|b| {
                        self.scopes.push(HashMap::new());
                        let result = self.lower_statements(b);
                        self.scopes.pop();
                        result
                    })
                    .transpose()?;

                // Build elif chain from end to start
                for (elif_cond, elif_body) in i.elif_branches.iter().rev() {
                    self.scopes.push(HashMap::new());
                    let elif_then = self.lower_statements(elif_body)?;
                    self.scopes.pop();
                    let elif_stmt = IrStmtKind::If {
                        condition: self.lower_expr_spanned(elif_cond)?,
                        then_branch: elif_then,
                        else_branch,
                    };
                    else_branch = Some(vec![IrStmt::new(elif_stmt)]);
                }

                let condition = self.lower_expr_spanned(&i.condition)?;
                self.scopes.push(HashMap::new());
                let then_branch = self.lower_statements(&i.then_body)?;
                self.scopes.pop();

                IrStmtKind::If {
                    condition,
                    then_branch,
                    else_branch,
                }
            }

            ast::Statement::While(w) => {
                // Push a new scope for the while-loop body
                self.scopes.push(HashMap::new());
                let condition = self.lower_expr_spanned(&w.condition)?;
                let body = self.lower_statements(&w.body)?;
                self.scopes.pop();
                IrStmtKind::While {
                    label: None,
                    condition,
                    body,
                }
            }

            ast::Statement::For(f) => {
                // Lower iterable before entering loop scope
                let iterable = self.lower_expr_spanned(&f.iter)?;

                // Push a new scope for the for-loop body
                self.scopes.push(HashMap::new());

                // Infer loop variable type from iterable and add to scope
                let loop_var_ty = match &iterable.ty {
                    IrType::List(elem) => (**elem).clone(),
                    IrType::Dict(k, _) => (**k).clone(),
                    IrType::String => IrType::String,
                    _ => IrType::Unknown,
                };
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(f.var.clone(), loop_var_ty);
                }

                let body = self.lower_statements(&f.body)?;
                self.scopes.pop();

                IrStmtKind::For {
                    label: None,
                    pattern: Pattern::Var(f.var.clone()),
                    iterable,
                    body,
                }
            }

            ast::Statement::Pass => {
                IrStmtKind::Expr(TypedExpr::new(IrExprKind::Unit, IrType::Unit))
            }
            ast::Statement::Break => IrStmtKind::Break(None),
            ast::Statement::Continue => IrStmtKind::Continue(None),

            ast::Statement::CompoundAssignment(ca) => IrStmtKind::CompoundAssign {
                target: AssignTarget::Var(ca.name.clone()),
                op: self.lower_compound_op(&ca.op),
                value: self.lower_expr_spanned(&ca.value)?,
            },

            ast::Statement::TupleUnpack(tu) => {
                let value = self.lower_expr_spanned(&tu.value)?;
                IrStmtKind::Let {
                    name: tu.names.join("_"),
                    ty: value.ty.clone(),
                    mutability: match tu.binding {
                        ast::BindingKind::Mutable => Mutability::Mutable,
                        _ => Mutability::Immutable,
                    },
                    value,
                }
            }

            ast::Statement::TupleAssign(_) => {
                return Err(LoweringError {
                    message: "TupleAssign not yet implemented".to_string(),
                    span: IrSpan::default(),
                });
            }

            ast::Statement::ChainedAssignment(ca) => {
                // Lower chained assignment x = y = z = 5 into:
                // let z = 5; let y = z; let x = y;
                // We return a block expression that does all the assignments
                let value = self.lower_expr_spanned(&ca.value)?;
                let ty = value.ty.clone();

                // Assign to last target first (rightmost)
                let last_target = match ca.targets.last() {
                    Some(t) => t,
                    None => {
                        return Err(LoweringError {
                            message: "empty chained assignment".to_string(),
                            span: IrSpan::default(),
                        });
                    }
                };
                let mutability = match ca.binding {
                    ast::BindingKind::Mutable => Mutability::Mutable,
                    _ => Mutability::Immutable,
                };

                // Record the last target in scope
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(last_target.clone(), ty.clone());
                }

                // Create the first assignment statement
                let mut stmts = vec![IrStmt::new(IrStmtKind::Let {
                    name: last_target.clone(),
                    ty: ty.clone(),
                    mutability,
                    value,
                })];

                // Now assign to each previous target from the next one
                for i in (0..ca.targets.len() - 1).rev() {
                    let target = &ca.targets[i];
                    let source = &ca.targets[i + 1];

                    if let Some(scope) = self.scopes.last_mut() {
                        scope.insert(target.clone(), ty.clone());
                    }

                    let source_expr = TypedExpr::new(
                        IrExprKind::Var {
                            name: source.clone(),
                            access: if ty.is_copy() {
                                VarAccess::Copy
                            } else {
                                VarAccess::Move
                            },
                        },
                        ty.clone(),
                    );

                    stmts.push(IrStmt::new(IrStmtKind::Let {
                        name: target.clone(),
                        ty: ty.clone(),
                        mutability,
                        value: source_expr,
                    }));
                }

                // Return a block that does all the assignments and returns unit
                return Ok(IrStmt::new(IrStmtKind::Expr(TypedExpr::new(
                    IrExprKind::Block { stmts, value: None },
                    IrType::Unit,
                ))));
            }
        };
        Ok(IrStmt::new(kind))
    }
}
