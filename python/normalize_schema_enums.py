#!/usr/bin/env python3
"""Normalize enum shapes in generated JSON Schemas through a typed JSON AST."""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import Mapping, TypeAlias

JsonScalar: TypeAlias = None | bool | int | float | str


class SchemaErrorKind(str, Enum):
    READ = "read"
    SYNTAX = "syntax"
    CONTRACT = "contract"
    INVARIANT = "invariant"


@dataclass(frozen=True)
class SchemaFailure:
    kind: SchemaErrorKind
    path: str
    detail: str

    def render(self) -> str:
        return f"{self.kind.value} error at {self.path}: {self.detail}"


class SchemaNormalizeError(ValueError):
    """Base exception carrying a structured schema failure."""

    kind: SchemaErrorKind

    def __init__(self, path: str, detail: str) -> None:
        self.failure = SchemaFailure(self.kind, path, detail)
        super().__init__(self.failure.render())


class SchemaReadError(SchemaNormalizeError):
    """The schema file could not be read from storage."""

    kind = SchemaErrorKind.READ


class SchemaSyntaxError(SchemaNormalizeError):
    """The schema file is not syntactically valid JSON."""

    kind = SchemaErrorKind.SYNTAX


class SchemaContractError(SchemaNormalizeError):
    """The decoded JSON violates the generated-schema input contract."""

    kind = SchemaErrorKind.CONTRACT


class SchemaInvariantError(SchemaNormalizeError):
    """Typed normalization reached a state its decoder should make impossible."""

    kind = SchemaErrorKind.INVARIANT


@dataclass(frozen=True)
class JsonScalarNode:
    value: JsonScalar

    def to_python(self) -> JsonScalar:
        return self.value


@dataclass(frozen=True)
class JsonArrayNode:
    values: tuple[JsonNode, ...]

    def to_python(self) -> list[object]:
        return [value.to_python() for value in self.values]


@dataclass(frozen=True)
class JsonObjectNode:
    fields: Mapping[str, JsonNode]

    def to_python(self) -> dict[str, object]:
        return {key: value.to_python() for key, value in self.fields.items()}


JsonNode: TypeAlias = JsonScalarNode | JsonArrayNode | JsonObjectNode


@dataclass(frozen=True)
class GeneratedSchemaRoot:
    node: JsonObjectNode
    dialect: str
    title: str
    required_fields: tuple[str, ...]

    @classmethod
    def parse(cls, node: JsonObjectNode, *, path: str) -> GeneratedSchemaRoot:
        dialect = required_string_field(node, "$schema", path=path)
        title = required_string_field(node, "title", path=path)
        schema_type = required_string_field(node, "type", path=path)
        if schema_type != "object":
            raise SchemaContractError(f"{path}.type", "must be 'object'")
        required_object_field(node, "definitions", path=path)
        required_object_field(node, "properties", path=path)
        required_fields = required_string_array_field(node, "required", path=path)
        return cls(node, dialect, title, required_fields)


class OneOfFamily(str, Enum):
    STRING = "string"
    OBJECT = "object"
    OTHER = "other"


@dataclass(frozen=True)
class NoCollapse:
    """The schema is valid but does not match a supported enum-collapse shape."""


@dataclass(frozen=True)
class StringEnumCollapse:
    values: tuple[str, ...]
    description: JsonNode | None

    def as_schema(self) -> JsonObjectNode:
        fields: dict[str, JsonNode] = {
            "type": JsonScalarNode("string"),
            "enum": JsonArrayNode(tuple(JsonScalarNode(value) for value in self.values)),
        }
        if self.description is not None:
            fields["description"] = self.description
        return JsonObjectNode(fields)


@dataclass(frozen=True)
class TaggedObjectEnumCollapse:
    tag_field: str
    values: tuple[str, ...]
    description: JsonNode | None

    def as_schema(self) -> JsonObjectNode:
        tag_schema = JsonObjectNode(
            {
                "type": JsonScalarNode("string"),
                "enum": JsonArrayNode(
                    tuple(JsonScalarNode(value) for value in self.values)
                ),
            }
        )
        fields: dict[str, JsonNode] = {
            "type": JsonScalarNode("object"),
            "required": JsonArrayNode((JsonScalarNode(self.tag_field),)),
            "properties": JsonObjectNode({self.tag_field: tag_schema}),
        }
        if self.description is not None:
            fields["description"] = self.description
        return JsonObjectNode(fields)


CollapseDecision: TypeAlias = NoCollapse | StringEnumCollapse | TaggedObjectEnumCollapse


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("usage: normalize_schema_enums.py <schema.json> [...]", file=sys.stderr)
        return 2
    for raw_path in argv[1:]:
        normalize_schema_file(Path(raw_path))
    return 0


