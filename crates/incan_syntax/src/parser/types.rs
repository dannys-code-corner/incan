/// Type-expression parsing methods.
///
/// This chunk parses syntactic type expressions (annotations), including:
/// - Simple names (`int`, `Foo`)
/// - Generic applications (`List[int]`)
/// - Tuple types (`(int, str)`)
/// - Function types (`(int, str) -> bool`)
///
/// ## Notes
/// - `Type` parsing is purely syntactic; semantic meaning is handled by later compiler phases.
impl<'a> Parser<'a> {
    // ========================================================================
    // Types
    // ========================================================================

    fn type_params(&mut self) -> Result<Vec<Ident>, CompileError> {
        if self.match_token(&TokenKind::Punctuation(PunctuationId::LBracket)) {
            let params = self.identifier_list()?;
            self.expect(
                &TokenKind::Punctuation(PunctuationId::RBracket),
                "Expected ']' after type parameters",
            )?;
            Ok(params)
        } else {
            Ok(Vec::new())
        }
    }

    fn type_expr(&mut self) -> Result<Spanned<Type>, CompileError> {
        let start = self.current_span().start;

        // Unit type
        if self.match_token(&TokenKind::Punctuation(PunctuationId::LParen)) {
            if self.match_token(&TokenKind::Punctuation(PunctuationId::RParen)) {
                // Could be unit type () or zero-arg function type () -> T
                if self.match_token(&TokenKind::Punctuation(PunctuationId::Arrow)) {
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
            if self.match_token(&TokenKind::Punctuation(PunctuationId::Comma)) {
                // Tuple type
                let mut types = vec![first];
                if !self.check(&TokenKind::Punctuation(PunctuationId::RParen)) {
                    loop {
                        types.push(self.type_expr()?);
                        if !self.match_token(&TokenKind::Punctuation(PunctuationId::Comma)) {
                            break;
                        }
                    }
                }
                self.expect(
                    &TokenKind::Punctuation(PunctuationId::RParen),
                    "Expected ')' after tuple type",
                )?;

                // Check for function type
                if self.match_token(&TokenKind::Punctuation(PunctuationId::Arrow)) {
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
            self.expect(&TokenKind::Punctuation(PunctuationId::RParen), "Expected ')'")?;

            // Check for function type
            if self.match_token(&TokenKind::Punctuation(PunctuationId::Arrow)) {
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
        if self.match_token(&TokenKind::Keyword(KeywordId::None)) {
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
        if self.match_token(&TokenKind::Punctuation(PunctuationId::LBracket)) {
            let args = self.type_list()?;
            self.expect(
                &TokenKind::Punctuation(PunctuationId::RBracket),
                "Expected ']' after type arguments",
            )?;
            let end = self.tokens[self.pos - 1].span.end;
            Ok(Spanned::new(Type::Generic(name, args), Span::new(start, end)))
        } else {
            let end = self.tokens[self.pos - 1].span.end;
            Ok(Spanned::new(Type::Simple(name), Span::new(start, end)))
        }
    }

    fn type_list(&mut self) -> Result<Vec<Spanned<Type>>, CompileError> {
        let mut types = Vec::new();
        if !self.check(&TokenKind::Punctuation(PunctuationId::RBracket))
            && !self.check(&TokenKind::Punctuation(PunctuationId::RParen))
        {
            loop {
                types.push(self.type_expr()?);
                if !self.match_token(&TokenKind::Punctuation(PunctuationId::Comma)) {
                    break;
                }
            }
        }
        Ok(types)
    }

}
