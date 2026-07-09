"""Strict JSON-object decoding for repository proof and capture scripts."""

from __future__ import annotations

import json
from enum import Enum
from pathlib import Path
from typing import TypeAlias, TypeVar, cast

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]
EnumValue = TypeVar("EnumValue", bound=Enum)


class JsonContractError(ValueError):
    """An input JSON document does not satisfy its producer contract."""


class JsonObjectReader:
    """Read required typed fields from one JSON object with path-rich errors."""

    def __init__(self, value: object, context: str) -> None:
        if not isinstance(value, dict):
            raise JsonContractError(
                f"{context} must be an object, got {type(value).__name__}"
            )
        self._value = cast(JsonObject, value)
        self.context = context

    @property
    def data(self) -> JsonObject:
        return self._value

    def value(self, key: str) -> JsonValue:
        if key not in self._value:
            raise JsonContractError(f"{self.context} missing required field `{key}`")
        return self._value[key]

    def optional_value(self, key: str) -> JsonValue:
        return self._value.get(key)

    def object(self, key: str) -> "JsonObjectReader":
        return JsonObjectReader(self.value(key), f"{self.context}.{key}")

    def optional_object(self, key: str) -> "JsonObjectReader | None":
        value = self.optional_value(key)
        if value is None:
            return None
        return JsonObjectReader(value, f"{self.context}.{key}")

    def objects(self, key: str) -> tuple["JsonObjectReader", ...]:
        values = self.value(key)
        if not isinstance(values, list):
            raise JsonContractError(f"{self.context}.{key} must be a list")
        return tuple(
            JsonObjectReader(value, f"{self.context}.{key}[{index}]")
            for index, value in enumerate(values)
        )

    def string(self, key: str, *, allow_empty: bool = False) -> str:
        value = self.value(key)
        if not isinstance(value, str) or (not allow_empty and not value.strip()):
            qualifier = "a string" if allow_empty else "a non-empty string"
            raise JsonContractError(f"{self.context}.{key} must be {qualifier}")
        return value

    def optional_string(self, key: str, *, allow_empty: bool = False) -> str | None:
        value = self.optional_value(key)
        if value is None:
            return None
        if not isinstance(value, str) or (not allow_empty and not value.strip()):
            qualifier = "a string" if allow_empty else "a non-empty string"
            raise JsonContractError(f"{self.context}.{key} must be {qualifier} or null")
        return value

    def nullable_string(self, key: str) -> str | None:
        value = self.value(key)
        if value is None:
            return None
        if not isinstance(value, str):
            raise JsonContractError(f"{self.context}.{key} must be a string or null")
        return value

    def null(self, key: str) -> None:
        value = self.value(key)
        if value is not None:
            raise JsonContractError(f"{self.context}.{key} must be null")

    def enum(self, key: str, enum_type: type[EnumValue]) -> EnumValue:
        value = self.string(key)
        try:
            return enum_type(value)
        except ValueError as exc:
            allowed = ", ".join(str(member.value) for member in enum_type)
            raise JsonContractError(
                f"{self.context}.{key} must be one of [{allowed}], got {value!r}"
            ) from exc

    def integer(self, key: str) -> int:
        value = self.value(key)
        if isinstance(value, bool) or not isinstance(value, int):
            raise JsonContractError(f"{self.context}.{key} must be an integer")
        return value

    def number(self, key: str) -> float:
        value = self.value(key)
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise JsonContractError(f"{self.context}.{key} must be numeric")
        return float(value)

    def nullable_number(self, key: str) -> float | None:
        value = self.value(key)
        if value is None:
            return None
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise JsonContractError(f"{self.context}.{key} must be numeric or null")
        return float(value)

    def boolean(self, key: str) -> bool:
        value = self.value(key)
        if not isinstance(value, bool):
            raise JsonContractError(f"{self.context}.{key} must be a boolean")
        return value

    def strings(self, key: str) -> tuple[str, ...]:
        value = self.value(key)
        if not isinstance(value, list) or not all(
            isinstance(item, str) for item in value
        ):
            raise JsonContractError(f"{self.context}.{key} must be a list of strings")
        return tuple(cast(list[str], value))


def decode_json_object(text: str, context: str) -> JsonObjectReader:
    try:
        value: object = json.loads(text)
    except json.JSONDecodeError as exc:
        raise JsonContractError(f"invalid JSON in {context}: {exc}") from exc
    return JsonObjectReader(value, context)


def read_json_object(path: Path) -> JsonObjectReader:
    return decode_json_object(path.read_text(), str(path))


def read_json_lines(path: Path) -> tuple[JsonObjectReader, ...]:
    rows: list[JsonObjectReader] = []
    for line_number, line in enumerate(path.read_text().splitlines(), start=1):
        if line.strip():
            rows.append(decode_json_object(line, f"{path}:{line_number}"))
    return tuple(rows)


def read_json_array_objects(path: Path) -> tuple[JsonObjectReader, ...]:
    try:
        value: object = json.loads(path.read_text())
    except json.JSONDecodeError as exc:
        raise JsonContractError(f"invalid JSON in {path}: {exc}") from exc
    if not isinstance(value, list):
        raise JsonContractError(f"{path} must be a list of objects")
    return tuple(
        JsonObjectReader(item, f"{path}[{index}]") for index, item in enumerate(value)
    )


def safe_filename_component(value: str, context: str) -> str:
    normalized = "".join(
        char if char.isascii() and (char.isalnum() or char in "_-") else "_"
        for char in value
    ).strip("_")
    if not normalized:
        raise JsonContractError(f"{context} contains no filename-safe characters")
    return normalized
