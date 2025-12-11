//! Function and method emission for code generation
//!
//! Handles emitting functions, methods, and web handlers.

use crate::frontend::ast::*;
use crate::backend::rust_emitter::RustEmitter;

use super::RustCodegen;

/// The Zen of Incan - embedded at compile time from stdlib/zen.txt
const ZEN_OF_INCAN: &str = include_str!("../../../stdlib/zen.txt");

impl RustCodegen<'_> {
    /// Emit a function declaration
    pub(crate) fn emit_function(&mut self, func: &FunctionDecl) {
        // Check if this function has @route decorator
        let route_info = func.decorators.iter().find(|d| d.node.name == "route");

        if route_info.is_some() {
            self.emit_axum_handler(func);
            return;
        }

        let params = Self::format_function_params(&func.params);
        let ret_type = self.type_to_rust(&func.return_type.node);

        let is_main = func.name == "main";

        // Check if this is a test function
        let is_test = self.test_mode && func.name.starts_with("test_") &&
            self.test_function.as_ref().map_or(true, |tf| tf == &func.name);

        // Check for skip/xfail decorators
        let has_skip = func.decorators.iter().any(|d| d.node.name == "skip");

        // For async main with tokio, we add the #[tokio::main] attribute
        let is_tokio_main = is_main && (func.is_async || self.needs_tokio);

        let (vis, name, ret, use_exit_code) = if is_main {
            if ret_type == "i64" {
                ("", "main".to_string(), "std::process::ExitCode".to_string(), true)
            } else {
                ("", "main".to_string(), String::new(), false)
            }
        } else {
            ("pub", func.name.clone(), ret_type, false)
        };

        let use_exit = use_exit_code;
        let param_names: Vec<String> = func.params.iter().map(|p| p.node.name.clone()).collect();

        // Add #[test] attribute for test functions
        if is_test {
            if has_skip {
                self.emitter.line("#[ignore]");
            }
            self.emitter.line("#[test]");
        }

        // Special handling for main with web app
        if is_main && self.needs_axum && !self.routes.is_empty() {
            self.emit_web_main(func);
            return;
        }

        // Add #[tokio::main] attribute for async main
        if is_tokio_main {
            self.emitter.line("#[tokio::main]");
        }

        let is_async = func.is_async || is_tokio_main;

        // Capture whether to emit Zen before the closure
        let emit_zen = is_main && self.emit_zen_in_main;

        // Format type parameters for generic functions
        let type_params: Vec<String> = func.type_params.iter().cloned().collect();

        self.emitter.function_generic(vis, is_async, &name, &type_params, &params, &ret, |e| {
            e.push_scope();
            for param_name in &param_names {
                e.declare_var(param_name);
            }

            // Emit the Zen of Incan if `import this` was used
            // Source of truth: stdlib/zen.txt (embedded at compile time)
            if emit_zen {
                for line in ZEN_OF_INCAN.lines() {
                    // Escape quotes and backslashes for the generated Rust string
                    let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
                    e.line(&format!(r#"println!("{}");"#, escaped));
                }
            }

            let body_len = func.body.len();
            for (i, stmt) in func.body.iter().enumerate() {
                let is_last = i == body_len - 1;
                if is_last && use_exit {
                    if let Statement::Expr(expr) = &stmt.node {
                        e.write_indent();
                        e.write("std::process::ExitCode::from(");
                        Self::emit_expr(e, &expr.node);
                        e.write(" as u8)\n");
                    } else if let Statement::Return(Some(expr)) = &stmt.node {
                        e.write_indent();
                        e.write("std::process::ExitCode::from(");
                        Self::emit_expr(e, &expr.node);
                        e.write(" as u8)\n");
                    } else {
                        Self::emit_statement_maybe_return(e, &stmt.node, is_last && !ret.is_empty());
                    }
                } else {
                    Self::emit_statement_maybe_return(e, &stmt.node, is_last && !ret.is_empty());
                }
            }

            e.pop_scope();
        });
    }

    /// Emit a function with @route decorator as an axum handler
    pub(crate) fn emit_axum_handler(&mut self, func: &FunctionDecl) {
        let mut axum_params = Vec::new();

        for param in &func.params {
            let param_type = Self::type_to_rust_static(&param.node.ty.node);
            let param_name = &param.node.name;

            match &param.node.ty.node {
                Type::Simple(name) if name == "int" || name == "i64" || name == "str" || name == "String" => {
                    axum_params.push(format!("Path({}): Path<{}>", param_name, param_type));
                }
                Type::Generic(name, _) if name == "Query" => {
                    axum_params.push(format!("{}: {}", param_name, param_type));
                }
                Type::Generic(name, args) if name == "Json" => {
                    let inner = if !args.is_empty() {
                        Self::type_to_rust_static(&args[0].node)
                    } else {
                        "serde_json::Value".to_string()
                    };
                    axum_params.push(format!("Json({}): Json<{}>", param_name, inner));
                }
                _ => {
                    axum_params.push(format!("Path({}): Path<{}>", param_name, param_type));
                }
            }
        }

        let ret_type = self.type_to_rust(&func.return_type.node);
        let axum_ret = if ret_type.starts_with("Json<") {
            ret_type
        } else if ret_type == "Response" || ret_type.is_empty() {
            "impl IntoResponse".to_string()
        } else if ret_type.starts_with("Html") {
            "Html<String>".to_string()
        } else {
            "impl IntoResponse".to_string()
        };

        let params_str = axum_params.join(", ");
        let param_names: Vec<String> = func.params.iter().map(|p| p.node.name.clone()).collect();

        self.emitter.function("pub", true, &func.name, &params_str, &axum_ret, |e| {
            e.push_scope();
            for param_name in &param_names {
                e.declare_var(param_name);
            }

            let body_len = func.body.len();
            for (i, stmt) in func.body.iter().enumerate() {
                let is_last = i == body_len - 1;
                Self::emit_statement_maybe_return(e, &stmt.node, is_last);
            }

            e.pop_scope();
        });
    }

    /// Emit main function with web server setup
    pub(crate) fn emit_web_main(&mut self, func: &FunctionDecl) {
        let mut port_expr: Option<Spanned<Expr>> = None;
        let mut host_expr: Option<Spanned<Expr>> = None;
        
        // Collect user statements that should be emitted before server startup
        // Skip: `app = App()` and `app.run(...)`
        let mut user_statements: Vec<&Spanned<Statement>> = Vec::new();

        for stmt in &func.body {
            let mut is_app_init = false;
            let mut is_app_run = false;
            
            // Check if this is `app = App()`
            if let Statement::Assignment(assign) = &stmt.node {
                if assign.name == "app" {
                    if let Expr::Call(callee, _) = &assign.value.node {
                        if let Expr::Ident(func_name) = &callee.node {
                            if func_name == "App" {
                                is_app_init = true;
                            }
                        }
                    }
                }
            }
            
            // Check if this is `app.run(...)`
            if let Statement::Expr(expr) = &stmt.node {
                if let Expr::MethodCall(receiver, method, args) = &expr.node {
                    if method == "run" {
                        if let Expr::Ident(name) = &receiver.node {
                            if name == "app" {
                                is_app_run = true;
                                // Extract port/host from app.run()
                                for arg in args {
                                    match arg {
                                        CallArg::Named(name, expr) => {
                                            if name == "port" {
                                                port_expr = Some(expr.clone());
                                            } else if name == "host" {
                                                host_expr = Some(expr.clone());
                                            }
                                        }
                                        CallArg::Positional(expr) => {
                                            if port_expr.is_none() {
                                                port_expr = Some(expr.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            // Keep user statements that aren't app init or app.run
            if !is_app_init && !is_app_run {
                user_statements.push(stmt);
            }
        }

        let routes = self.routes.clone();

        self.emitter.line("#[tokio::main]");
        self.emitter.function("", true, "main", "", "", |e| {
            // Emit user statements first (before server setup)
            for stmt in &user_statements {
                Self::emit_statement(e, &stmt.node);
            }
            
            if !user_statements.is_empty() {
                e.blank_line();
            }
            
            // Host binding
            match &host_expr {
                Some(expr) => {
                    e.line("let host: String = {");
                    e.indent();
                    e.write_indent();
                    e.write("let tmp = ");
                    Self::emit_expr(e, &expr.node);
                    e.line(";");
                    e.write_indent();
                    e.line("tmp.to_string()");
                    e.dedent();
                    e.line("};");
                }
                None => {
                    e.line("let host: String = \"127.0.0.1\".to_string();");  // default host is 127.0.0.1
                }
            }

            // Port binding
            match &port_expr {
                Some(expr) => {
                    e.line("let port: u16 = {");
                    e.indent();
                    e.write_indent();
                    e.write("let tmp = ");
                    Self::emit_expr(e, &expr.node);
                    e.line(";");
                    e.write_indent();
                    e.line("tmp as u16");
                    e.dedent();
                    e.line("};");
                }
                None => {
                    e.line("let port: u16 = 8080u16;");  // default port is 8080
                }
            }

            e.blank_line();
            e.line("let app = Router::new()");
            e.indent();

            for route in &routes {
                let axum_path = route.path.replace("{", ":").replace("}", "");
                let method_fn = match route.methods.first().map(|s| s.as_str()) {
                    Some("GET") | None => "get",
                    Some("POST") => "post",
                    Some("PUT") => "put",
                    Some("DELETE") => "delete",
                    Some("PATCH") => "patch",
                    _ => "get",
                };
                e.line(&format!(".route(\"{}\", {}({}))", axum_path, method_fn, route.handler_name));
            }

            e.dedent();
            e.line(";");
            e.blank_line();

            e.line("let addr: std::net::SocketAddr = format!(\"{}:{}\", host, port)");
            e.indent();
            e.line(".parse()");
            e.line("    .expect(\"invalid host/port combination for binding\");");
            e.dedent();
            e.line("let listener = tokio::net::TcpListener::bind(addr).await");
            e.indent();
            e.line(".expect(\"failed to bind to address\");");
            e.dedent();
            e.line("axum::serve(listener, app).await");
            e.indent();
            e.line(".expect(\"server error\");");
            e.dedent();
        });
    }

    /// Emit a method in an impl block
    pub(crate) fn emit_method_in_impl(emitter: &mut RustEmitter, method: &MethodDecl) {
        Self::emit_method_with_visibility(emitter, method, "pub");
    }

    /// Emit a method in a trait impl (no visibility)
    pub(crate) fn emit_method_in_trait_impl(emitter: &mut RustEmitter, method: &MethodDecl) {
        Self::emit_method_with_visibility(emitter, method, "");
    }

    /// Emit a method with given visibility
    pub(crate) fn emit_method_with_visibility(emitter: &mut RustEmitter, method: &MethodDecl, visibility: &str) {
        let params = Self::format_params(&method.receiver, &method.params);
        let ret_type = Self::type_to_rust_static(&method.return_type.node);
        let param_names: Vec<String> = method.params.iter().map(|p| p.node.name.clone()).collect();

        emitter.function(visibility, method.is_async, &method.name, &params, &ret_type, |e| {
            e.push_scope();
            for param_name in &param_names {
                e.declare_var(param_name);
            }

            if let Some(body) = &method.body {
                for stmt in body {
                    Self::emit_statement(e, &stmt.node);
                }
            } else {
                e.line("todo!()");
            }

            e.pop_scope();
        });
    }

    /// Emit a trait method
    pub(crate) fn emit_trait_method(emitter: &mut RustEmitter, method: &MethodDecl) {
        let params = Self::format_params(&method.receiver, &method.params);
        let ret_type = Self::type_to_rust_static(&method.return_type.node);
        let async_str = if method.is_async { "async " } else { "" };

        if method.body.is_some() {
            emitter.function("", method.is_async, &method.name, &params, &ret_type, |e| {
                if let Some(body) = &method.body {
                    for stmt in body {
                        Self::emit_statement(e, &stmt.node);
                    }
                }
            });
        } else {
            emitter.line(&format!(
                "{}fn {}({}) -> {};",
                async_str,
                method.name,
                params,
                ret_type
            ));
        }
    }
}
