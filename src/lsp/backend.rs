//! LSP (Language Server Protocol) backend implementation for Incan

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::frontend::{lexer, parser, typechecker};
use crate::frontend::ast::{Program, Declaration, Span, Type};
use crate::lsp::diagnostics::{compile_error_to_diagnostic, span_to_range};

/// Document state stored by the LSP
#[derive(Debug, Clone)]
pub struct DocumentState {
    pub source: String,
    pub ast: Option<Program>,
    pub version: i32,
}

/// Incan Language Server
pub struct IncanLanguageServer {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
}

impl IncanLanguageServer {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Analyze a document and publish diagnostics
    async fn analyze_document(&self, uri: &Url, source: &str, version: i32) {
        let mut diagnostics = Vec::new();

        // Step 1: Lex
        let tokens = match lexer::lex(source) {
            Ok(tokens) => tokens,
            Err(errors) => {
                // Convert all lexer errors to diagnostics
                for error in &errors {
                    diagnostics.push(compile_error_to_diagnostic(error, source, uri));
                }
                self.client
                    .publish_diagnostics(uri.clone(), diagnostics, Some(version))
                    .await;
                return;
            }
        };

        // Step 2: Parse
        let ast = match parser::parse(&tokens) {
            Ok(ast) => ast,
            Err(errors) => {
                // Convert all parse errors to diagnostics
                for error in &errors {
                    diagnostics.push(compile_error_to_diagnostic(error, source, uri));
                }
                self.client
                    .publish_diagnostics(uri.clone(), diagnostics, Some(version))
                    .await;
                return;
            }
        };

        // Step 3: Type check
        let mut checker = typechecker::TypeChecker::new();
        if let Err(errors) = checker.check_program(&ast) {
            for error in &errors {
                diagnostics.push(compile_error_to_diagnostic(error, source, uri));
            }
        }

        // Store AST for hover/goto
        {
            let mut docs = self.documents.write().await;
            docs.insert(
                uri.clone(),
                DocumentState {
                    source: source.to_string(),
                    ast: Some(ast),
                    version,
                },
            );
        }

        // Publish diagnostics (even if empty, to clear old ones)
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, Some(version))
            .await;
    }

    /// Find the symbol at a position in the AST
    fn find_symbol_at_position(
        &self,
        ast: &Program,
        source: &str,
        position: Position,
    ) -> Option<SymbolInfo> {
        let offset = position_to_offset(source, position)?;

        for decl in &ast.declarations {
            if let Some(info) = self.find_in_declaration(&decl.node, decl.span, offset) {
                return Some(info);
            }
        }

        None
    }

    fn find_in_declaration(
        &self,
        decl: &Declaration,
        span: Span,
        offset: usize,
    ) -> Option<SymbolInfo> {
        match decl {
            Declaration::Function(func) => {
                if span.start <= offset && offset < span.end {
                    // Check if cursor is on function name
                    // For now, return the function signature
                    return Some(SymbolInfo {
                        name: func.name.clone(),
                        kind: "function".to_string(),
                        detail: format_function_signature(func),
                        span,
                    });
                }
            }
            Declaration::Model(model) => {
                if span.start <= offset && offset < span.end {
                    return Some(SymbolInfo {
                        name: model.name.clone(),
                        kind: "model".to_string(),
                        detail: format!("model {}", model.name),
                        span,
                    });
                }
            }
            Declaration::Class(class) => {
                if span.start <= offset && offset < span.end {
                    return Some(SymbolInfo {
                        name: class.name.clone(),
                        kind: "class".to_string(),
                        detail: format!("class {}", class.name),
                        span,
                    });
                }
            }
            Declaration::Trait(tr) => {
                if span.start <= offset && offset < span.end {
                    return Some(SymbolInfo {
                        name: tr.name.clone(),
                        kind: "trait".to_string(),
                        detail: format!("trait {}", tr.name),
                        span,
                    });
                }
            }
            Declaration::Enum(en) => {
                if span.start <= offset && offset < span.end {
                    return Some(SymbolInfo {
                        name: en.name.clone(),
                        kind: "enum".to_string(),
                        detail: format!("enum {}", en.name),
                        span,
                    });
                }
            }
            Declaration::Newtype(nt) => {
                if span.start <= offset && offset < span.end {
                    return Some(SymbolInfo {
                        name: nt.name.clone(),
                        kind: "newtype".to_string(),
                        detail: format!("newtype {} = {}", nt.name, format_type(&nt.underlying.node)),
                        span,
                    });
                }
            }
            _ => {}
        }

        None
    }

    /// Find the definition location of a symbol
    fn find_definition(
        &self,
        ast: &Program,
        name: &str,
    ) -> Option<Span> {
        for decl in &ast.declarations {
            match &decl.node {
                Declaration::Function(func) if func.name == name => {
                    return Some(decl.span);
                }
                Declaration::Model(model) if model.name == name => {
                    return Some(decl.span);
                }
                Declaration::Class(class) if class.name == name => {
                    return Some(decl.span);
                }
                Declaration::Trait(tr) if tr.name == name => {
                    return Some(decl.span);
                }
                Declaration::Enum(en) if en.name == name => {
                    return Some(decl.span);
                }
                Declaration::Newtype(nt) if nt.name == name => {
                    return Some(decl.span);
                }
                _ => {}
            }
        }
        None
    }
}

/// Symbol information for hover/goto
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: String,
    pub detail: String,
    pub span: Span,
}

