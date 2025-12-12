//! Parser for the Incan programming language
//!
//! Converts a token stream into an AST following RFC 000: Incan Core Language RFC (Phase 1).

use crate::frontend::ast::*;
use crate::frontend::diagnostics::CompileError;
use crate::frontend::lexer::{Token, TokenKind, FStringPart as LexFStringPart};

/// Result of parsing index brackets - either a single index or a slice
enum IndexOrSlice {
    Index(Spanned<Expr>),
    Slice(SliceExpr),
}

/// Parser state
pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    errors: Vec<CompileError>,
}

impl<'a> Parser<'a> {
    pub fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            errors: Vec::new(),
        }
    }

    /// Parse the entire token stream into a program
    pub fn parse(mut self) -> Result<Program, Vec<CompileError>> {
        let mut declarations = Vec::new();

        // Skip leading newlines
        self.skip_newlines();
        // Stray top-level DEDENT can appear after error recovery (e.g. unexpected indentation).
        // Ignore it at the module level to avoid cascaded errors.
        self.skip_dedents();

        while !self.is_at_end() {
            match self.declaration() {
                Ok(decl) => declarations.push(decl),
                Err(e) => {
                    self.errors.push(e);
                    self.synchronize();
                }
            }
            self.skip_newlines();
            // Same rationale as above: at the module level we should not see DEDENT tokens,
            // but the lexer may emit them and recovery may leave us positioned on them.
            self.skip_dedents();
        }

        if self.errors.is_empty() {
            Ok(Program { declarations })
        } else {
            Err(self.errors)
        }
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_next(&self) -> &Token {
        if self.pos + 1 < self.tokens.len() {
            &self.tokens[self.pos + 1]
        } else {
            &self.tokens[self.tokens.len() - 1]
        }
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.pos += 1;
        }
        &self.tokens[self.pos - 1]
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(kind)
    }

    fn check_keyword(&self, keyword: &TokenKind) -> bool {
        self.peek().kind == *keyword
    }

    fn match_token(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind, msg: &str) -> Result<&Token, CompileError> {
        if self.check(kind) {
            Ok(self.advance())
        } else {
            Err(CompileError::syntax(
                format!("{}, found {:?}", msg, self.peek().kind),
                self.peek().span,
            ))
        }
    }

    fn skip_newlines(&mut self) {
        while self.match_token(&TokenKind::Newline) {}
    }

    /// Skip stray DEDENT tokens at the current position.
    ///
    /// These should not normally appear at module level, but can show up after error recovery.
    fn skip_dedents(&mut self) {
        while self.match_token(&TokenKind::Dedent) {}
    }

    fn synchronize(&mut self) {
        self.advance();
        while !self.is_at_end() {
            if matches!(
                self.peek().kind,
                TokenKind::Def
                    | TokenKind::Class
                    | TokenKind::Model
                    | TokenKind::Trait
                    | TokenKind::Enum
                    | TokenKind::Type
                    | TokenKind::Import
            ) {
                return;
            }
            if matches!(self.peek().kind, TokenKind::Newline) {
                self.advance();
                return;
            }
            self.advance();
        }
    }

    fn current_span(&self) -> Span {
        self.peek().span
    }

    /// Check if the current token can start an expression
    fn is_at_expr_start(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::Ident(_)
                | TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::FString(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::None
                | TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
                | TokenKind::Minus
                | TokenKind::Not
                | TokenKind::SelfKw
                | TokenKind::Await
                | TokenKind::Match
                | TokenKind::If
        )
    }

    // ========================================================================
    // Declarations
    // ========================================================================

    fn declaration(&mut self) -> Result<Spanned<Declaration>, CompileError> {
        let start = self.current_span().start;

        // Handle module-level docstrings (string literals at top level)
        if let TokenKind::String(s) = &self.peek().kind {
            let doc = s.clone();
            self.advance();
            // Skip optional newline after docstring
            self.match_token(&TokenKind::Newline);
            let end = self.tokens[self.pos.saturating_sub(1)].span.end;
            return Ok(Spanned::new(Declaration::Docstring(doc), Span::new(start, end)));
        }

        // Collect decorators
        let decorators = self.decorators()?;

        let decl = if self.check_keyword(&TokenKind::Import) || self.check_keyword(&TokenKind::From) {
            Declaration::Import(self.import_decl()?)
        } else if self.check_keyword(&TokenKind::Model) {
            Declaration::Model(self.model_decl(decorators)?)
        } else if self.check_keyword(&TokenKind::Class) {
            Declaration::Class(self.class_decl(decorators)?)
        } else if self.check_keyword(&TokenKind::Trait) {
            Declaration::Trait(self.trait_decl(decorators)?)
        } else if self.check_keyword(&TokenKind::Type) || self.check_keyword(&TokenKind::Newtype) {
            Declaration::Newtype(self.newtype_decl()?)
        } else if self.check_keyword(&TokenKind::Enum) {
            Declaration::Enum(self.enum_decl()?)
        } else if self.check_keyword(&TokenKind::Def) || self.check_keyword(&TokenKind::Async) {
            Declaration::Function(self.function_decl(decorators)?)
        } else {
            return Err(CompileError::syntax(
                format!("Expected declaration, found {:?}", self.peek().kind),
                self.current_span(),
            ));
        };

        let end = self.tokens[self.pos.saturating_sub(1)].span.end;
        Ok(Spanned::new(decl, Span::new(start, end)))
    }

    fn decorators(&mut self) -> Result<Vec<Spanned<Decorator>>, CompileError> {
        let mut decorators = Vec::new();
        while self.match_token(&TokenKind::At) {
            let start = self.tokens[self.pos - 1].span.start;
            let name = self.identifier()?;
            let args = if self.match_token(&TokenKind::LParen) {
                let args = self.decorator_args()?;
                self.expect(&TokenKind::RParen, "Expected ')' after decorator arguments")?;
                args
            } else {
                Vec::new()
            };
            let end = self.tokens[self.pos - 1].span.end;
            decorators.push(Spanned::new(Decorator { name, args }, Span::new(start, end)));
            self.skip_newlines();
        }
        Ok(decorators)
    }

    fn decorator_args(&mut self) -> Result<Vec<DecoratorArg>, CompileError> {
        let mut args = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                // Check for named argument (name: Type or name=value)
                if let TokenKind::Ident(name) = &self.peek().kind {
                    let name = name.clone();
                    if self.peek_next().kind == TokenKind::Colon {
                        self.advance(); // consume name
                        self.advance(); // consume :
                        let ty = self.type_expr()?;
                        args.push(DecoratorArg::Named(name, DecoratorArgValue::Type(ty)));
                    } else if self.peek_next().kind == TokenKind::Eq {
                        self.advance(); // consume name
                        self.advance(); // consume =
                        let expr = self.expression()?;
                        args.push(DecoratorArg::Named(name, DecoratorArgValue::Expr(expr)));
                    } else {
                        let expr = self.expression()?;
                        args.push(DecoratorArg::Positional(expr));
                    }
                } else {
                    let expr = self.expression()?;
                    args.push(DecoratorArg::Positional(expr));
                }

                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
        }
        Ok(args)
    }

    fn import_decl(&mut self) -> Result<ImportDecl, CompileError> {
        // Check for "from ... import ..." syntax
        if self.match_token(&TokenKind::From) {
            // Check for "from rust::crate import ..." syntax
            if self.match_token(&TokenKind::RustKw) {
                self.expect(&TokenKind::ColonColon, "Expected '::' after 'rust'")?;
                let (crate_name, path) = self.rust_crate_path()?;
                self.expect(&TokenKind::Import, "Expected 'import' after rust crate path")?;
                
                // Parse import items
                let mut items = Vec::new();
                loop {
                    let name = self.identifier()?;
                    let alias = if self.match_token(&TokenKind::As) {
                        Some(self.identifier()?)
                    } else {
                        None
                    };
                    items.push(ImportItem { name, alias });
                    
                    if !self.match_token(&TokenKind::Comma) {
                        break;
                    }
                }
                
                return Ok(ImportDecl {
                    kind: ImportKind::RustFrom { crate_name, path, items },
                    alias: None,
                });
            }
            
            // Regular from import
            let module = self.import_path()?;
            self.expect(&TokenKind::Import, "Expected 'import' after module path")?;
            
            // Parse import items: item1, item2 as alias, item3, ...
            let mut items = Vec::new();
            loop {
                let name = self.identifier()?;
                let alias = if self.match_token(&TokenKind::As) {
                    Some(self.identifier()?)
                } else {
                    None
                };
                items.push(ImportItem { name, alias });
                
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            
            return Ok(ImportDecl {
                kind: ImportKind::From { module, items },
                alias: None,
            });
        }
        
        // Regular import syntax (Rust-style)
        self.expect(&TokenKind::Import, "Expected 'import'")?;

        let kind = if self.match_token(&TokenKind::Py) {
            // Python import: import py "package" as alias
            let pkg = self.string_literal()?;
            ImportKind::Python(pkg)
        } else if self.match_token(&TokenKind::RustKw) {
            // Rust crate import: import rust::serde_json or import rust::serde_json::Value
            self.expect(&TokenKind::ColonColon, "Expected '::' after 'rust'")?;
            let (crate_name, path) = self.rust_crate_path()?;
            ImportKind::RustCrate { crate_name, path }
        } else {
            // Module import: import foo::bar::baz or import super::foo or import crate::foo
            let path = self.import_path()?;
            ImportKind::Module(path)
        };

        let alias = if self.match_token(&TokenKind::As) {
            Some(self.identifier()?)
        } else {
            None
        };

        Ok(ImportDecl { kind, alias })
    }
    
    /// Parse a Rust crate path after `rust::`
    /// Returns (crate_name, optional_path_within_crate)
    /// Examples:
    /// - `serde_json` -> ("serde_json", [])
    /// - `serde_json::Value` -> ("serde_json", ["Value"])
    /// - `std::collections::HashMap` -> ("std", ["collections", "HashMap"])
    fn rust_crate_path(&mut self) -> Result<(String, Vec<Ident>), CompileError> {
        let crate_name = self.identifier()?;
        let mut path = Vec::new();
        
        while self.match_token(&TokenKind::ColonColon) {
            let segment = self.identifier()?;
            path.push(segment);
        }
        
        Ok((crate_name, path))
    }

    /// Parse an import path, supporting:
    /// - Simple: `models`, `utils::helpers`
    /// - Relative with dots: `..common`, `...shared.utils`
    /// - Relative with super: `super::common`, `super::super::utils`
    /// - Absolute with crate: `crate::config`
    /// - Dotted paths: `db.models`, `api.handlers.auth`
    fn import_path(&mut self) -> Result<ImportPath, CompileError> {
        let mut parent_levels = 0;
        let mut is_absolute = false;
        let mut segments = Vec::new();

        // Check for leading `..` (Python-style parent navigation)
        while self.match_token(&TokenKind::DotDot) {
            parent_levels += 1;
        }

        // Check for `crate` (absolute path)
        if parent_levels == 0 && self.match_token(&TokenKind::Crate) {
            is_absolute = true;
            // Expect :: or . after crate
            if !self.match_token(&TokenKind::ColonColon) && !self.match_token(&TokenKind::Dot) {
                return Err(CompileError::syntax(
                    "Expected '::' or '.' after 'crate'".to_string(),
                    self.current_span(),
                ));
            }
        }

        // Check for `super` (Rust-style parent navigation)
        while self.match_token(&TokenKind::Super) {
            parent_levels += 1;
            // Expect :: or . after super
            if !self.match_token(&TokenKind::ColonColon) && !self.match_token(&TokenKind::Dot) {
                // Could be end of path if no more segments
                if !self.check(&TokenKind::Import) && !self.check(&TokenKind::As) && !self.check(&TokenKind::Newline) {
                    return Err(CompileError::syntax(
                        "Expected '::' or '.' after 'super'".to_string(),
                        self.current_span(),
                    ));
                }
            }
        }

        // Parse the actual path segments
        // First segment
        if let Ok(first) = self.identifier() {
            segments.push(first);
            
            // Continue with :: or . separators
            loop {
                if self.match_token(&TokenKind::ColonColon) {
                    segments.push(self.identifier()?);
                } else if self.match_token(&TokenKind::Dot) {
                    segments.push(self.identifier()?);
                } else {
                    break;
                }
            }
        }

        Ok(ImportPath {
            parent_levels,
            is_absolute,
            segments,
        })
    }

    fn model_decl(&mut self, decorators: Vec<Spanned<Decorator>>) -> Result<ModelDecl, CompileError> {
        self.expect(&TokenKind::Model, "Expected 'model'")?;
        let name = self.identifier()?;
        let type_params = self.type_params()?;
        self.expect(&TokenKind::Colon, "Expected ':' after model name")?;
        self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
        self.expect(&TokenKind::Indent, "Expected indented block")?;

        let (fields, methods) = self.fields_and_methods()?;

        self.expect(&TokenKind::Dedent, "Expected dedent after model body")?;

        Ok(ModelDecl {
            decorators,
            name,
            type_params,
            fields,
            methods,
        })
    }

    fn class_decl(&mut self, decorators: Vec<Spanned<Decorator>>) -> Result<ClassDecl, CompileError> {
        self.expect(&TokenKind::Class, "Expected 'class'")?;
        let name = self.identifier()?;
        let type_params = self.type_params()?;

        let extends = if self.match_token(&TokenKind::Extends) {
            Some(self.identifier()?)
        } else {
            None
        };

        let traits = if self.match_token(&TokenKind::With) {
            self.identifier_list()?
        } else {
            Vec::new()
        };

        self.expect(&TokenKind::Colon, "Expected ':' after class header")?;
        self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
        self.expect(&TokenKind::Indent, "Expected indented block")?;

        let (fields, methods) = self.fields_and_methods()?;

        self.expect(&TokenKind::Dedent, "Expected dedent after class body")?;

        Ok(ClassDecl {
            decorators,
            name,
            type_params,
            extends,
            traits,
            fields,
            methods,
        })
    }

    fn trait_decl(&mut self, decorators: Vec<Spanned<Decorator>>) -> Result<TraitDecl, CompileError> {
        self.expect(&TokenKind::Trait, "Expected 'trait'")?;
        let name = self.identifier()?;
        let type_params = self.type_params()?;
        self.expect(&TokenKind::Colon, "Expected ':' after trait name")?;
        self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
        self.expect(&TokenKind::Indent, "Expected indented block")?;

        let mut methods = Vec::new();
        self.skip_newlines();
        while !self.check(&TokenKind::Dedent) && !self.is_at_end() {
            let method_decorators = self.decorators()?;
            methods.push(self.method_decl(method_decorators)?);
            self.skip_newlines();
        }

        self.expect(&TokenKind::Dedent, "Expected dedent after trait body")?;

        Ok(TraitDecl {
            decorators,
            name,
            type_params,
            methods,
        })
    }

    fn newtype_decl(&mut self) -> Result<NewtypeDecl, CompileError> {
        // Support both: "type X = newtype T" and "newtype X = T"
        if self.match_token(&TokenKind::Newtype) {
            // newtype X = T syntax
        } else {
            self.expect(&TokenKind::Type, "Expected 'type' or 'newtype'")?;
        }
        let name = self.identifier()?;
        self.expect(&TokenKind::Eq, "Expected '=' after type name")?;
        // Skip optional 'newtype' keyword if present (for "type X = newtype T" form)
        self.match_token(&TokenKind::Newtype);
        let underlying = self.type_expr()?;

        let methods = if self.match_token(&TokenKind::Colon) {
            self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
            self.expect(&TokenKind::Indent, "Expected indented block")?;

            let mut methods = Vec::new();
            self.skip_newlines();
            while !self.check(&TokenKind::Dedent) && !self.is_at_end() {
                let method_decorators = self.decorators()?;
                methods.push(self.method_decl(method_decorators)?);
                self.skip_newlines();
            }

            self.expect(&TokenKind::Dedent, "Expected dedent after newtype body")?;
            methods
        } else {
            Vec::new()
        };

        Ok(NewtypeDecl {
            name,
            underlying,
            methods,
        })
    }

    fn enum_decl(&mut self) -> Result<EnumDecl, CompileError> {
        self.expect(&TokenKind::Enum, "Expected 'enum'")?;
        let name = self.identifier()?;
        let type_params = self.type_params()?;
        self.expect(&TokenKind::Colon, "Expected ':' after enum name")?;
        self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
        self.expect(&TokenKind::Indent, "Expected indented block")?;

        let mut variants = Vec::new();
        self.skip_newlines();
        while !self.check(&TokenKind::Dedent) && !self.is_at_end() {
            variants.push(self.variant_decl()?);
            self.skip_newlines();
        }

        self.expect(&TokenKind::Dedent, "Expected dedent after enum body")?;

        Ok(EnumDecl {
            name,
            type_params,
            variants,
        })
    }

    fn variant_decl(&mut self) -> Result<Spanned<VariantDecl>, CompileError> {
        let start = self.current_span().start;
        let name = self.identifier()?;
        let fields = if self.match_token(&TokenKind::LParen) {
            let fields = self.type_list()?;
            self.expect(&TokenKind::RParen, "Expected ')' after variant fields")?;
            fields
        } else {
            Vec::new()
        };
        let end = self.tokens[self.pos - 1].span.end;
        Ok(Spanned::new(VariantDecl { name, fields }, Span::new(start, end)))
    }

    fn function_decl(&mut self, decorators: Vec<Spanned<Decorator>>) -> Result<FunctionDecl, CompileError> {
        let is_async = self.match_token(&TokenKind::Async);
        self.expect(&TokenKind::Def, "Expected 'def'")?;
        let name = self.identifier()?;
        
        // Parse optional generic type parameters: def func[T, E](...)
        let type_params = self.type_params()?;
        
        self.expect(&TokenKind::LParen, "Expected '(' after function name")?;
        let params = self.params()?;
        self.expect(&TokenKind::RParen, "Expected ')' after parameters")?;
        self.expect(&TokenKind::Arrow, "Expected '->' before return type")?;
        let return_type = self.type_expr()?;
        self.expect(&TokenKind::Colon, "Expected ':' after return type")?;
        self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
        self.expect(&TokenKind::Indent, "Expected indented block")?;

        let body = self.block()?;

        self.expect(&TokenKind::Dedent, "Expected dedent after function body")?;

        Ok(FunctionDecl {
            decorators,
            is_async,
            name,
            type_params,
            params,
            return_type,
            body,
        })
    }

    fn method_decl(&mut self, decorators: Vec<Spanned<Decorator>>) -> Result<Spanned<MethodDecl>, CompileError> {
        let start = self.current_span().start;
        let is_async = self.match_token(&TokenKind::Async);
        self.expect(&TokenKind::Def, "Expected 'def'")?;
        let name = self.identifier()?;
        self.expect(&TokenKind::LParen, "Expected '(' after method name")?;

        // Parse receiver and params
        let (receiver, params) = self.receiver_and_params()?;

        self.expect(&TokenKind::RParen, "Expected ')' after parameters")?;
        self.expect(&TokenKind::Arrow, "Expected '->' before return type")?;
        let return_type = self.type_expr()?;

        // Check for abstract method (...) or block
        let body = if self.match_token(&TokenKind::Colon) {
            if self.match_token(&TokenKind::Ellipsis) {
                None
            } else {
                self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
                self.expect(&TokenKind::Indent, "Expected indented block")?;
                let b = self.block()?;
                self.expect(&TokenKind::Dedent, "Expected dedent after method body")?;
                Some(b)
            }
        } else {
            return Err(CompileError::syntax(
                "Expected ':' after return type".to_string(),
                self.current_span(),
            ));
        };

        let end = self.tokens[self.pos - 1].span.end;

        Ok(Spanned::new(
            MethodDecl {
                decorators,
                is_async,
                name,
                receiver,
                params,
                return_type,
                body,
            },
            Span::new(start, end),
        ))
    }

    fn receiver_and_params(&mut self) -> Result<(Option<Receiver>, Vec<Spanned<Param>>), CompileError> {
        // Check for receiver
        let receiver = if self.check_keyword(&TokenKind::Mut) {
            self.advance();
            self.expect(&TokenKind::SelfKw, "Expected 'self' after 'mut'")?;
            if self.check(&TokenKind::Comma) {
                self.advance();
            }
            Some(Receiver::Mutable)
        } else if self.check_keyword(&TokenKind::SelfKw) {
            self.advance();
            if self.check(&TokenKind::Comma) {
                self.advance();
            }
            Some(Receiver::Immutable)
        } else {
            None
        };

        let params = if !self.check(&TokenKind::RParen) {
            self.params()?
        } else {
            Vec::new()
        };

        Ok((receiver, params))
    }

    fn params(&mut self) -> Result<Vec<Spanned<Param>>, CompileError> {
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                params.push(self.param()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
        }
        Ok(params)
    }

    fn param(&mut self) -> Result<Spanned<Param>, CompileError> {
        let start = self.current_span().start;
        let name = self.identifier()?;
        self.expect(&TokenKind::Colon, "Expected ':' after parameter name")?;
        let ty = self.type_expr()?;
        let default = if self.match_token(&TokenKind::Eq) {
            Some(self.expression()?)
        } else {
            None
        };
        let end = self.tokens[self.pos - 1].span.end;
        Ok(Spanned::new(Param { name, ty, default }, Span::new(start, end)))
    }

    fn fields_and_methods(&mut self) -> Result<(Vec<Spanned<FieldDecl>>, Vec<Spanned<MethodDecl>>), CompileError> {
        let mut fields = Vec::new();
        let mut methods = Vec::new();

        self.skip_newlines();
        while !self.check(&TokenKind::Dedent) && !self.is_at_end() {
            let decorators = self.decorators()?;

            // Check if it's a method (starts with def or async def)
            if self.check_keyword(&TokenKind::Def) || self.check_keyword(&TokenKind::Async) {
                methods.push(self.method_decl(decorators)?);
            } else {
                // It's a field
                if !decorators.is_empty() {
                    return Err(CompileError::syntax(
                        "Decorators on fields are not supported".to_string(),
                        decorators[0].span,
                    ));
                }
                fields.push(self.field_decl()?);
            }
            self.skip_newlines();
        }

        Ok((fields, methods))
    }

    fn field_decl(&mut self) -> Result<Spanned<FieldDecl>, CompileError> {
        let start = self.current_span().start;
        let name = self.identifier()?;
        self.expect(&TokenKind::Colon, "Expected ':' after field name")?;
        let ty = self.type_expr()?;
        let default = if self.match_token(&TokenKind::Eq) {
            Some(self.expression()?)
        } else {
            None
        };
        let end = self.tokens[self.pos - 1].span.end;
        Ok(Spanned::new(FieldDecl { name, ty, default }, Span::new(start, end)))
    }

    // ========================================================================
    // Types
    // ========================================================================

    fn type_params(&mut self) -> Result<Vec<Ident>, CompileError> {
        if self.match_token(&TokenKind::LBracket) {
            let params = self.identifier_list()?;
            self.expect(&TokenKind::RBracket, "Expected ']' after type parameters")?;
            Ok(params)
        } else {
            Ok(Vec::new())
        }
    }

    fn type_expr(&mut self) -> Result<Spanned<Type>, CompileError> {
        let start = self.current_span().start;

        // Unit type
        if self.match_token(&TokenKind::LParen) {
            if self.match_token(&TokenKind::RParen) {
                // Could be unit type () or zero-arg function type () -> T
                if self.match_token(&TokenKind::Arrow) {
                    let ret = self.type_expr()?;
                    let end = ret.span.end;
                    return Ok(Spanned::new(
                        Type::Function(vec![], Box::new(ret)),
                        Span::new(start, end),
                    ));
                }
                let end = self.tokens[self.pos - 1].span.end;
                return Ok(Spanned::new(Type::Unit, Span::new(start, end)));
            }
            // Could be tuple type or function type
            let first = self.type_expr()?;
            if self.match_token(&TokenKind::Comma) {
                // Tuple type
                let mut types = vec![first];
                if !self.check(&TokenKind::RParen) {
                    loop {
                        types.push(self.type_expr()?);
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RParen, "Expected ')' after tuple type")?;

                // Check for function type
                if self.match_token(&TokenKind::Arrow) {
                    let ret = self.type_expr()?;
                    let end = ret.span.end;
                    return Ok(Spanned::new(
                        Type::Function(types, Box::new(ret)),
                        Span::new(start, end),
                    ));
                }

                let end = self.tokens[self.pos - 1].span.end;
                return Ok(Spanned::new(Type::Tuple(types), Span::new(start, end)));
            }
            self.expect(&TokenKind::RParen, "Expected ')'")?;

            // Check for function type
            if self.match_token(&TokenKind::Arrow) {
                let ret = self.type_expr()?;
                let end = ret.span.end;
                return Ok(Spanned::new(
                    Type::Function(vec![first], Box::new(ret)),
                    Span::new(start, end),
                ));
            }

            // Just a parenthesized type
            return Ok(first);
        }

        // Handle None as a type (alias for unit/void)
        if self.match_token(&TokenKind::None) {
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Type::Simple("None".to_string()), Span::new(start, end)));
        }

        // Named type
        let name = self.identifier()?;

        // Check for Self type (refers to the implementing type in traits)
        if name == "Self" {
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Type::SelfType, Span::new(start, end)));
        }

        // Check for generic arguments
        if self.match_token(&TokenKind::LBracket) {
            let args = self.type_list()?;
            self.expect(&TokenKind::RBracket, "Expected ']' after type arguments")?;
            let end = self.tokens[self.pos - 1].span.end;
            Ok(Spanned::new(Type::Generic(name, args), Span::new(start, end)))
        } else {
            let end = self.tokens[self.pos - 1].span.end;
            Ok(Spanned::new(Type::Simple(name), Span::new(start, end)))
        }
    }

    fn type_list(&mut self) -> Result<Vec<Spanned<Type>>, CompileError> {
        let mut types = Vec::new();
        if !self.check(&TokenKind::RBracket) && !self.check(&TokenKind::RParen) {
            loop {
                types.push(self.type_expr()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
        }
        Ok(types)
    }

    // ========================================================================
    // Statements
    // ========================================================================

    fn block(&mut self) -> Result<Vec<Spanned<Statement>>, CompileError> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.check(&TokenKind::Dedent) && !self.is_at_end() {
            stmts.push(self.statement()?);
            self.skip_newlines();
        }
        Ok(stmts)
    }

    fn statement(&mut self) -> Result<Spanned<Statement>, CompileError> {
        let start = self.current_span().start;

        let stmt = if self.check_keyword(&TokenKind::Return) {
            self.return_stmt()?
        } else if self.check_keyword(&TokenKind::If) {
            self.if_stmt()?
        } else if self.check_keyword(&TokenKind::While) {
            self.while_stmt()?
        } else if self.check_keyword(&TokenKind::For) {
            self.for_stmt()?
        } else if self.check_keyword(&TokenKind::Break) {
            self.advance();
            Statement::Break
        } else if self.check_keyword(&TokenKind::Continue) {
            self.advance();
            Statement::Continue
        } else if self.check_keyword(&TokenKind::Pass) {
            self.advance();
            Statement::Pass
        } else if self.check(&TokenKind::Ellipsis) {
            // ... is equivalent to pass (Python-style placeholder)
            self.advance();
            Statement::Pass
        } else if self.check_keyword(&TokenKind::Let) || self.check_keyword(&TokenKind::Mut) {
            self.assignment_stmt()?
        } else {
            // Could be assignment or expression
            self.assignment_or_expr_stmt()?
        };

        let end = self.tokens[self.pos.saturating_sub(1)].span.end;
        Ok(Spanned::new(stmt, Span::new(start, end)))
    }

    /// Parse a single inline statement (for use in inline case arms)
    /// Supports: return, expression statements, pass
    fn inline_statement(&mut self) -> Result<Spanned<Statement>, CompileError> {
        let start = self.current_span().start;
        
        let stmt = if self.check_keyword(&TokenKind::Return) {
            self.advance();
            let expr = if !self.check(&TokenKind::Newline) && !self.check(&TokenKind::Case) && !self.check(&TokenKind::Dedent) {
                Some(self.expression()?)
            } else {
                None
            };
            Statement::Return(expr)
        } else if self.check_keyword(&TokenKind::Pass) {
            self.advance();
            Statement::Pass
        } else if self.check(&TokenKind::Ellipsis) {
            self.advance();
            Statement::Pass
        } else {
            // Expression statement
            let expr = self.expression()?;
            Statement::Expr(expr)
        };
        
        let end = self.tokens[self.pos.saturating_sub(1)].span.end;
        Ok(Spanned::new(stmt, Span::new(start, end)))
    }

    fn return_stmt(&mut self) -> Result<Statement, CompileError> {
        self.expect(&TokenKind::Return, "Expected 'return'")?;
        let expr = if !self.check(&TokenKind::Newline) && !self.check(&TokenKind::Dedent) {
            Some(self.expression()?)
        } else {
            None
        };
        Ok(Statement::Return(expr))
    }

    fn if_stmt(&mut self) -> Result<Statement, CompileError> {
        self.expect(&TokenKind::If, "Expected 'if'")?;
        let condition = self.expression()?;
        self.expect(&TokenKind::Colon, "Expected ':' after if condition")?;
        self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
        self.expect(&TokenKind::Indent, "Expected indented block")?;
        let then_body = self.block()?;
        self.expect(&TokenKind::Dedent, "Expected dedent after if body")?;

        let else_body = if self.match_token(&TokenKind::Else) {
            self.expect(&TokenKind::Colon, "Expected ':' after else")?;
            self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
            self.expect(&TokenKind::Indent, "Expected indented block")?;
            let body = self.block()?;
            self.expect(&TokenKind::Dedent, "Expected dedent after else body")?;
            Some(body)
        } else {
            None
        };

        Ok(Statement::If(IfStmt {
            condition,
            then_body,
            else_body,
        }))
    }

    fn while_stmt(&mut self) -> Result<Statement, CompileError> {
        self.expect(&TokenKind::While, "Expected 'while'")?;
        let condition = self.expression()?;
        self.expect(&TokenKind::Colon, "Expected ':' after while condition")?;
        self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
        self.expect(&TokenKind::Indent, "Expected indented block")?;
        let body = self.block()?;
        self.expect(&TokenKind::Dedent, "Expected dedent after while body")?;

        Ok(Statement::While(WhileStmt { condition, body }))
    }

    fn for_stmt(&mut self) -> Result<Statement, CompileError> {
        self.expect(&TokenKind::For, "Expected 'for'")?;
        let var = self.identifier()?;
        self.expect(&TokenKind::In, "Expected 'in' after for variable")?;
        let iter = self.expression()?;
        self.expect(&TokenKind::Colon, "Expected ':' after for expression")?;
        self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
        self.expect(&TokenKind::Indent, "Expected indented block")?;
        let body = self.block()?;
        self.expect(&TokenKind::Dedent, "Expected dedent after for body")?;

        Ok(Statement::For(ForStmt { var, iter, body }))
    }

    fn assignment_stmt(&mut self) -> Result<Statement, CompileError> {
        let binding = if self.match_token(&TokenKind::Let) {
            BindingKind::Let
        } else if self.match_token(&TokenKind::Mut) {
            BindingKind::Mutable
        } else {
            BindingKind::Inferred
        };

        let name = self.identifier()?;
        
        // Check for tuple unpacking: a, b, c = expr
        if self.match_token(&TokenKind::Comma) {
            let mut names = vec![name];
            loop {
                names.push(self.identifier()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::Eq, "Expected '=' in tuple unpacking")?;
            let value = self.expression()?;
            return Ok(Statement::TupleUnpack(TupleUnpackStmt {
                binding,
                names,
                value,
            }));
        }
        
        let ty = if self.match_token(&TokenKind::Colon) {
            Some(self.type_expr()?)
        } else {
            None
        };
        self.expect(&TokenKind::Eq, "Expected '=' in assignment")?;
        let value = self.expression()?;

        Ok(Statement::Assignment(AssignmentStmt {
            binding,
            name,
            ty,
            value,
        }))
    }

    fn assignment_or_expr_stmt(&mut self) -> Result<Statement, CompileError> {
        // Look for `ident = expr` or `ident, ident = expr` pattern (simple or tuple assignment)
        if let TokenKind::Ident(_) = &self.peek().kind {
            // Check if next is = or : (for assignment) or , (for tuple unpacking)
            if self.peek_next().kind == TokenKind::Eq 
                || self.peek_next().kind == TokenKind::Colon
                || self.peek_next().kind == TokenKind::Comma {
                return self.assignment_stmt();
            }
            // Check for compound assignment: ident += expr, ident -= expr, etc.
            let compound_op = match &self.peek_next().kind {
                TokenKind::PlusEq => Some(CompoundOp::Add),
                TokenKind::MinusEq => Some(CompoundOp::Sub),
                TokenKind::StarEq => Some(CompoundOp::Mul),
                TokenKind::SlashEq => Some(CompoundOp::Div),
                TokenKind::PercentEq => Some(CompoundOp::Mod),
                _ => None,
            };
            if let Some(op) = compound_op {
                let name = self.identifier()?;
                self.advance(); // consume the compound operator
                let value = self.expression()?;
                return Ok(Statement::CompoundAssignment(CompoundAssignmentStmt {
                    name,
                    op,
                    value,
                }));
            }
        }

        // Parse the expression (could be field access like self.field or index like arr[i])
        let expr = self.expression()?;
        
        // Check for assignment: expr.field = value or expr[index] = value
        if self.match_token(&TokenKind::Eq) {
            match expr.node {
                Expr::Field(object, field) => {
                    let value = self.expression()?;
                    return Ok(Statement::FieldAssignment(FieldAssignmentStmt {
                        object: *object,
                        field,
                        value,
                    }));
                }
                Expr::Index(object, index) => {
                    let value = self.expression()?;
                    return Ok(Statement::IndexAssignment(IndexAssignmentStmt {
                        object: *object,
                        index: *index,
                        value,
                    }));
                }
                _ => {
                    return Err(CompileError::syntax(
                        "Invalid assignment target".to_string(),
                        expr.span,
                    ));
                }
            }
        }
        
        // Otherwise it's an expression statement
        Ok(Statement::Expr(expr))
    }

    // ========================================================================
    // Expressions
    // ========================================================================

    fn expression(&mut self) -> Result<Spanned<Expr>, CompileError> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> Result<Spanned<Expr>, CompileError> {
        let mut left = self.and_expr()?;
        while self.match_token(&TokenKind::Or) {
            let right = self.and_expr()?;
            let span = left.span.merge(right.span);
            left = Spanned::new(
                Expr::Binary(Box::new(left), BinaryOp::Or, Box::new(right)),
                span,
            );
        }
        Ok(left)
    }

    fn and_expr(&mut self) -> Result<Spanned<Expr>, CompileError> {
        let mut left = self.not_expr()?;
        while self.match_token(&TokenKind::And) {
            let right = self.not_expr()?;
            let span = left.span.merge(right.span);
            left = Spanned::new(
                Expr::Binary(Box::new(left), BinaryOp::And, Box::new(right)),
                span,
            );
        }
        Ok(left)
    }

    fn not_expr(&mut self) -> Result<Spanned<Expr>, CompileError> {
        if self.match_token(&TokenKind::Not) {
            let start = self.tokens[self.pos - 1].span.start;
            let expr = self.not_expr()?;
            let span = Span::new(start, expr.span.end);
            Ok(Spanned::new(Expr::Unary(UnaryOp::Not, Box::new(expr)), span))
        } else {
            self.comparison()
        }
    }

    fn comparison(&mut self) -> Result<Spanned<Expr>, CompileError> {
        let mut left = self.range_expr()?;

        loop {
            let op = if self.match_token(&TokenKind::EqEq) {
                BinaryOp::Eq
            } else if self.match_token(&TokenKind::NotEq) {
                BinaryOp::NotEq
            } else if self.match_token(&TokenKind::Lt) {
                BinaryOp::Lt
            } else if self.match_token(&TokenKind::Gt) {
                BinaryOp::Gt
            } else if self.match_token(&TokenKind::LtEq) {
                BinaryOp::LtEq
            } else if self.match_token(&TokenKind::GtEq) {
                BinaryOp::GtEq
            } else if self.match_token(&TokenKind::In) {
                BinaryOp::In
            } else if self.check_keyword(&TokenKind::Not) && self.peek_next().kind == TokenKind::In {
                self.advance(); // not
                self.advance(); // in
                BinaryOp::NotIn
            } else if self.match_token(&TokenKind::Is) {
                BinaryOp::Is
            } else {
                break;
            };

            let right = self.range_expr()?;
            let span = left.span.merge(right.span);
            left = Spanned::new(Expr::Binary(Box::new(left), op, Box::new(right)), span);
        }

        Ok(left)
    }

    /// Parse range expressions: `start..end` or `start..=end`
    fn range_expr(&mut self) -> Result<Spanned<Expr>, CompileError> {
        let left = self.additive()?;

        // Check for range operators
        let is_inclusive = if self.match_token(&TokenKind::DotDotEq) {
            true
        } else if self.match_token(&TokenKind::DotDot) {
            false
        } else {
            return Ok(left);
        };

        let right = self.additive()?;
        let span = left.span.merge(right.span);

        Ok(Spanned::new(
            Expr::Range {
                start: Box::new(left),
                end: Box::new(right),
                inclusive: is_inclusive,
            },
            span,
        ))
    }

    fn additive(&mut self) -> Result<Spanned<Expr>, CompileError> {
        let mut left = self.multiplicative()?;

        loop {
            let op = if self.match_token(&TokenKind::Plus) {
                BinaryOp::Add
            } else if self.match_token(&TokenKind::Minus) {
                BinaryOp::Sub
            } else {
                break;
            };

            let right = self.multiplicative()?;
            let span = left.span.merge(right.span);
            left = Spanned::new(Expr::Binary(Box::new(left), op, Box::new(right)), span);
        }

        Ok(left)
    }

    fn multiplicative(&mut self) -> Result<Spanned<Expr>, CompileError> {
        let mut left = self.unary()?;

        loop {
            let op = if self.match_token(&TokenKind::Star) {
                BinaryOp::Mul
            } else if self.match_token(&TokenKind::Slash) {
                BinaryOp::Div
            } else if self.match_token(&TokenKind::Percent) {
                BinaryOp::Mod
            } else {
                break;
            };

            let right = self.unary()?;
            let span = left.span.merge(right.span);
            left = Spanned::new(Expr::Binary(Box::new(left), op, Box::new(right)), span);
        }

        Ok(left)
    }

    fn unary(&mut self) -> Result<Spanned<Expr>, CompileError> {
        if self.match_token(&TokenKind::Minus) {
            let start = self.tokens[self.pos - 1].span.start;
            let expr = self.unary()?;
            let span = Span::new(start, expr.span.end);
            Ok(Spanned::new(Expr::Unary(UnaryOp::Neg, Box::new(expr)), span))
        } else if self.match_token(&TokenKind::Await) {
            let start = self.tokens[self.pos - 1].span.start;
            let expr = self.unary()?;
            let span = Span::new(start, expr.span.end);
            Ok(Spanned::new(Expr::Await(Box::new(expr)), span))
        } else {
            self.postfix()
        }
    }

    fn postfix(&mut self) -> Result<Spanned<Expr>, CompileError> {
        let mut expr = self.primary()?;

        loop {
            if self.match_token(&TokenKind::Question) {
                let span = Span::new(expr.span.start, self.tokens[self.pos - 1].span.end);
                expr = Spanned::new(Expr::Try(Box::new(expr)), span);
            } else if self.match_token(&TokenKind::Dot) {
                // Check for tuple index access (.0, .1, etc) vs field/method access
                if let TokenKind::Int(n) = &self.peek().kind {
                    // Tuple index access: expr.0, expr.1
                    let idx = *n;
                    self.advance();
                    let span = Span::new(expr.span.start, self.tokens[self.pos - 1].span.end);
                    // Use the index as a string field name
                    expr = Spanned::new(Expr::Field(Box::new(expr), idx.to_string()), span);
                } else {
                    let name = self.identifier()?;
                    if self.match_token(&TokenKind::LParen) {
                        let args = self.call_args()?;
                        self.expect(&TokenKind::RParen, "Expected ')' after arguments")?;
                        let span = Span::new(expr.span.start, self.tokens[self.pos - 1].span.end);
                        expr = Spanned::new(Expr::MethodCall(Box::new(expr), name, args), span);
                    } else {
                        let span = Span::new(expr.span.start, self.tokens[self.pos - 1].span.end);
                        expr = Spanned::new(Expr::Field(Box::new(expr), name), span);
                    }
                }
            } else if self.match_token(&TokenKind::LBracket) {
                // Check for slice syntax: [start:end] or [start:end:step]
                let result = self.index_or_slice()?;
                self.expect(&TokenKind::RBracket, "Expected ']' after index/slice")?;
                let span = Span::new(expr.span.start, self.tokens[self.pos - 1].span.end);
                expr = match result {
                    IndexOrSlice::Index(index) => {
                        Spanned::new(Expr::Index(Box::new(expr), Box::new(index)), span)
                    }
                    IndexOrSlice::Slice(slice) => {
                        Spanned::new(Expr::Slice(Box::new(expr), slice), span)
                    }
                };
            } else if self.match_token(&TokenKind::LParen) {
                let args = self.call_args()?;
                self.expect(&TokenKind::RParen, "Expected ')' after arguments")?;
                let span = Span::new(expr.span.start, self.tokens[self.pos - 1].span.end);
                expr = Spanned::new(Expr::Call(Box::new(expr), args), span);
            } else {
                break;
            }
        }

        Ok(expr)
    }

    /// Parse index or slice expression inside brackets
    /// Handles: [expr], [start:end], [start:end:step], [:end], [start:], [::step]
    fn index_or_slice(&mut self) -> Result<IndexOrSlice, CompileError> {
        // Check for immediate colon (slice starting with no start value)
        if self.check(&TokenKind::Colon) {
            return self.parse_slice(None);
        }
        
        // Check for immediate closing bracket (not valid, but let expression handle error)
        if self.check(&TokenKind::RBracket) {
            return Err(CompileError::syntax(
                "Empty index is not allowed".to_string(),
                self.current_span(),
            ));
        }
        
        // Parse first expression
        let first = self.expression()?;
        
        // Check if this is a slice (has colon after first expression)
        if self.check(&TokenKind::Colon) {
            return self.parse_slice(Some(first));
        }
        
        // Just a regular index
        Ok(IndexOrSlice::Index(first))
    }
    
    /// Parse slice syntax after optional start expression
    /// start is already parsed, now parse [:end[:step]]
    fn parse_slice(&mut self, start: Option<Spanned<Expr>>) -> Result<IndexOrSlice, CompileError> {
        // Consume the first colon
        self.expect(&TokenKind::Colon, "Expected ':' in slice")?;
        
        // Parse end (optional - check for ] or :)
        let end = if !self.check(&TokenKind::RBracket) && !self.check(&TokenKind::Colon) {
            Some(Box::new(self.expression()?))
        } else {
            None
        };
        
        // Parse step (optional - only if there's another colon)
        let step = if self.match_token(&TokenKind::Colon) {
            if !self.check(&TokenKind::RBracket) {
                Some(Box::new(self.expression()?))
            } else {
                None
            }
        } else {
            None
        };
        
        Ok(IndexOrSlice::Slice(SliceExpr {
            start: start.map(Box::new),
            end,
            step,
        }))
    }

    fn primary(&mut self) -> Result<Spanned<Expr>, CompileError> {
        let start = self.current_span().start;
        
        // Await expression
        if self.match_token(&TokenKind::Await) {
            let inner = self.expression()?;
            let end = inner.span.end;
            return Ok(Spanned::new(Expr::Await(Box::new(inner)), Span::new(start, end)));
        }

        // Yield expression (for fixtures/generators)
        if self.match_token(&TokenKind::Yield) {
            // yield can be followed by an expression or stand alone
            let end_span = self.tokens[self.pos - 1].span.end;
            if self.is_at_expr_start() {
                let inner = self.expression()?;
                let end = inner.span.end;
                return Ok(Spanned::new(Expr::Yield(Some(Box::new(inner))), Span::new(start, end)));
            } else {
                return Ok(Spanned::new(Expr::Yield(None), Span::new(start, end_span)));
            }
        }

        // Match expression
        if self.match_token(&TokenKind::Match) {
            return self.match_expr(start);
        }

        // If expression (when used as expression)
        if self.check_keyword(&TokenKind::If) {
            return self.if_expr(start);
        }

        // self
        if self.match_token(&TokenKind::SelfKw) {
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Expr::SelfExpr, Span::new(start, end)));
        }

        // Literals
        if let Some(lit) = self.try_literal() {
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Expr::Literal(lit), Span::new(start, end)));
        }

        // f-string
        if let TokenKind::FString(parts) = &self.peek().kind {
            let parts = parts.clone();
            let fstring_span = self.peek().span; // Capture span before advancing
            self.advance();
            let fparts = self.convert_fstring_parts(&parts, fstring_span);
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Expr::FString(fparts), Span::new(start, end)));
        }

        // List literal or comprehension
        if self.match_token(&TokenKind::LBracket) {
            return self.list_or_comp(start);
        }

        // Dict literal or comprehension
        if self.match_token(&TokenKind::LBrace) {
            return self.dict_or_comp(start);
        }

        // Parenthesized expression or tuple
        if self.match_token(&TokenKind::LParen) {
            return self.paren_or_tuple(start);
        }

        // Identifier (or constructor)
        if let TokenKind::Ident(name) = &self.peek().kind {
            let name = name.clone();
            self.advance();

            // Check if it's a constructor call (identifier followed by parentheses with named args)
            // This is tricky - we'll let the type checker figure it out
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Expr::Ident(name), Span::new(start, end)));
        }

        Err(CompileError::syntax(
            format!("Expected expression, found {:?}", self.peek().kind),
            self.current_span(),
        ))
    }

    fn try_literal(&mut self) -> Option<Literal> {
        match &self.peek().kind {
            TokenKind::Int(n) => {
                let n = *n;
                self.advance();
                Some(Literal::Int(n))
            }
            TokenKind::Float(f) => {
                let f = *f;
                self.advance();
                Some(Literal::Float(f))
            }
            TokenKind::String(s) => {
                let s = s.clone();
                self.advance();
                Some(Literal::String(s))
            }
            TokenKind::Bytes(b) => {
                let b = b.clone();
                self.advance();
                Some(Literal::Bytes(b))
            }
            TokenKind::True => {
                self.advance();
                Some(Literal::Bool(true))
            }
            TokenKind::False => {
                self.advance();
                Some(Literal::Bool(false))
            }
            TokenKind::None => {
                self.advance();
                Some(Literal::None)
            }
            _ => None,
        }
    }

    fn convert_fstring_parts(&self, parts: &[LexFStringPart], fstring_span: Span) -> Vec<FStringPart> {
        parts
            .iter()
            .map(|p| match p {
                LexFStringPart::Literal(s) => FStringPart::Literal(s.clone()),
                LexFStringPart::Expr(s) => {
                    // Parse simple field access chains like "user.name" or "obj.field.sub"
                    let expr = self.parse_fstring_expr(s);
                    // Use the f-string's span so errors point to the f-string, not line 1
                    FStringPart::Expr(Spanned::new(expr, fstring_span))
                }
            })
            .collect()
    }

    fn parse_fstring_expr(&self, s: &str) -> Expr {
        // Properly parse the expression string by lexing and parsing it
        use crate::frontend::lexer;
        
        // Try to lex and parse the expression
        if let Ok(mut tokens) = lexer::lex(s) {
            // Ensure we have an EOF token at the end for the parser
            if tokens.is_empty() || !matches!(tokens.last().map(|t| &t.kind), Some(TokenKind::Eof)) {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    span: Span::default(),
                });
            }
            
            if tokens.len() > 1 { // At least one real token plus EOF
                let mut parser = Parser::new(&tokens);
                if let Ok(expr) = parser.expression() {
                    return expr.node;
                }
            }
        }
        
        // Fallback: treat as simple identifier
        Expr::Ident(s.to_string())
    }

    fn match_expr(&mut self, start: usize) -> Result<Spanned<Expr>, CompileError> {
        let subject = self.expression()?;
        self.expect(&TokenKind::Colon, "Expected ':' after match subject")?;
        self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
        self.expect(&TokenKind::Indent, "Expected indented block")?;

        let mut arms = Vec::new();
        self.skip_newlines();
        while !self.check(&TokenKind::Dedent) && !self.is_at_end() {
            arms.push(self.match_arm()?);
            self.skip_newlines();
        }

        self.expect(&TokenKind::Dedent, "Expected dedent after match body")?;
        let end = self.tokens[self.pos - 1].span.end;
        Ok(Spanned::new(
            Expr::Match(Box::new(subject), arms),
            Span::new(start, end),
        ))
    }

    fn match_arm(&mut self) -> Result<Spanned<MatchArm>, CompileError> {
        let start = self.current_span().start;

        // Support both `case Pattern:` and `Pattern =>` syntax
        let pattern = if self.match_token(&TokenKind::Case) {
            let pat = self.pattern()?;
            
            // Check for optional guard: `case pattern if condition:`
            let guard = if self.match_token(&TokenKind::If) {
                Some(self.expression()?)
            } else {
                None
            };
            
            self.expect(&TokenKind::Colon, "Expected ':' after case pattern")?;
            
            // Check if inline or block
            if self.match_token(&TokenKind::Newline) {
                self.expect(&TokenKind::Indent, "Expected indented block")?;
                let body = self.block()?;
                self.expect(&TokenKind::Dedent, "Expected dedent after case body")?;
                let end = self.tokens[self.pos - 1].span.end;
                return Ok(Spanned::new(
                    MatchArm {
                        pattern: pat,
                        guard,
                        body: MatchBody::Block(body),
                    },
                    Span::new(start, end),
                ));
            } else {
                // Inline: could be expression or statement (like `return 0`)
                // Try parsing as a single statement and wrap in block
                let stmt = self.inline_statement()?;
                // Consume trailing newline after inline statement
                self.match_token(&TokenKind::Newline);
                let end = stmt.span.end;
                return Ok(Spanned::new(
                    MatchArm {
                        pattern: pat,
                        guard,
                        body: MatchBody::Block(vec![stmt]),
                    },
                    Span::new(start, end),
                ));
            }
        } else {
            self.pattern()?
        };

        // Rust-style => syntax
        self.expect(&TokenKind::FatArrow, "Expected '=>' after pattern")?;

        // Check for block or expression
        if self.match_token(&TokenKind::Newline) {
            self.expect(&TokenKind::Indent, "Expected indented block")?;
            let body = self.block()?;
            self.expect(&TokenKind::Dedent, "Expected dedent after arm body")?;
            let end = self.tokens[self.pos - 1].span.end;
            Ok(Spanned::new(
                MatchArm {
                    pattern,
                    guard: None,
                    body: MatchBody::Block(body),
                },
                Span::new(start, end),
            ))
        } else {
            let expr = self.expression()?;
            let end = expr.span.end;
            Ok(Spanned::new(
                MatchArm {
                    pattern,
                    guard: None,
                    body: MatchBody::Expr(expr),
                },
                Span::new(start, end),
            ))
        }
    }

    fn pattern(&mut self) -> Result<Spanned<Pattern>, CompileError> {
        let start = self.current_span().start;

        // Wildcard
        if let TokenKind::Ident(name) = &self.peek().kind {
            if name == "_" {
                self.advance();
                let end = self.tokens[self.pos - 1].span.end;
                return Ok(Spanned::new(Pattern::Wildcard, Span::new(start, end)));
            }
        }

        // Literal patterns
        if let Some(lit) = self.try_literal() {
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Pattern::Literal(lit), Span::new(start, end)));
        }

        // Tuple pattern
        if self.match_token(&TokenKind::LParen) {
            let mut patterns = Vec::new();
            if !self.check(&TokenKind::RParen) {
                loop {
                    patterns.push(self.pattern()?);
                    if !self.match_token(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RParen, "Expected ')' after tuple pattern")?;
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Pattern::Tuple(patterns), Span::new(start, end)));
        }

        // Identifier (binding) or constructor pattern
        if let TokenKind::Ident(name) = &self.peek().kind {
            let mut name = name.clone();
            self.advance();

            // Check for qualified pattern: Type.Variant or Type.Variant(args)
            if self.match_token(&TokenKind::Dot) {
                if let TokenKind::Ident(variant) = &self.peek().kind {
                    let variant = variant.clone();
                    self.advance();
                    // Build qualified name: "Type::Variant" for Rust
                    name = format!("{}::{}", name, variant);
                } else {
                    return Err(CompileError::syntax(
                        "Expected variant name after '.'".to_string(),
                        self.current_span(),
                    ));
                }
            }

            if self.match_token(&TokenKind::LParen) {
                // Constructor pattern: Some(x), Ok(value), Shape::Circle(r), etc.
                let mut patterns = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    loop {
                        patterns.push(self.pattern()?);
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RParen, "Expected ')' after constructor pattern")?;
                let end = self.tokens[self.pos - 1].span.end;
                return Ok(Spanned::new(
                    Pattern::Constructor(name, patterns),
                    Span::new(start, end),
                ));
            }

            // Check if this is a unit variant (qualified without parens): Type.Variant
            if name.contains("::") {
                let end = self.tokens[self.pos - 1].span.end;
                return Ok(Spanned::new(
                    Pattern::Constructor(name, vec![]),
                    Span::new(start, end),
                ));
            }

            // Just a binding
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Pattern::Binding(name), Span::new(start, end)));
        }

        Err(CompileError::syntax(
            format!("Expected pattern, found {:?}", self.peek().kind),
            self.current_span(),
        ))
    }

    fn if_expr(&mut self, start: usize) -> Result<Spanned<Expr>, CompileError> {
        self.expect(&TokenKind::If, "Expected 'if'")?;
        let condition = self.expression()?;
        self.expect(&TokenKind::Colon, "Expected ':' after if condition")?;
        self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
        self.expect(&TokenKind::Indent, "Expected indented block")?;
        let then_body = self.block()?;
        self.expect(&TokenKind::Dedent, "Expected dedent after if body")?;

        let else_body = if self.match_token(&TokenKind::Else) {
            self.expect(&TokenKind::Colon, "Expected ':' after else")?;
            self.expect(&TokenKind::Newline, "Expected newline after ':'")?;
            self.expect(&TokenKind::Indent, "Expected indented block")?;
            let body = self.block()?;
            self.expect(&TokenKind::Dedent, "Expected dedent after else body")?;
            Some(body)
        } else {
            None
        };

        let end = self.tokens[self.pos - 1].span.end;
        Ok(Spanned::new(
            Expr::If(Box::new(IfExpr {
                condition,
                then_body,
                else_body,
            })),
            Span::new(start, end),
        ))
    }

    fn list_or_comp(&mut self, start: usize) -> Result<Spanned<Expr>, CompileError> {
        // Implicit line continuation: skip newlines after [
        self.skip_newlines();
        
        // Empty list
        if self.match_token(&TokenKind::RBracket) {
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Expr::List(Vec::new()), Span::new(start, end)));
        }

        let first = self.expression()?;
        self.skip_newlines();

        // Check for comprehension
        if self.match_token(&TokenKind::For) {
            self.skip_newlines();
            let var = self.identifier()?;
            self.skip_newlines();
            self.expect(&TokenKind::In, "Expected 'in' in comprehension")?;
            self.skip_newlines();
            let iter = self.expression()?;
            self.skip_newlines();
            let filter = if self.match_token(&TokenKind::If) {
                self.skip_newlines();
                Some(self.expression()?)
            } else {
                None
            };
            self.skip_newlines();
            self.expect(&TokenKind::RBracket, "Expected ']' after comprehension")?;
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(
                Expr::ListComp(Box::new(ListComp {
                    expr: first,
                    var,
                    iter,
                    filter,
                })),
                Span::new(start, end),
            ));
        }

        // List literal
        let mut elements = vec![first];
        while self.match_token(&TokenKind::Comma) {
            self.skip_newlines();
            if self.check(&TokenKind::RBracket) {
                break;
            }
            elements.push(self.expression()?);
            self.skip_newlines();
        }
        self.expect(&TokenKind::RBracket, "Expected ']' after list")?;
        let end = self.tokens[self.pos - 1].span.end;
        Ok(Spanned::new(Expr::List(elements), Span::new(start, end)))
    }

    fn dict_or_comp(&mut self, start: usize) -> Result<Spanned<Expr>, CompileError> {
        // Implicit line continuation: skip newlines after {
        self.skip_newlines();
        
        // Empty dict/set
        if self.match_token(&TokenKind::RBrace) {
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Expr::Dict(Vec::new()), Span::new(start, end)));
        }

        let first = self.expression()?;
        self.skip_newlines();
        
        // Determine if this is a dict (has :) or set (no :)
        if self.match_token(&TokenKind::Colon) {
            self.skip_newlines();
            // It's a dict
            let first_value = self.expression()?;
            self.skip_newlines();

            // Check for comprehension
            if self.match_token(&TokenKind::For) {
                self.skip_newlines();
                let var = self.identifier()?;
                self.skip_newlines();
                self.expect(&TokenKind::In, "Expected 'in' in comprehension")?;
                self.skip_newlines();
                let iter = self.expression()?;
                self.skip_newlines();
                let filter = if self.match_token(&TokenKind::If) {
                    self.skip_newlines();
                    Some(self.expression()?)
                } else {
                    None
                };
                self.skip_newlines();
                self.expect(&TokenKind::RBrace, "Expected '}' after comprehension")?;
                let end = self.tokens[self.pos - 1].span.end;
                return Ok(Spanned::new(
                    Expr::DictComp(Box::new(DictComp {
                        key: first,
                        value: first_value,
                        var,
                        iter,
                        filter,
                    })),
                    Span::new(start, end),
                ));
            }

            // Dict literal
            let mut entries = vec![(first, first_value)];
            while self.match_token(&TokenKind::Comma) {
                self.skip_newlines();
                if self.check(&TokenKind::RBrace) {
                    break;
                }
                let key = self.expression()?;
                self.skip_newlines();
                self.expect(&TokenKind::Colon, "Expected ':' in dict entry")?;
                self.skip_newlines();
                let value = self.expression()?;
                self.skip_newlines();
                entries.push((key, value));
            }
            self.expect(&TokenKind::RBrace, "Expected '}' after dict")?;
            let end = self.tokens[self.pos - 1].span.end;
            Ok(Spanned::new(Expr::Dict(entries), Span::new(start, end)))
        } else {
            // It's a set literal: {expr, expr, ...}
            let mut elements = vec![first];
            while self.match_token(&TokenKind::Comma) {
                self.skip_newlines();
                if self.check(&TokenKind::RBrace) {
                    break;
                }
                elements.push(self.expression()?);
                self.skip_newlines();
            }
            self.expect(&TokenKind::RBrace, "Expected '}' after set")?;
            let end = self.tokens[self.pos - 1].span.end;
            Ok(Spanned::new(Expr::Set(elements), Span::new(start, end)))
        }
    }

    fn paren_or_tuple(&mut self, start: usize) -> Result<Spanned<Expr>, CompileError> {
        // Implicit line continuation: skip newlines after (
        self.skip_newlines();
        
        // Empty parens - could be () => expr (closure) or () (unit tuple)
        if self.match_token(&TokenKind::RParen) {
            // Check for arrow function: () => expr
            if self.match_token(&TokenKind::FatArrow) {
                self.skip_newlines();
                let body = self.expression()?;
                let end = body.span.end;
                return Ok(Spanned::new(Expr::Closure(Vec::new(), Box::new(body)), Span::new(start, end)));
            }
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Expr::Tuple(Vec::new()), Span::new(start, end)));
        }

        let first = self.expression()?;
        self.skip_newlines();

        // Check for tuple (needs comma)
        if self.match_token(&TokenKind::Comma) {
            self.skip_newlines();
            let mut elements = vec![first];
            if !self.check(&TokenKind::RParen) {
                loop {
                    elements.push(self.expression()?);
                    self.skip_newlines();
                    if !self.match_token(&TokenKind::Comma) {
                        break;
                    }
                    self.skip_newlines();
                    if self.check(&TokenKind::RParen) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RParen, "Expected ')' after tuple")?;
            
            // Check for arrow function: (x, y) => expr
            if self.match_token(&TokenKind::FatArrow) {
                self.skip_newlines();
                let params = self.exprs_to_params(&elements)?;
                let body = self.expression()?;
                let end = body.span.end;
                return Ok(Spanned::new(Expr::Closure(params, Box::new(body)), Span::new(start, end)));
            }
            
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(Spanned::new(Expr::Tuple(elements), Span::new(start, end)));
        }

        // Just parenthesized expression (or single-param closure)
        self.expect(&TokenKind::RParen, "Expected ')'")?;
        
        // Check for arrow function: (x) => expr
        if self.match_token(&TokenKind::FatArrow) {
            self.skip_newlines();
            let params = self.exprs_to_params(&[first.clone()])?;
            let body = self.expression()?;
            let end = body.span.end;
            return Ok(Spanned::new(Expr::Closure(params, Box::new(body)), Span::new(start, end)));
        }
        
        let end = self.tokens[self.pos - 1].span.end;
        Ok(Spanned::new(Expr::Paren(Box::new(first)), Span::new(start, end)))
    }
    
    /// Convert expressions to closure parameters
    /// Only identifiers are valid as closure params
    fn exprs_to_params(&self, exprs: &[Spanned<Expr>]) -> Result<Vec<Spanned<Param>>, CompileError> {
        let mut params = Vec::new();
        for expr in exprs {
            match &expr.node {
                Expr::Ident(name) => {
                    // Closure params have inferred types (represented as "_")
                    let inferred_ty = Spanned::new(Type::Simple("_".to_string()), expr.span);
                    params.push(Spanned::new(
                        Param {
                            name: name.clone(),
                            ty: inferred_ty,
                            default: None,
                        },
                        expr.span,
                    ));
                }
                _ => {
                    return Err(CompileError::syntax(
                        "Closure parameters must be identifiers".to_string(),
                        expr.span,
                    ));
                }
            }
        }
        Ok(params)
    }

    fn call_args(&mut self) -> Result<Vec<CallArg>, CompileError> {
        // Implicit line continuation: skip newlines after (
        self.skip_newlines();
        
        let mut args = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                // Check for named argument
                if let TokenKind::Ident(name) = &self.peek().kind {
                    let name = name.clone();
                    if self.peek_next().kind == TokenKind::Eq {
                        self.advance(); // consume name
                        self.advance(); // consume =
                        self.skip_newlines();
                        let value = self.expression()?;
                        self.skip_newlines();
                        args.push(CallArg::Named(name, value));
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                        self.skip_newlines();
                        continue;
                    }
                }
                let expr = self.expression()?;
                self.skip_newlines();
                args.push(CallArg::Positional(expr));
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
                self.skip_newlines();
            }
        }
        Ok(args)
    }

    // ========================================================================
    // Utilities
    // ========================================================================

    fn identifier(&mut self) -> Result<Ident, CompileError> {
        match &self.peek().kind {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(CompileError::syntax(
                format!("Expected identifier, found {:?}", self.peek().kind),
                self.current_span(),
            )),
        }
    }

    fn identifier_list(&mut self) -> Result<Vec<Ident>, CompileError> {
        let mut idents = vec![self.identifier()?];
        while self.match_token(&TokenKind::Comma) {
            idents.push(self.identifier()?);
        }
        Ok(idents)
    }

    fn string_literal(&mut self) -> Result<String, CompileError> {
        match &self.peek().kind {
            TokenKind::String(s) => {
                let s = s.clone();
                self.advance();
                Ok(s)
            }
            _ => Err(CompileError::syntax(
                format!("Expected string literal, found {:?}", self.peek().kind),
                self.current_span(),
            )),
        }
    }
}

