from __future__ import annotations

import sys
from pathlib import Path


def on_config(config):  # noqa: D401 - MkDocs hook signature
    """Register custom Pygments lexers before Markdown rendering."""
    root = Path(__file__).resolve().parent
    if str(root) not in sys.path:
        sys.path.insert(0, str(root))

    from incan_pygments import register_incan_lexer

    register_incan_lexer()
    return config
