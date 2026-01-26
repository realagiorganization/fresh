#!/bin/bash
# Validate a Fresh package manifest (package.json)
# Usage: curl -sSL https://raw.githubusercontent.com/sinelaw/fresh/main/scripts/validate-package.sh | bash
#
# Prerequisite: pip install jsonschema

set -e

python3 -c "
import json, jsonschema, urllib.request, sys

with open('package.json') as f:
    data = json.load(f)

schema_url = data.get('\$schema')
if not schema_url:
    print('⚠ No \$schema field in package.json')
    sys.exit(0)

try:
    with urllib.request.urlopen(schema_url, timeout=5) as resp:
        schema = json.load(resp)
    jsonschema.validate(data, schema)
    print('✓ package.json is valid')
except urllib.error.URLError:
    print('⚠ Could not fetch schema (URL may not exist yet)')
except jsonschema.ValidationError as e:
    print(f'✗ Validation error: {e.message}')
    sys.exit(1)
"
