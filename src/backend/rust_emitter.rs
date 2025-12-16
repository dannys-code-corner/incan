//! Rust code emitter - generates Rust source code strings
//!
//! This module provides utilities for building well-formatted Rust code.

use std::fmt::Write;
use std::collections::{HashSet, HashMap};

/// Known collection kinds for local variables (used for better codegen decisions)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionKind {
    List,
    Dict,
    Set,
}

/// Information about a function's parameters
#[derive(Debug, Clone, Default)]
pub struct FunctionParamInfo {
    /// Which parameter indices are mutable collection parameters (expect &mut)
    pub mut_params: HashSet<usize>,
}

/// A buffer for building Rust source code with proper indentation
#[derive(Debug, Default)]
pub struct RustEmitter {
    buffer: String,
    indent_level: usize,
    indent_str: &'static str,
    /// Stack of scopes, each containing declared variable names
    scope_stack: Vec<HashSet<String>>,
    /// Variables that are mutable collection parameters (should be passed directly, already &mut)
    mut_ref_params: HashSet<String>,
    /// Local mutable collection variables (should be passed as &mut at call sites)
    mut_collection_vars: HashSet<String>,
    /// Function signature info for call site analysis
    function_params: HashMap<String, FunctionParamInfo>,
    /// Known collection kinds for variables in the current module/function
    collection_kinds: HashMap<String, CollectionKind>,
    /// Variables known to be String (helps choose between cast vs parse for int()/float() builtins)
    string_vars: HashSet<String>,
}

