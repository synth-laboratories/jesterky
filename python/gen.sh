#!/usr/bin/env bash
# Regenerate the Python contract types from the pinned JSON Schema.
# Source of truth = the Rust jesterky-contract crate (ADR #1). Run from repo root.
set -euo pipefail
cd "$(dirname "$0")/.."

# 1. Re-emit the schema artifacts from the Rust types.
cargo run -q -p jesterky-contract --example emit_schema workflow > jesterky.schema.json
cargo run -q -p jesterky-contract --example emit_schema manifest > jesterky.manifest.schema.json
python3 python/normalize_schema_enums.py jesterky.schema.json jesterky.manifest.schema.json

# 2. Codegen pydantic v2 models. --disable-timestamp keeps the output byte-stable
#    so drift is detectable in CI.
gen() {
  uvx --from datamodel-code-generator datamodel-codegen \
    --input "$1" --input-file-type jsonschema \
    --output "python/jesterky/$2" --output-model-type pydantic_v2.BaseModel \
    --use-schema-description --target-python-version 3.10 \
    --enum-field-as-literal one --infer-union-variant-names \
    --disable-timestamp --formatters black
}
gen jesterky.schema.json spec.py
gen jesterky.manifest.schema.json manifest.py
python3 python/normalize_generated_models.py \
  python/jesterky/spec.py python/jesterky/manifest.py

echo "regenerated python/jesterky/{spec,manifest}.py"