/// Convenience function to parse a token stream
pub fn parse(tokens: &[Token]) -> Result<Program, Vec<CompileError>> {
    Parser::new(tokens).parse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::lexer;

    fn parse_str(source: &str) -> Result<Program, Vec<CompileError>> {
        let tokens = lexer::lex(source).map_err(|_| vec![])?;
        parse(&tokens)
    }

    #[test]
    fn test_unexpected_indent_at_toplevel_is_single_clear_error() {
        // We intentionally allow the lexer to emit INDENT/DEDENT tokens at the top-level.
        // The parser should produce a single clear error and avoid cascading failures.
        let source = "  x = 1\n";
        let err = parse_str(source).expect_err("Top-level indentation should be rejected by the parser");
        assert_eq!(err.len(), 1, "Parser should return exactly one error (no cascade)");
        assert!(
            err[0].message.contains("Expected declaration") && err[0].message.contains("Indent"),
            "Error message should clearly indicate the unexpected INDENT token; got: {}",
            err[0].message
        );
    }

    #[test]
    fn test_parse_model() {
        let source = r#"
model User:
  name: str
  age: int = 0
"#;
        let program = parse_str(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
        match &program.declarations[0].node {
            Declaration::Model(m) => {
                assert_eq!(m.name, "User");
                assert_eq!(m.fields.len(), 2);
            }
            _ => panic!("Expected model"),
        }
    }

    #[test]
    fn test_parse_function() {
        let source = r#"
def add(a: int, b: int) -> int:
  return a + b
"#;
        let program = parse_str(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
        match &program.declarations[0].node {
            Declaration::Function(f) => {
                assert_eq!(f.name, "add");
                assert_eq!(f.params.len(), 2);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_parse_import() {
        let source = "import polars::prelude as pl";
        let program = parse_str(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
        match &program.declarations[0].node {
            Declaration::Import(i) => {
                match &i.kind {
                    ImportKind::Module(path) => {
                        assert_eq!(path.segments, vec!["polars".to_string(), "prelude".to_string()]);
                        assert_eq!(path.parent_levels, 0);
                        assert!(!path.is_absolute);
                    }
                    _ => panic!("Expected module import"),
                }
                assert_eq!(i.alias, Some("pl".to_string()));
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_parse_match() {
        let source = r#"
def handle(opt: Option[int]) -> int:
  match opt:
    case Some(x):
      return x
    case None:
      return 0
"#;
        let program = parse_str(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
    }
}
