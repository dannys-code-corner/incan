#!/usr/bin/env python3
"""
Align GitHub-flavored Markdown tables in-place.

Why:
- markdownlint MD060 with style "aligned" requires table pipes to line up across rows.
- markdownlint-cli2 --fix does not reliably realign all tables.

What this does:
- Walks provided files/directories and rewrites *.md files in-place
- Skips content inside fenced code blocks
- For each table (header + delimiter + rows), normalizes:
  - `| col | col |` layout
  - consistent spacing around pipes
  - aligned pipe positions by padding cells to the max width for the column in that table

Limitations:
- Only handles "pipe tables" (lines containing `|` with a delimiter row).
"""

from __future__ import annotations

import argparse
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List, Optional, Tuple


def is_md_file(path: Path) -> bool:
    """Check if a path is a markdown file."""
    return path.is_file() and path.suffix.lower() == ".md"


def iter_md_files(paths: Iterable[Path]) -> Iterable[Path]:
    """Iterate over all markdown files in the given paths."""
    for p in paths:
        if p.is_dir():
            for root, _, files in os.walk(p):
                for f in files:
                    fp = Path(root) / f
                    if is_md_file(fp):
                        yield fp
        elif is_md_file(p):
            yield p


def split_row(line: str) -> List[str]:
    """
    Split a pipe-table row into cells (without surrounding pipes).

    Important: do NOT split on pipes inside inline code spans (backticks) or escaped pipes (\\|),
    because GFM tables treat those as literal content.
    """
    s = line.strip()
    if s.startswith("|"):
        s = s[1:]
    if s.endswith("|"):
        s = s[:-1]

    cells: List[str] = []
    buf: List[str] = []

    in_code = False
    code_tick_len: Optional[int] = None

    i = 0
    while i < len(s):
        ch = s[i]

        # Escaped pipe: \|
        # Keep it escaped in output (do NOT unescape), otherwise we would introduce a real
        # column separator and break the table (MD056).
        if ch == "\\" and i + 1 < len(s) and s[i + 1] == "|":
            buf.append("\\|")
            i += 2
            continue

        # Inline code span handling (supports multi-backtick spans)
        if ch == "`":
            j = i
            while j < len(s) and s[j] == "`":
                j += 1
            tick_len = j - i
            ticks = "`" * tick_len
            buf.append(ticks)

            if not in_code:
                in_code = True
                code_tick_len = tick_len
            else:
                if code_tick_len == tick_len:
                    in_code = False
                    code_tick_len = None

            i = j
            continue

        # Column separator (only when not inside code)
        if ch == "|" and not in_code:
            cells.append("".join(buf).strip())
            buf = []
            i += 1
            continue

        buf.append(ch)
        i += 1

    cells.append("".join(buf).strip())
    return cells


def is_delimiter_cell(cell: str) -> bool:
    # Accept common delimiter patterns: --- , :--- , ---: , :---:
    if not cell:
        return False
    c = cell.strip()
    left = c.startswith(":")
    right = c.endswith(":")
    core = c[1:-1] if (left and right) else (c[1:] if left else (c[:-1] if right else c))
    core = core.strip()
    return len(core) >= 3 and all(ch == "-" for ch in core)


def is_delimiter_row(line: str) -> bool:
    if "|" not in line:
        return False
    cells = split_row(line)
    return len(cells) >= 2 and all(is_delimiter_cell(c) for c in cells)


@dataclass(frozen=True)
class Table:
    start: int
    end: int  # inclusive
    rows: List[List[str]]  # includes header + delimiter + body rows


