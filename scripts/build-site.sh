#!/bin/bash
set -e

# Clean
rm -rf dist

# Build VitePress docs
# It will build to dist/docs because of outDir in config.ts
bun run docs:build

# Copy custom homepage to root of dist
cp homepage/index.html dist/

# Copy homepage assets to dist/public/assets
mkdir -p dist/public/assets
if [ -d "homepage/public/assets" ] && [ "$(ls -A homepage/public/assets)" ]; then
    cp -r homepage/public/assets/* dist/public/assets/
fi

# Add .nojekyll to bypass Jekyll on GitHub Pages
touch dist/.nojekyll

echo "Build complete! Output is in dist/"
