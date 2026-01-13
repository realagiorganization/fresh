//! TypeScript to JavaScript transpilation using oxc
//!
//! This module provides TypeScript transpilation without deno_ast,
//! using the oxc toolchain for parsing, transformation, and code generation.

use anyhow::{anyhow, Result};
use oxc_allocator::Allocator;
use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Transpile TypeScript source code to JavaScript
pub fn transpile_typescript(source: &str, filename: &str) -> Result<String> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(filename).unwrap_or_default();

    // Parse
    let parser_ret = Parser::new(&allocator, source, source_type).parse();
    if !parser_ret.errors.is_empty() {
        let errors: Vec<String> = parser_ret.errors.iter().map(|e| e.to_string()).collect();
        return Err(anyhow!("TypeScript parse errors: {}", errors.join("; ")));
    }

    let mut program = parser_ret.program;

    // Semantic analysis (required for transformer)
    let semantic_ret = SemanticBuilder::new().build(&program);

    if !semantic_ret.errors.is_empty() {
        let errors: Vec<String> = semantic_ret.errors.iter().map(|e| e.to_string()).collect();
        return Err(anyhow!("Semantic errors: {}", errors.join("; ")));
    }

    // Get scoping info for transformer
    let scoping = semantic_ret.semantic.into_scoping();

    // Transform (strip TypeScript types)
    let transform_options = TransformOptions::default();
    let transformer_ret = Transformer::new(
        &allocator,
        Path::new(filename),
        &transform_options,
    )
    .build_with_scoping(scoping, &mut program);

    if !transformer_ret.errors.is_empty() {
        let errors: Vec<String> = transformer_ret.errors.iter().map(|e| e.to_string()).collect();
        return Err(anyhow!("Transform errors: {}", errors.join("; ")));
    }

    // Generate JavaScript
    let codegen_ret = Codegen::new().build(&program);

    Ok(codegen_ret.code)
}

/// Check if source contains ES module syntax (imports or exports)
/// This determines if the code needs bundling to work with QuickJS eval
pub fn has_es_module_syntax(source: &str) -> bool {
    // Check for imports: import X from "...", import { X } from "...", import * as X from "..."
    let has_imports = source.contains("import ") && source.contains(" from ");
    // Check for exports: export const, export function, export class, export interface, etc.
    let has_exports = source.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("export ")
    });
    has_imports || has_exports
}

/// Check if source contains ES module imports (import ... from ...)
/// Kept for backwards compatibility
pub fn has_es_imports(source: &str) -> bool {
    source.contains("import ") && source.contains(" from ")
}

/// Bundle a module and all its local imports into a single file
/// Only handles relative imports (./path or ../path), not npm packages
pub fn bundle_module(entry_path: &Path) -> Result<String> {
    let mut visited = HashSet::new();
    let mut output = String::new();
    bundle_recursive(entry_path, &mut visited, &mut output)?;
    Ok(output)
}

fn bundle_recursive(
    path: &Path,
    visited: &mut HashSet<PathBuf>,
    output: &mut String,
) -> Result<()> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical.clone()) {
        return Ok(()); // Already bundled (circular import protection)
    }

    let source = std::fs::read_to_string(path)
        .map_err(|e| anyhow!("Failed to read {}: {}", path.display(), e))?;

    let imports = extract_local_imports(&source);
    let parent_dir = path.parent().unwrap_or(Path::new("."));

    // Resolve and bundle dependencies first (topological order)
    for import_path in imports {
        let resolved = resolve_import(&import_path, parent_dir)?;
        bundle_recursive(&resolved, visited, output)?;
    }

    // Strip imports/exports and transpile
    let stripped = strip_imports_and_exports(&source);
    let filename = path.to_str().unwrap_or("unknown.ts");
    let transpiled = transpile_typescript(&stripped, filename)?;
    output.push_str(&transpiled);
    output.push('\n');

    Ok(())
}

