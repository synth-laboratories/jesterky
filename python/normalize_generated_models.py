#!/usr/bin/env python3
"""Repair datamodel-codegen's invalid nullable recursive forward references."""

from __future__ import annotations

import sys
from pathlib import Path

INVALID_FORWARD_REF = '"JsonValue" | None'
VALID_FORWARD_REF = '"JsonValue | None"'


class GeneratedModelNormalizeError(RuntimeError):
    """Generated Python did not match the pinned normalization contract."""


def normalize(path: Path) -> None:
    source = path.read_text()
    replacements = source.count(INVALID_FORWARD_REF)
    if replacements == 0:
        raise GeneratedModelNormalizeError(
            f"{path} does not contain expected generated form {INVALID_FORWARD_REF!r}"
        )
    normalized = source.replace(INVALID_FORWARD_REF, VALID_FORWARD_REF)
    compile(normalized, str(path), "exec")
    path.write_text(normalized)


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("usage: normalize_generated_models.py <model.py> [...]", file=sys.stderr)
        return 2
    for raw_path in argv[1:]:
        normalize(Path(raw_path))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