def normalize_schema_file(path: Path) -> None:
    schema = read_generated_schema(path)
    schema_with_json_value = inject_json_value_definition(schema, path=str(path))
    normalized = normalize_node(schema_with_json_value, path=str(path))
    if not isinstance(normalized, JsonObjectNode):
        raise SchemaInvariantError(str(path), "normalization produced a non-object schema")
    try:
        path.write_text(
            json.dumps(normalized.to_python(), indent=2, ensure_ascii=False) + "\n"
        )
    except OSError as err:
        raise SchemaReadError(str(path), f"unable to write normalized schema: {err}") from err


def inject_json_value_definition(
    schema: GeneratedSchemaRoot, *, path: str
) -> JsonObjectNode:
    definitions = required_object_field(schema.node, "definitions", path=path)
    fields = dict(schema.node.fields)
    definition_fields = dict(definitions.fields)
    definition_fields["JsonValue"] = json_value_definition()
    fields["definitions"] = JsonObjectNode(definition_fields)
    return JsonObjectNode(fields)


def json_value_definition() -> JsonObjectNode:
    reference = JsonObjectNode({"$ref": JsonScalarNode("#/definitions/JsonValue")})
    variants = (
        JsonObjectNode({"type": JsonScalarNode("null")}),
        JsonObjectNode({"type": JsonScalarNode("boolean")}),
        JsonObjectNode({"type": JsonScalarNode("integer")}),
        JsonObjectNode({"type": JsonScalarNode("number")}),
        JsonObjectNode({"type": JsonScalarNode("string")}),
        JsonObjectNode(
            {"type": JsonScalarNode("array"), "items": reference}
        ),
        JsonObjectNode(
            {
                "type": JsonScalarNode("object"),
                "additionalProperties": reference,
            }
        ),
    )
    return JsonObjectNode({"anyOf": JsonArrayNode(variants)})


def read_generated_schema(path: Path) -> GeneratedSchemaRoot:
    try:
        raw = path.read_text()
    except OSError as err:
        raise SchemaReadError(str(path), str(err)) from err
    try:
        decoded: object = json.loads(raw)
    except json.JSONDecodeError as err:
        raise SchemaSyntaxError(str(path), str(err)) from err
    node = decode_node(decoded, path=str(path))
    if not isinstance(node, JsonObjectNode):
        raise SchemaContractError(str(path), "must contain a JSON object schema")
    return GeneratedSchemaRoot.parse(node, path=str(path))


def required_string_field(node: JsonObjectNode, key: str, *, path: str) -> str:
    match node.fields.get(key):
        case JsonScalarNode(value=str(value)) if value.strip():
            return value
        case _:
            raise SchemaContractError(f"{path}.{key}", "must be a non-empty string")


def required_object_field(
    node: JsonObjectNode, key: str, *, path: str
) -> JsonObjectNode:
    value = node.fields.get(key)
    if not isinstance(value, JsonObjectNode):
        raise SchemaContractError(f"{path}.{key}", "must be an object")
    return value


def required_string_array_field(
    node: JsonObjectNode, key: str, *, path: str
) -> tuple[str, ...]:
    value = node.fields.get(key)
    if not isinstance(value, JsonArrayNode):
        raise SchemaContractError(f"{path}.{key}", "must be an array")
    strings: list[str] = []
    for index, item in enumerate(value.values):
        match item:
            case JsonScalarNode(value=str(string)) if string.strip():
                strings.append(string)
            case _:
                raise SchemaContractError(
                    f"{path}.{key}[{index}]", "must be a non-empty string"
                )
    return tuple(strings)


def decode_node(value: object, *, path: str) -> JsonNode:
    if value is None or isinstance(value, (bool, int, float, str)):
        return JsonScalarNode(value)
    if isinstance(value, list):
        return JsonArrayNode(
            tuple(
                decode_node(item, path=f"{path}[{index}]")
                for index, item in enumerate(value)
            )
        )
    if isinstance(value, dict):
        fields: dict[str, JsonNode] = {}
        for key, item in value.items():
            if not isinstance(key, str):
                raise SchemaContractError(path, "contains a non-string object key")
            fields[key] = decode_node(item, path=f"{path}.{key}")
        return JsonObjectNode(fields)
    raise SchemaContractError(
        path, f"contains unsupported JSON value {type(value).__name__}"
    )


def normalize_node(node: JsonNode, *, path: str) -> JsonNode:
    match node:
        case JsonScalarNode(value=True) if is_unconstrained_schema_path(path):
            return JsonObjectNode(
                {"$ref": JsonScalarNode("#/definitions/JsonValue")}
            )
        case JsonScalarNode():
            return node
        case JsonArrayNode(values=values):
            return JsonArrayNode(
                tuple(
                    normalize_node(value, path=f"{path}[{index}]")
                    for index, value in enumerate(values)
                )
            )
        case JsonObjectNode() if is_unconstrained_schema_object(node, path=path):
            annotations = {
                key: value
                for key, value in node.fields.items()
                if key in {"description", "default", "title"}
            }
            return JsonObjectNode(
                {
                    "$ref": JsonScalarNode("#/definitions/JsonValue"),
                    **annotations,
                }
            )
        case JsonObjectNode(fields=fields):
            decision = collapse_one_of(node, path=path)
            match decision:
                case StringEnumCollapse() | TaggedObjectEnumCollapse():
                    return decision.as_schema()
                case NoCollapse():
                    return JsonObjectNode(
                        {
                            key: normalize_node(value, path=f"{path}.{key}")
                            for key, value in fields.items()
                        }
                    )


