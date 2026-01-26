#!/bin/bash
# Validate a Fresh theme file against the schema
# Usage: ./scripts/validate-theme.sh path/to/theme.json
#
# Prerequisite: pip install jsonschema

set -e

if [ -z "$1" ]; then
    echo "Usage: $0 <theme.json>"
    exit 1
fi

THEME_FILE="$1"
SCRIPT_DIR="$(dirname "$0")"
SCHEMA_FILE="$SCRIPT_DIR/../crates/fresh-editor/plugins/schemas/theme.schema.json"

if [ ! -f "$SCHEMA_FILE" ]; then
    echo "Schema not found. Run ./scripts/gen_schema.sh first."
    exit 1
fi

python3 -c "
import json, jsonschema, sys

with open('$THEME_FILE') as f:
    data = json.load(f)

with open('$SCHEMA_FILE') as f:
    schema = json.load(f)

try:
    jsonschema.validate(data, schema)
    print('✓ Theme is valid')
except jsonschema.ValidationError as e:
    print(f'✗ Validation error at {e.json_path}: {e.message}')
    sys.exit(1)
"