/// Extract local relative imports from source using AST parsing
/// Handles both imports and re-exports, including multi-line statements
/// Only extracts paths starting with ./ or ../
fn extract_local_imports(source: &str) -> Vec<String> {
    let allocator = Allocator::default();
    let source_type = SourceType::default().with_module(true).with_typescript(true);

    let parser_ret = Parser::new(&allocator, source, source_type).parse();
    if !parser_ret.errors.is_empty() {
        // If parsing fails, return empty (caller will handle the error during transpilation)
        return Vec::new();
    }

    let mut imports = Vec::new();

    for stmt in &parser_ret.program.body {
        let path = match stmt {
            // import ... from "path"
            Statement::ImportDeclaration(import_decl) => {
                Some(import_decl.source.value.as_str())
            }
            // export { X } from "path" or export * from "path"
            Statement::ExportNamedDeclaration(export_decl) => {
                export_decl.source.as_ref().map(|s| s.value.as_str())
            }
            Statement::ExportAllDeclaration(export_all) => {
                Some(export_all.source.value.as_str())
            }
            _ => None,
        };

        if let Some(path) = path {
            // Only include local imports (relative paths)
            if path.starts_with("./") || path.starts_with("../") {
                imports.push(path.to_string());
            }
        }
    }

    imports
}

/// Resolve an import path relative to the importing file's directory
fn resolve_import(import_path: &str, parent_dir: &Path) -> Result<PathBuf> {
    let base = parent_dir.join(import_path);

    // Try various extensions
    if base.exists() {
        return Ok(base);
    }

    let with_ts = base.with_extension("ts");
    if with_ts.exists() {
        return Ok(with_ts);
    }

    let with_js = base.with_extension("js");
    if with_js.exists() {
        return Ok(with_js);
    }

    // Try index files
    let index_ts = base.join("index.ts");
    if index_ts.exists() {
        return Ok(index_ts);
    }

    let index_js = base.join("index.js");
    if index_js.exists() {
        return Ok(index_js);
    }

    Err(anyhow!("Cannot resolve import '{}' from {}", import_path, parent_dir.display()))
}

/// Strip import statements and export keywords from source using AST transformation
/// Converts ES module syntax to plain JavaScript that QuickJS can eval
pub fn strip_imports_and_exports(source: &str) -> String {
    let allocator = Allocator::default();
    // Parse as module with TypeScript to accept import/export and TS syntax
    let source_type = SourceType::default().with_module(true).with_typescript(true);

    let parser_ret = Parser::new(&allocator, source, source_type).parse();
    if !parser_ret.errors.is_empty() {
        // If parsing fails, return original source (let transpiler handle errors)
        return source.to_string();
    }

    let mut program = parser_ret.program;

    // Transform the AST: remove imports, convert exports to declarations
    strip_module_syntax_ast(&allocator, &mut program);

    // Generate code from transformed AST
    let codegen_ret = Codegen::new().build(&program);
    codegen_ret.code
}

/// Strip ES module syntax from a program AST
/// - Removes ImportDeclaration statements
/// - Converts ExportNamedDeclaration to its inner declaration
/// - Handles ExportDefaultDeclaration, ExportAllDeclaration
fn strip_module_syntax_ast<'a>(
    allocator: &'a Allocator,
    program: &mut oxc_ast::ast::Program<'a>,
) {
    use oxc_allocator::Vec as OxcVec;

    // Collect transformed statements
    let mut new_body: OxcVec<'a, Statement<'a>> = OxcVec::with_capacity_in(program.body.len(), allocator);

    for stmt in program.body.drain(..) {
        match stmt {
            // Remove import declarations entirely
            Statement::ImportDeclaration(_) => {
                // Skip - dependency should already be bundled
            }

            // Convert export named declarations to their inner declaration
            Statement::ExportNamedDeclaration(export_decl) => {
                let inner = export_decl.unbox();
                if let Some(decl) = inner.declaration {
                    // Export has a declaration - keep just the declaration
                    // Convert Declaration to Statement
                    let stmt = declaration_to_statement(decl);
                    new_body.push(stmt);
                }
                // If no declaration (re-export like `export { X } from './y'`), skip
            }

            // Handle export default
            Statement::ExportDefaultDeclaration(export_default) => {
                let inner = export_default.unbox();
                match inner.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                        new_body.push(Statement::FunctionDeclaration(func));
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                        new_body.push(Statement::ClassDeclaration(class));
                    }
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {
                        // TypeScript interface - will be removed by transformer
                    }
                    _ => {
                        // Expression exports (export default expr) - skip
                    }
                }
            }

            // Remove export * declarations (re-exports)
            Statement::ExportAllDeclaration(_) => {
                // Skip
            }

            // Keep all other statements unchanged
            other => {
                new_body.push(other);
            }
        }
    }

    program.body = new_body;
}

