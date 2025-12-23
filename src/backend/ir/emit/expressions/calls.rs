//! Emit Rust code for function calls and binary operations.
//!
//! This module handles emission of regular function calls (user-defined functions) and binary operator expressions.

use proc_macro2::TokenStream;
use quote::quote;

use super::super::super::conversions::{ConversionContext, determine_conversion};
use super::super::super::expr::{BinOp, IrExprKind, TypedExpr, VarAccess};
use super::super::super::types::{IrType, Mutability};
use super::super::{EmitError, IrEmitter};

impl<'a> IrEmitter<'a> {
    /// Emit a function call expression.
    ///
    /// Handles regular function calls (user-defined functions).
    /// Built-in functions are handled by `emit_builtin_call` or `try_emit_builtin_call`.
    pub(in super::super) fn emit_call_expr(
        &self,
        func: &TypedExpr,
        args: &[TypedExpr],
    ) -> Result<TokenStream, EmitError> {
        // Handle builtin functions specially (legacy string-based path)
        if let IrExprKind::Var { name, .. } = &func.kind {
            if let Some(result) = self.try_emit_builtin_call(name, args)? {
                return Ok(result);
            }
        }

        let f = self.emit_expr(func)?;

        // Look up function signature
        let function_sig = if let IrExprKind::Var { name, .. } = &func.kind {
            self.function_registry.get(name)
        } else {
            None
        };

        // Handle argument passing with signature-based borrow insertion
        let arg_tokens: Vec<TokenStream> = args
            .iter()
            .enumerate()
            .map(|(idx, a)| {
                let emitted = self.emit_expr(a)?;

                // Check VarAccess for explicit borrow requirements
                if let IrExprKind::Var { access, .. } = &a.kind {
                    match access {
                        VarAccess::BorrowMut => return Ok(quote! { &mut #emitted }),
                        VarAccess::Borrow => return Ok(quote! { &#emitted }),
                        _ => {}
                    }
                }

                // If we have function signature, use it to determine borrows
                if let Some(sig) = function_sig {
                    if let Some(param) = sig.params.get(idx) {
                        if param.mutability == Mutability::Mutable {
                            match &a.ty {
                                IrType::Ref(_) | IrType::RefMut(_) => return Ok(emitted),
                                _ => return Ok(quote! { &mut #emitted }),
                            }
                        }
                        if matches!(&param.ty, IrType::Ref(_)) {
                            match &a.ty {
                                IrType::Ref(_) | IrType::RefMut(_) => return Ok(emitted),
                                _ => {
                                    if !a.ty.is_copy() {
                                        return Ok(quote! { &#emitted });
                                    }
                                }
                            }
                        }
                    }
                }

                // Determine conversion context based on whether this is an Incan or Rust function
                let context = if let IrExprKind::Var { name, .. } = &func.kind {
                    // External Rust functions: either explicit rust:: prefix or imported from Rust crate
                    if name.starts_with("rust::") || self.external_rust_functions.contains(name) {
                        ConversionContext::ExternalFunctionArg
                    } else {
                        ConversionContext::IncanFunctionArg
                    }
                } else {
                    ConversionContext::IncanFunctionArg
                };

                let target_ty = function_sig
                    .and_then(|sig| sig.params.get(idx))
                    .map(|param| &param.ty);

                let conversion = determine_conversion(a, target_ty, context);
                Ok(conversion.apply(emitted))
            })
            .collect::<Result<_, _>>()?;

        Ok(quote! { #f(#(#arg_tokens),*) })
    }

    /// Emit a binary operation expression.
    pub(in super::super) fn emit_binop_expr(
        &self,
        op: &BinOp,
        left: &TypedExpr,
        right: &TypedExpr,
    ) -> Result<TokenStream, EmitError> {
        // Special-case: const-fold string additions using literals/known consts
        if matches!(op, BinOp::Add) {
            if let Some(tokens) = self.try_emit_static_str_add(left, right)? {
                return Ok(tokens);
            }
        }

        let l = self.emit_expr(left)?;
        let r = self.emit_expr(right)?;

        // Power operator is a method call, not an infix operator
        if matches!(op, BinOp::Pow) {
            match &left.ty {
                IrType::Float => return Ok(quote! { #l.powf(#r) }),
                IrType::Int => return Ok(quote! { #l.pow(#r as u32) }),
                _ => return Ok(quote! { #l.powf(#r) }),
            }
        }

        let op_tokens = self.emit_binop(op);

        // Handle reference vs value comparisons
        let is_comparison = matches!(
            op,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        );

        if is_comparison {
            let left_is_ref = matches!(&left.ty, IrType::Ref(_) | IrType::RefMut(_));
            let right_is_value = !matches!(&right.ty, IrType::Ref(_) | IrType::RefMut(_));

            if left_is_ref && right_is_value {
                return Ok(quote! { *#l #op_tokens #r });
            }
        }

        Ok(quote! { #l #op_tokens #r })
    }
}
