//! Incan Language Server binary entry point
//!
//! Run with: incan-lsp
//!
//! The LSP communicates via stdin/stdout using the Language Server Protocol.

use tower_lsp::{LspService, Server};
use incan::lsp::IncanLanguageServer;

#[tokio::main]
async fn main() {
    // Create LSP service
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| IncanLanguageServer::new(client));

    // Run server
    Server::new(stdin, stdout, socket).serve(service).await;
}