/// Convert a Declaration to a Statement
fn declaration_to_statement(decl: Declaration<'_>) -> Statement<'_> {
    match decl {
        Declaration::VariableDeclaration(d) => Statement::VariableDeclaration(d),
        Declaration::FunctionDeclaration(d) => Statement::FunctionDeclaration(d),
        Declaration::ClassDeclaration(d) => Statement::ClassDeclaration(d),
        Declaration::TSTypeAliasDeclaration(d) => Statement::TSTypeAliasDeclaration(d),
        Declaration::TSInterfaceDeclaration(d) => Statement::TSInterfaceDeclaration(d),
        Declaration::TSEnumDeclaration(d) => Statement::TSEnumDeclaration(d),
        Declaration::TSModuleDeclaration(d) => Statement::TSModuleDeclaration(d),
        Declaration::TSImportEqualsDeclaration(d) => Statement::TSImportEqualsDeclaration(d),
        Declaration::TSGlobalDeclaration(d) => Statement::TSGlobalDeclaration(d),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transpile_basic_typescript() {
        let source = r#"
            const x: number = 42;
            function greet(name: string): string {
                return `Hello, ${name}!`;
            }
        "#;

        let result = transpile_typescript(source, "test.ts").unwrap();
        assert!(result.contains("const x = 42"));
        assert!(result.contains("function greet(name)"));
        assert!(!result.contains(": number"));
        assert!(!result.contains(": string"));
    }

    #[test]
    fn test_transpile_interface() {
        let source = r#"
            interface User {
                name: string;
                age: number;
            }
            const user: User = { name: "Alice", age: 30 };
        "#;

        let result = transpile_typescript(source, "test.ts").unwrap();
        assert!(!result.contains("interface"));
        assert!(result.contains("const user = {"));
    }

    #[test]
    fn test_transpile_type_alias() {
        let source = r#"
            type ID = number | string;
            const id: ID = 123;
        "#;

        let result = transpile_typescript(source, "test.ts").unwrap();
        assert!(!result.contains("type ID"));
        assert!(result.contains("const id = 123"));
    }

    #[test]
    fn test_has_es_imports() {
        assert!(has_es_imports("import { foo } from './lib'"));
        assert!(has_es_imports("import foo from 'bar'"));
        assert!(!has_es_imports("const x = 1;"));
        // Note: comment detection is a known limitation - simple heuristic doesn't parse JS
        // This is OK because false positives just mean we bundle when not strictly needed
        assert!(has_es_imports("// import foo from 'bar'")); // heuristic doesn't parse comments
    }

    #[test]
    fn test_extract_local_imports() {
        let source = r#"
            import { foo } from "./lib/utils";
            import bar from "../shared/bar";
            import external from "external-package";
            export { PanelManager } from "./panel-manager.ts";
            export * from "./types.ts";
            const x = 1;
        "#;

        let imports = extract_local_imports(source);
        assert_eq!(imports.len(), 4);
        assert!(imports.contains(&"./lib/utils".to_string()));
        assert!(imports.contains(&"../shared/bar".to_string()));
        assert!(imports.contains(&"./panel-manager.ts".to_string()));
        assert!(imports.contains(&"./types.ts".to_string()));
        // external-package should NOT be included
        assert!(!imports.iter().any(|i| i.contains("external")));
    }

    #[test]
    fn test_extract_local_imports_multiline() {
        // Test multi-line exports like in lib/index.ts
        let source = r#"
export type {
    RGB,
    Location,
    PanelOptions,
} from "./types.ts";

export {
    Finder,
    defaultFuzzyFilter,
} from "./finder.ts";

import {
    something,
    somethingElse,
} from "./multiline-import.ts";
        "#;

        let imports = extract_local_imports(source);
        assert_eq!(imports.len(), 3);
        assert!(imports.contains(&"./types.ts".to_string()));
        assert!(imports.contains(&"./finder.ts".to_string()));
        assert!(imports.contains(&"./multiline-import.ts".to_string()));
    }

    #[test]
    fn test_strip_imports_and_exports() {
        let source = r#"import { foo } from "./lib";
import bar from "../bar";
export const API_VERSION = 1;
export function greet() { return "hi"; }
export interface User { name: string; }
const x = foo() + bar();"#;

        let stripped = strip_imports_and_exports(source);
        // Imports are removed entirely
        assert!(!stripped.contains("import { foo }"));
        assert!(!stripped.contains("import bar from"));
        // Exports are converted to regular declarations
        assert!(!stripped.contains("export const"));
        assert!(!stripped.contains("export function"));
        assert!(!stripped.contains("export interface"));
        // But the declarations themselves remain
        assert!(stripped.contains("const API_VERSION = 1"));
        assert!(stripped.contains("function greet()"));
        assert!(stripped.contains("interface User"));
        assert!(stripped.contains("const x = foo() + bar();"));
    }
}