/// Convert LSP Position to byte offset
fn position_to_offset(source: &str, position: Position) -> Option<usize> {
    let mut line = 0u32;
    let mut col = 0u32;
    let mut offset = 0usize;

    for (i, c) in source.char_indices() {
        if line == position.line && col == position.character {
            return Some(i);
        }
        if c == '\n' {
            if line == position.line {
                // Position is beyond line end
                return Some(i);
            }
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
        offset = i + c.len_utf8();
    }

    // Position at end of file
    if line == position.line && col == position.character {
        Some(offset)
    } else {
        None
    }
}

/// Format a function signature for display
fn format_function_signature(func: &crate::frontend::ast::FunctionDecl) -> String {
    let mut sig = String::new();

    if func.is_async {
        sig.push_str("async ");
    }

    sig.push_str("def ");
    sig.push_str(&func.name);
    sig.push('(');

    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            format!("{}: {}", p.node.name, format_type(&p.node.ty.node))
        })
        .collect();

    sig.push_str(&params.join(", "));
    sig.push(')');

    sig.push_str(" -> ");
    sig.push_str(&format_type(&func.return_type.node));

    sig
}

/// Format a Type for display
fn format_type(ty: &Type) -> String {
    match ty {
        Type::Simple(name) => name.clone(),
        Type::Generic(name, params) => {
            let params_str: Vec<String> = params.iter().map(|p| format_type(&p.node)).collect();
            format!("{}[{}]", name, params_str.join(", "))
        }
        Type::Tuple(types) => {
            let types_str: Vec<String> = types.iter().map(|t| format_type(&t.node)).collect();
            format!("({})", types_str.join(", "))
        }
        Type::Function(params, ret) => {
            let params_str: Vec<String> = params.iter().map(|p| format_type(&p.node)).collect();
            format!("({}) -> {}", params_str.join(", "), format_type(&ret.node))
        }
        Type::Unit => "()".to_string(),
        Type::SelfType => "Self".to_string(),
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for IncanLanguageServer {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                // Real-time diagnostics via text sync
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                // Hover support
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                // Go-to-definition
                definition_provider: Some(OneOf::Left(true)),
                // Completions (basic)
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string(), ":".to_string()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "incan-lsp".to_string(),
                version: Some("0.1.0".to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Incan LSP initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let source = params.text_document.text;
        let version = params.text_document.version;

        self.analyze_document(&uri, &source, version).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;

        // We use FULL sync, so there's only one change with the full content
        if let Some(change) = params.content_changes.into_iter().next() {
            self.analyze_document(&uri, &change.text, version).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;

        // Remove document from cache
        let mut docs = self.documents.write().await;
        docs.remove(&uri);

        // Clear diagnostics
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let docs = self.documents.read().await;
        let doc = match docs.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let ast = match &doc.ast {
            Some(ast) => ast,
            None => return Ok(None),
        };

        if let Some(info) = self.find_symbol_at_position(ast, &doc.source, position) {
            let markdown = format!(
                "```incan\n{}\n```\n\n*{}*",
                info.detail, info.kind
            );

            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: markdown,
                }),
                range: Some(span_to_range(&doc.source, info.span.start, info.span.end)),
            }));
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let docs = self.documents.read().await;
        let doc = match docs.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let ast = match &doc.ast {
            Some(ast) => ast,
            None => return Ok(None),
        };

        // Find what symbol the cursor is on
        if let Some(info) = self.find_symbol_at_position(ast, &doc.source, position) {
            // Find definition of that symbol
            if let Some(def_span) = self.find_definition(ast, &info.name) {
                let range = span_to_range(&doc.source, def_span.start, def_span.end);
                return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                    uri: uri.clone(),
                    range,
                })));
            }
        }

        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;

        let docs = self.documents.read().await;
        let doc = match docs.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let mut items = Vec::new();

        // Add keywords
        let keywords = [
            "def", "async", "await", "return", "if", "elif", "else", "match", "case",
            "for", "in", "while", "let", "mut", "model", "class", "trait", "enum",
            "newtype", "import", "from", "as", "with", "extends", "pub", "True", "False",
            "None", "Ok", "Err", "Some", "Result", "Option",
        ];

        for kw in keywords {
            items.push(CompletionItem {
                label: kw.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            });
        }

        // Add symbols from the current document
        if let Some(ast) = &doc.ast {
            for decl in &ast.declarations {
                match &decl.node {
                    Declaration::Function(func) => {
                        items.push(CompletionItem {
                            label: func.name.clone(),
                            kind: Some(CompletionItemKind::FUNCTION),
                            detail: Some(format_function_signature(func)),
                            ..Default::default()
                        });
                    }
                    Declaration::Model(model) => {
                        items.push(CompletionItem {
                            label: model.name.clone(),
                            kind: Some(CompletionItemKind::STRUCT),
                            detail: Some(format!("model {}", model.name)),
                            ..Default::default()
                        });
                    }
                    Declaration::Class(class) => {
                        items.push(CompletionItem {
                            label: class.name.clone(),
                            kind: Some(CompletionItemKind::CLASS),
                            detail: Some(format!("class {}", class.name)),
                            ..Default::default()
                        });
                    }
                    Declaration::Trait(tr) => {
                        items.push(CompletionItem {
                            label: tr.name.clone(),
                            kind: Some(CompletionItemKind::INTERFACE),
                            detail: Some(format!("trait {}", tr.name)),
                            ..Default::default()
                        });
                    }
                    Declaration::Enum(en) => {
                        items.push(CompletionItem {
                            label: en.name.clone(),
                            kind: Some(CompletionItemKind::ENUM),
                            detail: Some(format!("enum {}", en.name)),
                            ..Default::default()
                        });
                    }
                    _ => {}
                }
            }
        }

        Ok(Some(CompletionResponse::Array(items)))
    }
}
