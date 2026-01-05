#!/bin/bash
# WASM build script using wasm-pack
# Creates a deployable WASM bundle in dist/

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

echo "=== Fresh Editor WASM Build ==="
echo ""

# Check if wasm-pack is installed
if ! command -v wasm-pack &> /dev/null; then
    echo "wasm-pack not found. Installing..."
    cargo install wasm-pack
fi

# Check if wasm32 target is installed
if ! rustup target list --installed | grep -q "wasm32-unknown-unknown"; then
    echo "Installing wasm32-unknown-unknown target..."
    rustup target add wasm32-unknown-unknown
fi

# Clean previous build
echo "Cleaning previous build..."
rm -rf pkg/ dist/

# Build with wasm-pack
echo "Building WASM with wasm-pack..."
wasm-pack build --target web --no-default-features --features wasm

# Create dist directory with all assets
echo "Assembling dist directory..."
mkdir -p dist

# Copy wasm-pack output
cp pkg/*.js dist/
cp pkg/*.wasm dist/
cp pkg/*.d.ts dist/ 2>/dev/null || true

# Create a simple HTML loader that works with wasm-pack output
cat > dist/index.html << 'HTMLEOF'
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Fresh Editor</title>
    <link rel="stylesheet" href="styles.css">
</head>
<body>
    <div id="terminal"></div>
    <div id="loading">
        <div class="spinner"></div>
        <p>Loading Fresh Editor...</p>
    </div>
    <script type="module">
        import init from './fresh.js';

        async function run() {
            try {
                await init();
                document.getElementById('loading').style.display = 'none';
            } catch (e) {
                console.error('Failed to initialize:', e);
                document.getElementById('loading').innerHTML =
                    '<p style="color: #ff6b6b;">Failed to load: ' + e.message + '</p>';
            }
        }

        run();
    </script>
</body>
</html>
HTMLEOF

# Copy styles
cp web/styles.css dist/

echo ""
echo "=== Build Complete ==="
echo ""
echo "Output directory: dist/"
echo ""
echo "File sizes:"
ls -lh dist/*.wasm 2>/dev/null || echo "  (no WASM files found)"
ls -lh dist/*.js 2>/dev/null || echo "  (no JS files found)"
echo ""
echo "To test locally:"
echo "  cd dist && python3 -m http.server 8080"
echo "  Open http://localhost:8080"
echo ""