impl RustEmitter {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            indent_level: 0,
            indent_str: "    ", // 4 spaces for Rust
            scope_stack: Vec::new(),
            mut_ref_params: HashSet::new(),
            mut_collection_vars: HashSet::new(),
            function_params: HashMap::new(),
            collection_kinds: HashMap::new(),
            string_vars: HashSet::new(),
        }
    }
    
    /// Register a function's parameter info (which params are mut)
    pub fn register_function_params(&mut self, name: &str, info: FunctionParamInfo) {
        self.function_params.insert(name.to_string(), info);
    }
    
    /// Check if a function parameter at a given index expects &mut
    pub fn function_param_is_mut(&self, func_name: &str, param_idx: usize) -> bool {
        self.function_params
            .get(func_name)
            .map(|info| info.mut_params.contains(&param_idx))
            .unwrap_or(false)
    }
    
    /// Register a parameter as a mutable reference (already &mut, pass directly)
    pub fn register_mut_ref_param(&mut self, name: &str) {
        self.mut_ref_params.insert(name.to_string());
    }
    
    /// Check if a variable is a mutable reference parameter
    pub fn is_mut_ref_param(&self, name: &str) -> bool {
        self.mut_ref_params.contains(name)
    }
    
    /// Register a local variable as a mutable collection (should be passed as &mut)
    pub fn register_mut_collection_var(&mut self, name: &str) {
        self.mut_collection_vars.insert(name.to_string());
    }
    
    /// Check if a variable is a mutable collection variable
    pub fn is_mut_collection_var(&self, name: &str) -> bool {
        self.mut_collection_vars.contains(name)
    }
    
    /// Clear mutable reference params and collection vars (call when exiting a function)
    pub fn clear_mut_ref_params(&mut self) {
        self.mut_ref_params.clear();
        self.mut_collection_vars.clear();
    }

    /// Register the collection kind of a variable (e.g. from `List[...]`, `Dict[...]`, `Set[...]`)
    pub fn register_collection_kind(&mut self, name: &str, kind: CollectionKind) {
        self.collection_kinds.insert(name.to_string(), kind);
    }

    /// Get the registered collection kind for a variable, if known
    pub fn collection_kind(&self, name: &str) -> Option<CollectionKind> {
        self.collection_kinds.get(name).copied()
    }

    /// Mark a variable as a String
    pub fn register_string_var(&mut self, name: &str) {
        self.string_vars.insert(name.to_string());
    }

    /// Check whether a variable is known to be a String
    pub fn is_string_var(&self, name: &str) -> bool {
        self.string_vars.contains(name)
    }
    
    /// Enter a new variable scope (e.g., function body)
    pub fn push_scope(&mut self) {
        self.scope_stack.push(HashSet::new());
    }
    
    /// Exit the current scope
    pub fn pop_scope(&mut self) {
        self.scope_stack.pop();
    }
    
    /// Declare a variable in the current scope
    pub fn declare_var(&mut self, name: &str) {
        if let Some(scope) = self.scope_stack.last_mut() {
            scope.insert(name.to_string());
        }
    }
    
    /// Check if a variable is declared in any enclosing scope
    pub fn is_var_declared(&self, name: &str) -> bool {
        self.scope_stack.iter().any(|scope| scope.contains(name))
    }

    /// Get the generated code
    pub fn finish(self) -> String {
        self.buffer
    }

    /// Get current buffer as string slice
    pub fn as_str(&self) -> &str {
        &self.buffer
    }

    /// Write a line with current indentation
    pub fn line(&mut self, s: &str) {
        self.write_indent();
        self.buffer.push_str(s);
        self.buffer.push('\n');
    }

    /// Write text without newline
    pub fn write(&mut self, s: &str) {
        self.buffer.push_str(s);
    }

    /// Write formatted text
    pub fn writef(&mut self, args: std::fmt::Arguments<'_>) {
        let _ = self.buffer.write_fmt(args);
    }

    /// Write a blank line
    pub fn blank_line(&mut self) {
        self.buffer.push('\n');
    }

    /// Write indentation only
    pub fn write_indent(&mut self) {
        for _ in 0..self.indent_level {
            self.buffer.push_str(self.indent_str);
        }
    }

    /// Increase indent level
    pub fn indent(&mut self) {
        self.indent_level += 1;
    }

    /// Decrease indent level
    pub fn dedent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }

    /// Write a block with braces
    pub fn block<F>(&mut self, header: &str, f: F)
    where
        F: FnOnce(&mut Self),
    {
        self.line(&format!("{} {{", header));
        self.indent();
        f(self);
        self.dedent();
        self.line("}");
    }

    /// Write an impl block
    pub fn impl_block<F>(&mut self, trait_name: Option<&str>, type_name: &str, f: F)
    where
        F: FnOnce(&mut Self),
    {
        let header = match trait_name {
            Some(t) => format!("impl {} for {}", t, type_name),
            None => format!("impl {}", type_name),
        };
        self.block(&header, f);
    }

    /// Write a function (without generic type parameters)
    pub fn function<F>(
        &mut self,
        visibility: &str,
        is_async: bool,
        name: &str,
        params: &str,
        return_type: &str,
        f: F,
    ) where
        F: FnOnce(&mut Self),
    {
        self.function_generic(visibility, is_async, name, &[], params, return_type, f);
    }

    /// Write a function with generic type parameters
    pub fn function_generic<F>(
        &mut self,
        visibility: &str,
        is_async: bool,
        name: &str,
        type_params: &[String],
        params: &str,
        return_type: &str,
        f: F,
    ) where
        F: FnOnce(&mut Self),
    {
        let async_str = if is_async { "async " } else { "" };
        let vis_str = if visibility.is_empty() {
            String::new()
        } else {
            format!("{} ", visibility)
        };
        let ret_str = if return_type.is_empty() {
            String::new()
        } else {
            format!(" -> {}", return_type)
        };
        let type_params_str = if type_params.is_empty() {
            String::new()
        } else {
            format!("<{}>", type_params.join(", "))
        };
        let header = format!("{}{}fn {}{}{}", vis_str, async_str, name, type_params_str, format!("({}){}", params, ret_str));
        self.block(&header, f);
    }

    /// Write a struct definition
    pub fn struct_def(&mut self, derives: &[&str], visibility: &str, name: &str, fields: &[(String, String)]) {
        // Derive attributes
        if !derives.is_empty() {
            self.line(&format!("#[derive({})]", derives.join(", ")));
        }

        let vis_str = if visibility.is_empty() {
            String::new()
        } else {
            format!("{} ", visibility)
        };

        if fields.is_empty() {
            self.line(&format!("{}struct {};", vis_str, name));
        } else {
            self.line(&format!("{}struct {} {{", vis_str, name));
            self.indent();
            for (field_name, field_type) in fields {
                self.line(&format!("pub {}: {},", field_name, field_type));
            }
            self.dedent();
            self.line("}");
        }
    }

    /// Write an enum definition
    pub fn enum_def(&mut self, derives: &[&str], visibility: &str, name: &str, variants: &[(String, Vec<String>)]) {
        if !derives.is_empty() {
            self.line(&format!("#[derive({})]", derives.join(", ")));
        }

        let vis_str = if visibility.is_empty() {
            String::new()
        } else {
            format!("{} ", visibility)
        };

        self.line(&format!("{}enum {} {{", vis_str, name));
        self.indent();
        for (variant_name, fields) in variants {
            if fields.is_empty() {
                self.line(&format!("{},", variant_name));
            } else {
                self.line(&format!("{}({}),", variant_name, fields.join(", ")));
            }
        }
        self.dedent();
        self.line("}");
    }

    /// Write a trait definition
    pub fn trait_def<F>(&mut self, visibility: &str, name: &str, f: F)
    where
        F: FnOnce(&mut Self),
    {
        let vis_str = if visibility.is_empty() {
            String::new()
        } else {
            format!("{} ", visibility)
        };
        self.block(&format!("{}trait {}", vis_str, name), f);
    }

    /// Write a newtype definition (tuple struct)
    pub fn newtype_def(&mut self, derives: &[&str], visibility: &str, name: &str, inner_type: &str) {
        if !derives.is_empty() {
            self.line(&format!("#[derive({})]", derives.join(", ")));
        }

        let vis_str = if visibility.is_empty() {
            String::new()
        } else {
            format!("{} ", visibility)
        };

        self.line(&format!("{}struct {}(pub {});", vis_str, name, inner_type));
    }

    /// Write a use statement
    pub fn use_stmt(&mut self, path: &str) {
        self.line(&format!("use {};", path));
    }

    /// Write a comment
    pub fn comment(&mut self, text: &str) {
        self.line(&format!("// {}", text));
    }

    /// Write a doc comment
    pub fn doc_comment(&mut self, text: &str) {
        self.line(&format!("/// {}", text));
    }
}

