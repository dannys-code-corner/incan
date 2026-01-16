from __future__ import annotations

import re
from pathlib import Path
from typing import Iterable

from pygments.lexer import RegexLexer, bygroups, words
from pygments.lexers import _mapping
from pygments.token import (
    Comment,
    Keyword,
    Name,
    Number,
    Operator,
    Punctuation,
    String,
    Text,
)


def _load_keywords_from_registry() -> list[str]:
    """Load canonical keywords from the Rust registry file."""
    repo_root = Path(__file__).resolve().parents[2]
    registry_path = repo_root / "crates" / "incan_core" / "src" / "lang" / "keywords.rs"
    if not registry_path.exists():
        return []

    text = registry_path.read_text(encoding="utf-8", errors="replace")
    pattern = re.compile(
        r'info(?:_with_aliases)?\(\s*KeywordId::[A-Za-z_]+,\s*"([^"]+)"',
        re.DOTALL,
    )
    return sorted({match.group(1) for match in pattern.finditer(text)})


def _fallback_keywords() -> list[str]:
    """Fallback keywords if registry parsing fails."""
    return [
        "and",
        "as",
        "async",
        "await",
        "break",
        "case",
        "class",
        "const",
        "continue",
        "crate",
        "def",
        "elif",
        "else",
        "enum",
        "false",
        "for",
        "from",
        "if",
        "import",
        "in",
        "is",
        "let",
        "match",
        "model",
        "mut",
        "newtype",
        "none",
        "not",
        "or",
        "pass",
        "pub",
        "python",
        "return",
        "rust",
        "self",
        "super",
        "trait",
        "true",
        "type",
        "while",
        "with",
        "yield",
    ]


def _keywords() -> Iterable[str]:
    keywords = _load_keywords_from_registry()
    return keywords if keywords else _fallback_keywords()


class IncanLexer(RegexLexer):
    """Pygments lexer for the Incan programmi≠ng language."""

    name = "Incan"
    aliases = ["incan", "incn"]
    filenames = ["*.incn"]
    mimetypes = ["text/x-incan"]

    flags = re.MULTILINE | re.DOTALL

    tokens = {
        "root": [
            (r"\s+", Text),
            (r"#.*$", Comment.Single),
            (r"//.*$", Comment.Single),
            (r"/\*", Comment.Multiline, "comment"),
            (words(list(_keywords()), prefix=r"\b", suffix=r"\b"), Keyword),
            (r"\b(true|false|None)\b", Keyword.Constant),
            (r"\b0x[0-9a-fA-F_]+\b", Number.Hex),
            (r"\b0b[01_]+\b", Number.Bin),
            (r"\b0o[0-7_]+\b", Number.Oct),
            (r"\b\d+(_\d+)*(\.\d+(_\d+)*)?([eE][+-]?\d+(_\d+)*)?\b", Number),
            (r"(fr|rf|f|r)?('''|\"\"\").*?\\2", String),
            (r"(fr|rf|f|r)?(\"([^\"\\\\]|\\\\.)*\")", bygroups(String.Affix, String.Double)),
            (r"(fr|rf|f|r)?('([^'\\\\]|\\\\.)*')", bygroups(String.Affix, String.Single)),
            (r"==|!=|<=|>=|->|=>|:=|=", Operator),
            (r"[+\-*/%<>]", Operator),
            (r"[{}\[\](),.:]", Punctuation),
            (r"[A-Za-z_][A-Za-z0-9_]*", Name),
        ],
        "comment": [
            (r"[^*/]+", Comment.Multiline),
            (r"/\*", Comment.Multiline, "#push"),
            (r"\*/", Comment.Multiline, "#pop"),
            (r"[*/]", Comment.Multiline),
        ],
    }


def register_incan_lexer() -> None:
    """Register Incan lexer with Pygments for MkDocs builds."""

    if "IncanLexer" in _mapping.LEXERS:
        return

    _mapping.LEXERS["IncanLexer"] = (
        __name__,
        IncanLexer.name,
        tuple(IncanLexer.aliases),
        tuple(IncanLexer.filenames),
        tuple(IncanLexer.mimetypes),
    )


__all__ = ["IncanLexer"]
