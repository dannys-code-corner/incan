//! Emit Rust code for built-in function calls.
//!
//! This module handles emission of known built-in functions using enum-based dispatch
//! (`BuiltinFn`). It also contains the legacy string-based fallback for `Call` expressions
//! that haven't been lowered to `BuiltinCall`.

use proc_macro2::TokenStream;
use quote::quote;

use super::super::super::expr::{BuiltinFn, IrExprKind, TypedExpr};
use super::super::super::types::IrType;
use super::super::{EmitError, IrEmitter};

impl<'a> IrEmitter<'a> {
    /// Emit a builtin function call using enum-based dispatch.
    ///
    /// This handles calls that have been lowered to `IrExprKind::BuiltinCall`.
    ///
    /// ## Parameters
    /// - `func`: The builtin function enum variant
    /// - `args`: The call arguments
    ///
    /// ## Returns
    /// - A Rust `TokenStream` for the builtin call
    pub(in super::super) fn emit_builtin_call(
        &self,
        func: &BuiltinFn,
        args: &[TypedExpr],
    ) -> Result<TokenStream, EmitError> {
        match func {
            BuiltinFn::Print => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { println!("{}", #a) })
                } else {
                    Ok(quote! { println!() })
                }
            }
            BuiltinFn::Len => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #a.len() as i64 })
                } else {
                    Ok(quote! { 0i64 })
                }
            }
            BuiltinFn::Sum => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    let elem_type = match &arg.ty {
                        IrType::List(elem) => elem.as_ref(),
                        IrType::Ref(inner) | IrType::RefMut(inner) => match inner.as_ref() {
                            IrType::List(elem) => elem.as_ref(),
                            _ => &IrType::Unknown,
                        },
                        _ => &IrType::Unknown,
                    };
                    match elem_type {
                        IrType::Bool => Ok(quote! {
                            (#a.iter().map(|x| if *x { 1i64 } else { 0i64 }).sum::<i64>())
                        }),
                        _ => Ok(quote! { (#a.iter().sum::<i64>()) }),
                    }
                } else {
                    Ok(quote! { 0i64 })
                }
            }
            BuiltinFn::Str => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #a.to_string() })
                } else {
                    Ok(quote! { String::new() })
                }
            }
            BuiltinFn::Int => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    match &arg.ty {
                        IrType::String => Ok(quote! { #a.parse::<i64>().unwrap() }),
                        IrType::Float => Ok(quote! { #a as i64 }),
                        IrType::Bool => Ok(quote! { if #a { 1 } else { 0 } }),
                        _ => Ok(quote! { #a as i64 }),
                    }
                } else {
                    Ok(quote! { 0i64 })
                }
            }
            BuiltinFn::Float => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    match &arg.ty {
                        IrType::String => Ok(quote! { #a.parse::<f64>().unwrap() }),
                        IrType::Int => Ok(quote! { #a as f64 }),
                        _ => Ok(quote! { #a as f64 }),
                    }
                } else {
                    Ok(quote! { 0.0f64 })
                }
            }
            BuiltinFn::Abs => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #a.abs() })
                } else {
                    Ok(quote! { 0 })
                }
            }
            BuiltinFn::Range => self
                .emit_range_call(args)
                .map(|opt| opt.unwrap_or_else(|| quote! { 0..0 })),
            BuiltinFn::Enumerate => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #a.iter().enumerate() })
                } else {
                    Ok(quote! { std::iter::empty::<(usize, ())>() })
                }
            }
            BuiltinFn::Zip => {
                if args.len() >= 2 {
                    let a = self.emit_expr(&args[0])?;
                    let b = self.emit_expr(&args[1])?;
                    Ok(quote! { #a.iter().zip(#b.iter()) })
                } else {
                    Ok(quote! { std::iter::empty::<((), ())>() })
                }
            }
            BuiltinFn::ReadFile => {
                if let Some(arg) = args.first() {
                    let path = self.emit_expr(arg)?;
                    Ok(quote! { std::fs::read_to_string(#path) })
                } else {
                    Ok(
                        quote! { Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "no path")) },
                    )
                }
            }
            BuiltinFn::WriteFile => {
                if args.len() >= 2 {
                    let path = self.emit_expr(&args[0])?;
                    let content = self.emit_expr(&args[1])?;
                    Ok(quote! { std::fs::write(#path, #content).map(|_| ()) })
                } else {
                    Ok(
                        quote! { Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing args")) },
                    )
                }
            }
            BuiltinFn::JsonStringify => {
                if let Some(arg) = args.first() {
                    let value = self.emit_expr(arg)?;
                    Ok(quote! { serde_json::to_string(&#value).unwrap() })
                } else {
                    Ok(quote! { String::from("null") })
                }
            }
            BuiltinFn::Sleep => {
                if let Some(arg) = args.first() {
                    let duration_arg = self.emit_expr(arg)?;
                    Ok(
                        quote! { tokio::time::sleep(tokio::time::Duration::from_secs_f64(#duration_arg)) },
                    )
                } else {
                    Ok(quote! { tokio::time::sleep(tokio::time::Duration::from_secs(0)) })
                }
            }
        }
    }

    /// Try to emit a builtin function call (legacy string-based dispatch).
    ///
    /// This is a fallback for `IrExprKind::Call` expressions where the function name
    /// matches a known builtin. Prefer using `emit_builtin_call` with enum dispatch.
    pub(in super::super) fn try_emit_builtin_call(
        &self,
        name: &str,
        args: &[TypedExpr],
    ) -> Result<Option<TokenStream>, EmitError> {
        match name {
            "print" | "println" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(Some(quote! { println!("{}", #a) }))
                } else {
                    Ok(Some(quote! { println!() }))
                }
            }
            "len" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(Some(quote! { #a.len() as i64 }))
                } else {
                    Ok(None)
                }
            }
            "sum" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    let elem_type = match &arg.ty {
                        IrType::List(elem) => elem.as_ref(),
                        IrType::Ref(inner) | IrType::RefMut(inner) => match inner.as_ref() {
                            IrType::List(elem) => elem.as_ref(),
                            _ => &IrType::Unknown,
                        },
                        _ => &IrType::Unknown,
                    };

                    match elem_type {
                        IrType::Bool => Ok(Some(quote! {
                            (#a.iter().map(|x| if *x { 1i64 } else { 0i64 }).sum::<i64>())
                        })),
                        _ => Ok(Some(quote! { (#a.iter().sum::<i64>()) })),
                    }
                } else {
                    Ok(None)
                }
            }
            "str" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(Some(quote! { #a.to_string() }))
                } else {
                    Ok(None)
                }
            }
            "int" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    match &arg.ty {
                        IrType::String => Ok(Some(quote! { #a.parse::<i64>().unwrap() })),
                        IrType::Float => Ok(Some(quote! { #a as i64 })),
                        IrType::Bool => Ok(Some(quote! { if #a { 1 } else { 0 } })),
                        _ => Ok(Some(quote! { #a as i64 })),
                    }
                } else {
                    Ok(None)
                }
            }
            "float" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    match &arg.ty {
                        IrType::String => Ok(Some(quote! { #a.parse::<f64>().unwrap() })),
                        IrType::Int => Ok(Some(quote! { #a as f64 })),
                        _ => Ok(Some(quote! { #a as f64 })),
                    }
                } else {
                    Ok(None)
                }
            }
            "abs" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(Some(quote! { #a.abs() }))
                } else {
                    Ok(None)
                }
            }
            "range" => self.emit_range_call(args),
            "enumerate" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(Some(quote! { #a.iter().enumerate() }))
                } else {
                    Ok(None)
                }
            }
            "zip" => {
                if args.len() >= 2 {
                    let a = self.emit_expr(&args[0])?;
                    let b = self.emit_expr(&args[1])?;
                    Ok(Some(quote! { #a.iter().zip(#b.iter()) }))
                } else {
                    Ok(None)
                }
            }
            "read_file" => {
                if let Some(arg) = args.first() {
                    let path = self.emit_expr(arg)?;
                    Ok(Some(quote! { std::fs::read_to_string(#path) }))
                } else {
                    Ok(None)
                }
            }
            "write_file" => {
                if args.len() >= 2 {
                    let path = self.emit_expr(&args[0])?;
                    let content = self.emit_expr(&args[1])?;
                    Ok(Some(quote! { std::fs::write(#path, #content).map(|_| ()) }))
                } else {
                    Ok(None)
                }
            }
            "json_stringify" => {
                if let Some(arg) = args.first() {
                    let value = self.emit_expr(arg)?;
                    Ok(Some(quote! { serde_json::to_string(&#value).unwrap() }))
                } else {
                    Ok(None)
                }
            }
            "sleep" => {
                if let Some(arg) = args.first() {
                    let duration_arg = self.emit_expr(arg)?;
                    Ok(Some(
                        quote! { tokio::time::sleep(tokio::time::Duration::from_secs_f64(#duration_arg)) },
                    ))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    /// Emit a range() function call.
    pub(in super::super) fn emit_range_call(
        &self,
        args: &[TypedExpr],
    ) -> Result<Option<TokenStream>, EmitError> {
        if args.len() == 1 {
            if let IrExprKind::Range {
                start,
                end,
                inclusive,
            } = &args[0].kind
            {
                match (start, end, inclusive) {
                    (Some(s), Some(e), false) => {
                        let ss = self.emit_expr(s)?;
                        let ee = self.emit_expr(e)?;
                        return Ok(Some(quote! { #ss..#ee }));
                    }
                    (Some(s), Some(e), true) => {
                        let ss = self.emit_expr(s)?;
                        let ee = self.emit_expr(e)?;
                        return Ok(Some(quote! { #ss..=#ee }));
                    }
                    (None, Some(e), _) => {
                        let ee = self.emit_expr(e)?;
                        return Ok(Some(quote! { 0..#ee }));
                    }
                    _ => {}
                }
            } else {
                let end = self.emit_expr(&args[0])?;
                return Ok(Some(quote! { 0..#end }));
            }
        }
        match args.len() {
            2 => {
                let start = self.emit_expr(&args[0])?;
                let end = self.emit_expr(&args[1])?;
                Ok(Some(quote! { #start..#end }))
            }
            3 => {
                let start = self.emit_expr(&args[0])?;
                let end = self.emit_expr(&args[1])?;
                let step = self.emit_expr(&args[2])?;
                Ok(Some(quote! { (#start..#end).step_by(#step as usize) }))
            }
            _ => Ok(None),
        }
    }
}