def find_tables(lines: List[str]) -> List[Table]:
    tables: List[Table] = []
    i = 0
    in_fence = False
    fence_marker: Optional[str] = None

    def toggle_fence(line: str) -> None:
        nonlocal in_fence, fence_marker
        stripped = line.lstrip()
        if stripped.startswith("```"):
            marker = "```"
        elif stripped.startswith("~~~"):
            marker = "~~~"
        else:
            return
        if not in_fence:
            in_fence = True
            fence_marker = marker
        elif fence_marker == marker:
            in_fence = False
            fence_marker = None

    while i < len(lines):
        toggle_fence(lines[i])
        if in_fence:
            i += 1
            continue

        # A table requires: header line with |, delimiter row next
        if "|" in lines[i] and i + 1 < len(lines) and is_delimiter_row(lines[i + 1]):
            start = i
            j = i
            # consume contiguous pipe lines (until blank or no pipe)
            while j < len(lines):
                # stop on blank line
                if not lines[j].strip():
                    break
                # stop if line doesn't contain pipe
                if "|" not in lines[j]:
                    break
                # stop if entering code fence
                nxt = lines[j]
                if nxt.lstrip().startswith("```") or nxt.lstrip().startswith("~~~"):
                    break
                j += 1

            end = j - 1
            raw_rows = [split_row(lines[k]) for k in range(start, end + 1)]
            tables.append(Table(start=start, end=end, rows=raw_rows))
            i = end + 1
            continue

        i += 1

    return tables


def align_table(table: Table) -> List[str]:
    # Determine number of columns as max across rows; pad missing cells.
    ncols = max(len(r) for r in table.rows)
    rows = [r + [""] * (ncols - len(r)) for r in table.rows]

    # Preserve delimiter alignment markers per column from delimiter row (second row).
    delim = rows[1]
    aligns: List[Tuple[bool, bool]] = []
    for c in delim:
        cs = c.strip()
        left = cs.startswith(":")
        right = cs.endswith(":")
        aligns.append((left, right))

    # Compute column widths from all non-delimiter rows.
    widths = [0] * ncols
    for ridx, r in enumerate(rows):
        if ridx == 1:
            continue
        for cidx, cell in enumerate(r):
            widths[cidx] = max(widths[cidx], len(cell))

    # Minimum width for delimiter cells is 3 (markdown spec), but align to content width.
    widths = [max(w, 3) for w in widths]

    out: List[str] = []
    for ridx, r in enumerate(rows):
        if ridx == 1:
            # delimiter row
            parts: List[str] = []
            for cidx in range(ncols):
                left, right = aligns[cidx]
                core_len = widths[cidx]
                core = "-" * core_len
                if left:
                    core = ":" + core[1:]
                if right:
                    core = core[:-1] + ":"
                parts.append(core)
            out.append("| " + " | ".join(parts) + " |")
        else:
            parts = [r[cidx].ljust(widths[cidx]) for cidx in range(ncols)]
            out.append("| " + " | ".join(parts) + " |")
    return out


def process_file(path: Path) -> bool:
    original = path.read_text(encoding="utf-8").splitlines()
    lines = original[:]

    tables = find_tables(lines)
    if not tables:
        return False

    # Apply bottom-up so indexes stay valid.
    changed = False
    for t in reversed(tables):
        # Must have at least header+delimiter and at least 2 columns
        if len(t.rows) < 2:
            continue
        if max(len(r) for r in t.rows) < 2:
            continue
        if not is_delimiter_row(lines[t.start + 1]):
            continue
        aligned = align_table(t)
        if lines[t.start : t.end + 1] != aligned:
            lines[t.start : t.end + 1] = aligned
            changed = True

    if changed:
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return changed


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("paths", nargs="+", help="Files/directories to process")
    ap.add_argument("--exclude", action="append", default=[], help="Path prefix (relative) to exclude")
    args = ap.parse_args()

    repo_root = Path.cwd()
    excludes = [str((repo_root / e).resolve()) for e in args.exclude]

    changed_files = 0
    for f in iter_md_files([Path(p) for p in args.paths]):
        resolved = str(f.resolve())
        if any(resolved.startswith(ex) for ex in excludes):
            continue
        if process_file(f):
            changed_files += 1

    print(f"Aligned tables in {changed_files} file(s).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())