/// Map Incan type to Rust type
pub fn incan_type_to_rust(ty: &str) -> String {
    match ty {
        "int" => "i64".to_string(),
        "float" => "f64".to_string(),
        "bool" => "bool".to_string(),
        "str" => "String".to_string(),
        "bytes" => "Vec<u8>".to_string(),
        "Unit" | "None" => "()".to_string(),
        // Sync primitives are Arc-wrapped for sharing between tasks
        "Semaphore" => "std::sync::Arc<tokio::sync::Semaphore>".to_string(),
        "Barrier" => "std::sync::Arc<tokio::sync::Barrier>".to_string(),
        _ => {
            // Handle generic types like List[T], Dict[K, V], etc.
            if ty.contains('[') {
                convert_generic_type(ty)
            } else {
                ty.to_string()
            }
        }
    }
}

/// Convert generic type syntax from Incan to Rust
fn convert_generic_type(ty: &str) -> String {
    // List[T] -> Vec<T>
    // Dict[K, V] -> std::collections::HashMap<K, V>
    // Set[T] -> std::collections::HashSet<T>
    // Option[T] -> Option<T>
    // Result[T, E] -> Result<T, E>
    // Tuple[T1, T2] -> (T1, T2)

    if let Some(start) = ty.find('[') {
        let name = &ty[..start];
        let args_str = &ty[start + 1..ty.len() - 1];
        let args: Vec<&str> = split_type_args(args_str);
        let converted_args: Vec<String> = args.iter().map(|a| incan_type_to_rust(a.trim())).collect();

        match name {
            "List" => format!("Vec<{}>", converted_args.join(", ")),
            "Dict" => format!("std::collections::HashMap<{}>", converted_args.join(", ")),
            "Set" => format!("std::collections::HashSet<{}>", converted_args.join(", ")),
            "Option" => format!("Option<{}>", converted_args.join(", ")),
            "Result" => format!("Result<{}>", converted_args.join(", ")),
            "Tuple" => format!("({})", converted_args.join(", ")),
            _ => format!("{}<{}>", name, converted_args.join(", ")),
        }
    } else {
        ty.to_string()
    }
}

/// Split type arguments, respecting nested brackets
fn split_type_args(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut start = 0;

    for (i, c) in s.char_indices() {
        match c {
            '[' | '<' | '(' => depth += 1,
            ']' | '>' | ')' => depth -= 1,
            ',' if depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }

    if start < s.len() {
        result.push(&s[start..]);
    }

    result
}

/// Convert Incan identifier to valid Rust identifier
pub fn to_rust_ident(name: &str) -> String {
    // Handle reserved words
    match name {
        "type" => "r#type".to_string(),
        "match" => "r#match".to_string(),
        "mod" => "r#mod".to_string(),
        "ref" => "r#ref".to_string(),
        "self" => "self".to_string(),
        _ => name.to_string(),
    }
}

/// Convert Incan binary operator to Rust
pub fn binary_op_to_rust(op: &str) -> &'static str {
    match op {
        "and" => "&&",
        "or" => "||",
        "==" => "==",
        "!=" => "!=",
        "+" => "+",
        "-" => "-",
        "*" => "*",
        "/" => "/",
        "%" => "%",
        "<" => "<",
        ">" => ">",
        "<=" => "<=",
        ">=" => ">=",
        _ => "/* unknown op */",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_conversion() {
        assert_eq!(incan_type_to_rust("int"), "i64");
        assert_eq!(incan_type_to_rust("str"), "String");
        assert_eq!(incan_type_to_rust("List[int]"), "Vec<i64>");
        assert_eq!(incan_type_to_rust("Dict[str, int]"), "std::collections::HashMap<String, i64>");
        assert_eq!(incan_type_to_rust("Result[int, str]"), "Result<i64, String>");
    }

    #[test]
    fn test_emitter_struct() {
        let mut e = RustEmitter::new();
        e.struct_def(
            &["Debug", "Clone"],
            "pub",
            "User",
            &[
                ("name".to_string(), "String".to_string()),
                ("age".to_string(), "i64".to_string()),
            ],
        );
        let code = e.finish();
        assert!(code.contains("#[derive(Debug, Clone)]"));
        assert!(code.contains("pub struct User {"));
        assert!(code.contains("pub name: String,"));
    }
}
