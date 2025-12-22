#![no_main]

use libfuzzer_sys::fuzz_target;
use incan::frontend::{lexer, parser};

fuzz_target!(|data: &[u8]| {
    // Convert bytes to UTF-8 string (ignore invalid UTF-8)
    if let Ok(s) = std::str::from_utf8(data) {
        // Fuzz the lexer
        if let Ok(tokens) = lexer::lex(s) {
            // If lexing succeeds, fuzz the parser
            let _ = parser::parse(&tokens);
        }
    }
});