def is_unconstrained_schema_path(path: str) -> bool:
    if ".default" in path:
        return False
    return ".properties." in path or ".oneOf[" in path or ".anyOf[" in path


def is_unconstrained_schema_object(node: JsonObjectNode, *, path: str) -> bool:
    if not is_unconstrained_schema_path(path):
        return False
    if path.endswith((".properties", ".definitions", ".patternProperties")):
        return False
    structural_keywords = {
        "$ref",
        "type",
        "enum",
        "const",
        "oneOf",
        "anyOf",
        "allOf",
        "not",
        "properties",
        "items",
        "additionalProperties",
    }
    return not structural_keywords.intersection(node.fields)


def collapse_one_of(node: JsonObjectNode, *, path: str) -> CollapseDecision:
    variants_node = node.fields.get("oneOf")
    if variants_node is None:
        return NoCollapse()
    if not isinstance(variants_node, JsonArrayNode):
        raise SchemaContractError(f"{path}.oneOf", "must be an array")
    if not variants_node.values:
        raise SchemaContractError(f"{path}.oneOf", "must not be empty")
    family = one_of_family(variants_node)
    description = node.fields.get("description")
    match family:
        case OneOfFamily.STRING:
            return string_enum_collapse(variants_node, description, path=path)
        case OneOfFamily.OBJECT:
            return tagged_object_enum_collapse(variants_node, description, path=path)
        case OneOfFamily.OTHER:
            return NoCollapse()


def one_of_family(variants: JsonArrayNode) -> OneOfFamily:
    families = {variant_family(variant) for variant in variants.values}
    if families == {OneOfFamily.STRING}:
        return OneOfFamily.STRING
    if families == {OneOfFamily.OBJECT}:
        return OneOfFamily.OBJECT
    return OneOfFamily.OTHER


def variant_family(variant: JsonNode) -> OneOfFamily:
    if not isinstance(variant, JsonObjectNode):
        return OneOfFamily.OTHER
    match variant.fields.get("type"):
        case JsonScalarNode(value="string"):
            return OneOfFamily.STRING
        case JsonScalarNode(value="object"):
            return OneOfFamily.OBJECT
        case _:
            return OneOfFamily.OTHER


def string_enum_collapse(
    variants: JsonArrayNode,
    description: JsonNode | None,
    *,
    path: str,
) -> CollapseDecision:
    enum_values: list[str] = []
    for variant in variants.values:
        if not isinstance(variant, JsonObjectNode):
            raise SchemaInvariantError(
                f"{path}.oneOf", "string family contains a non-object"
            )
        match variant.fields.get("enum"):
            case JsonArrayNode(values=(JsonScalarNode(value=str(value)),)):
                enum_values.append(value)
            case _:
                return NoCollapse()
    if len(set(enum_values)) != len(enum_values):
        raise SchemaContractError(
            f"{path}.oneOf", "has duplicate string enum variants"
        )
    return StringEnumCollapse(tuple(enum_values), description)


def tagged_object_enum_collapse(
    variants: JsonArrayNode,
    description: JsonNode | None,
    *,
    path: str,
) -> CollapseDecision:
    tag_field: str | None = None
    enum_values: list[str] = []
    for variant in variants.values:
        if not isinstance(variant, JsonObjectNode):
            raise SchemaInvariantError(
                f"{path}.oneOf", "object family contains a non-object"
            )
        required = variant.fields.get("required")
        properties = variant.fields.get("properties")
        match required:
            case JsonArrayNode(values=(JsonScalarNode(value=str(required_field)),)):
                pass
            case _:
                return NoCollapse()
        if not isinstance(properties, JsonObjectNode) or set(properties.fields) != {
            required_field
        }:
            return NoCollapse()
        if tag_field is None:
            tag_field = required_field
        elif tag_field != required_field:
            return NoCollapse()
        property_schema = properties.fields[required_field]
        if not isinstance(property_schema, JsonObjectNode):
            return NoCollapse()
        match (
            property_schema.fields.get("type"),
            property_schema.fields.get("enum"),
        ):
            case (
                JsonScalarNode(value="string"),
                JsonArrayNode(values=(JsonScalarNode(value=str(enum_value)),)),
            ):
                enum_values.append(enum_value)
            case _:
                return NoCollapse()
    if tag_field is None:
        raise SchemaInvariantError(f"{path}.oneOf", "object family is empty")
    if len(set(enum_values)) != len(enum_values):
        raise SchemaContractError(
            f"{path}.oneOf", f"has duplicate `{tag_field}` variants"
        )
    return TaggedObjectEnumCollapse(tag_field, tuple(enum_values), description)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
