//! Proc macros for type-safe plugin API bindings
//!
//! This crate provides the `#[plugin_api]` attribute macro that generates
//! QuickJS bindings and TypeScript definitions from a trait definition.
//!
//! # Return Type Convention
//!
//! The method's return type determines how it's exposed to JavaScript:
//!
//! - `T` (any type) → Sync method, returns value directly
//! - `Promise<T>` → Async method returning `Promise<T>` in JS
//! - `Thenable<T>` → Async method returning a thenable with `.kill()` support
//!
//! # Example
//!
//! ```rust,ignore
//! use fresh_plugin_api_macros::plugin_api;
//!
//! #[plugin_api]
//! pub trait EditorApi {
//!     /// Get the active buffer ID
//!     fn get_active_buffer_id(&self) -> u32;
//!
//!     /// Delay execution
//!     fn delay(&self, duration_ms: u64) -> Promise<()>;
//!
//!     /// Spawn a process (cancellable)
//!     fn spawn_process(&self, command: String, args: Vec<String>) -> Thenable<SpawnResult>;
//! }
//! ```

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, Attribute, FnArg, GenericArgument, Ident, ItemTrait, Meta, Pat,
    PathArguments, ReturnType, TraitItem, Type,
};

/// API method kind determined by return type
#[derive(Debug, Clone, PartialEq)]
enum ApiKind {
    /// Synchronous method - returns value directly
    Sync,
    /// Async method that returns a simple Promise<T>
    AsyncPromise,
    /// Async method that returns a Thenable<T> (with .kill() support)
    AsyncThenable,
}

/// Parsed API method information
#[derive(Debug)]
struct ApiMethod {
    /// Rust method name (snake_case)
    rust_name: Ident,
    /// JavaScript method name (camelCase)
    js_name: String,
    /// Method kind (sync/async)
    kind: ApiKind,
    /// Parameters: (name, type)
    params: Vec<(Ident, Type)>,
    /// Return type (inner type for Promise/Thenable)
    return_type: Option<Type>,
    /// Doc comment for TypeScript
    doc_comment: String,
}

/// Convert snake_case to camelCase
fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;
    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// Check if a type path matches a given name (e.g., "Promise", "Thenable")
fn type_path_matches(ty: &Type, name: &str) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            return segment.ident == name;
        }
    }
    false
}

/// Extract the inner type from Promise<T> or Thenable<T>
fn extract_inner_type(ty: &Type) -> Option<Type> {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if let PathArguments::AngleBracketed(args) = &segment.arguments {
                if let Some(GenericArgument::Type(inner)) = args.args.first() {
                    return Some(inner.clone());
                }
            }
        }
    }
    None
}

/// Determine the API kind from the return type
fn determine_api_kind(return_type: &Option<Type>) -> (ApiKind, Option<Type>) {
    match return_type {
        Some(ty) if type_path_matches(ty, "Promise") => {
            (ApiKind::AsyncPromise, extract_inner_type(ty))
        }
        Some(ty) if type_path_matches(ty, "Thenable") => {
            (ApiKind::AsyncThenable, extract_inner_type(ty))
        }
        other => (ApiKind::Sync, other.clone()),
    }
}

/// Extract doc comments from attributes
fn extract_doc_comment(attrs: &[Attribute]) -> String {
    let mut docs = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(expr_lit) = &meta.value {
                    if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                        docs.push(lit_str.value().trim().to_string());
                    }
                }
            }
        }
    }
    docs.join("\n")
}

/// Check for #[api(skip)] attribute
fn should_skip(attrs: &[Attribute]) -> bool {
    for attr in attrs {
        if attr.path().is_ident("api") {
            if let Meta::List(meta_list) = &attr.meta {
                let tokens = meta_list.tokens.to_string();
                if tokens.contains("skip") {
                    return true;
                }
            }
        }
    }
    false
}

/// Check for #[api(js_name = "...")] attribute and extract custom JS name
fn get_custom_js_name(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("api") {
            if let Meta::List(meta_list) = &attr.meta {
                let tokens = meta_list.tokens.to_string();
                // Simple parsing for js_name = "..."
                if let Some(start) = tokens.find("js_name") {
                    let rest = &tokens[start..];
                    if let Some(eq_pos) = rest.find('=') {
                        let after_eq = rest[eq_pos + 1..].trim();
                        if after_eq.starts_with('"') {
                            if let Some(end_quote) = after_eq[1..].find('"') {
                                return Some(after_eq[1..end_quote + 1].to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Parse a trait method into an ApiMethod
fn parse_method(item: &TraitItem) -> Option<ApiMethod> {
    if let TraitItem::Fn(method) = item {
        // Skip methods marked with #[api(skip)]
        if should_skip(&method.attrs) {
            return None;
        }

        let rust_name = method.sig.ident.clone();

        // Get JS name: custom or converted from snake_case
        let js_name = get_custom_js_name(&method.attrs)
            .unwrap_or_else(|| to_camel_case(&rust_name.to_string()));

        let doc_comment = extract_doc_comment(&method.attrs);

        // Parse parameters (skip &self and &mut self)
        let params: Vec<(Ident, Type)> = method
            .sig
            .inputs
            .iter()
            .filter_map(|arg| {
                if let FnArg::Typed(pat_type) = arg {
                    if let Pat::Ident(pat_ident) = &*pat_type.pat {
                        return Some((pat_ident.ident.clone(), (*pat_type.ty).clone()));
                    }
                }
                None
            })
            .collect();

        // Parse return type
        let raw_return_type = match &method.sig.output {
            ReturnType::Default => None,
            ReturnType::Type(_, ty) => Some((**ty).clone()),
        };

        // Determine kind and inner return type
        let (kind, return_type) = determine_api_kind(&raw_return_type);

        Some(ApiMethod {
            rust_name,
            js_name,
            kind,
            params,
            return_type,
            doc_comment,
        })
    } else {
        None
    }
}

/// Convert a Rust type to TypeScript type string
fn rust_type_to_typescript(ty: &Type) -> String {
    match ty {
        Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                let ident = segment.ident.to_string();
                match ident.as_str() {
                    // Primitive types
                    "u8" | "u16" | "u32" | "i8" | "i16" | "i32" | "usize" | "isize" => {
                        "number".to_string()
                    }
                    "u64" | "i64" => "number".to_string(), // Note: may lose precision
                    "f32" | "f64" => "number".to_string(),
                    "bool" => "boolean".to_string(),
                    "String" => "string".to_string(),
                    "str" => "string".to_string(),

                    // Option<T> -> T | null
                    "Option" => {
                        if let Some(inner) = extract_inner_type(ty) {
                            format!("{} | null", rust_type_to_typescript(&inner))
                        } else {
                            "unknown | null".to_string()
                        }
                    }

                    // Vec<T> -> T[]
                    "Vec" => {
                        if let Some(inner) = extract_inner_type(ty) {
                            format!("{}[]", rust_type_to_typescript(&inner))
                        } else {
                            "unknown[]".to_string()
                        }
                    }

                    // HashMap -> Record<string, T>
                    "HashMap" => "Record<string, unknown>".to_string(),

                    // Unit type
                    "()" => "void".to_string(),

                    // Known API types - these should match interfaces in .d.ts
                    "BufferInfo" | "CursorInfo" | "ViewportInfo" | "SpawnResult"
                    | "BackgroundProcessResult" | "DirEntry" | "FileStat"
                    | "CreateVirtualBufferResult" | "PromptSuggestion"
                    | "TextPropertyEntry" => ident,

                    // Default: use the type name as-is (assumes it's a known interface)
                    _ => ident,
                }
            } else {
                "unknown".to_string()
            }
        }
        Type::Tuple(tuple) if tuple.elems.is_empty() => "void".to_string(),
        Type::Reference(reference) => rust_type_to_typescript(&reference.elem),
        _ => "unknown".to_string(),
    }
}

/// Generate TypeScript method signature
fn generate_ts_method(method: &ApiMethod) -> String {
    let mut lines = Vec::new();

    // Add doc comment
    if !method.doc_comment.is_empty() {
        lines.push("  /**".to_string());
        for line in method.doc_comment.lines() {
            lines.push(format!("   * {}", line));
        }
        lines.push("   */".to_string());
    }

    // For async methods, filter out callback_id from the TypeScript signature
    let is_async = matches!(method.kind, ApiKind::AsyncPromise | ApiKind::AsyncThenable);

    // Build parameter list
    let params: Vec<String> = method
        .params
        .iter()
        .filter(|(name, _)| !is_async || name != "callback_id")
        .map(|(name, ty)| {
            let ts_type = rust_type_to_typescript(ty);
            format!("{}: {}", to_camel_case(&name.to_string()), ts_type)
        })
        .collect();

    // Build return type
    let return_type = match &method.kind {
        ApiKind::Sync => {
            method
                .return_type
                .as_ref()
                .map(rust_type_to_typescript)
                .unwrap_or_else(|| "void".to_string())
        }
        ApiKind::AsyncPromise => {
            let inner = method
                .return_type
                .as_ref()
                .map(rust_type_to_typescript)
                .unwrap_or_else(|| "void".to_string());
            format!("Promise<{}>", inner)
        }
        ApiKind::AsyncThenable => {
            let inner = method
                .return_type
                .as_ref()
                .map(rust_type_to_typescript)
                .unwrap_or_else(|| "void".to_string());
            // Thenable returns ProcessHandle-like object
            format!("ProcessHandle<{}>", inner)
        }
    };

    lines.push(format!("  {}({}): {};", method.js_name, params.join(", "), return_type));

    lines.join("\n")
}

/// Generate QuickJS binding code for a sync method
fn generate_sync_binding(method: &ApiMethod) -> TokenStream2 {
    let js_name = &method.js_name;
    let rust_name = &method.rust_name;

    let param_names: Vec<_> = method.params.iter().map(|(name, _)| name).collect();
    let param_types: Vec<_> = method.params.iter().map(|(_, ty)| ty).collect();

    if param_names.is_empty() {
        quote! {
            {
                let api_impl = api_impl.clone();
                editor.set(#js_name, rquickjs::Function::new(ctx.clone(), move || {
                    api_impl.borrow().#rust_name()
                })?)?;
            }
        }
    } else {
        quote! {
            {
                let api_impl = api_impl.clone();
                editor.set(#js_name, rquickjs::Function::new(ctx.clone(), move |#(#param_names: #param_types),*| {
                    api_impl.borrow().#rust_name(#(#param_names),*)
                })?)?;
            }
        }
    }
}

/// Generate QuickJS binding code for an async Promise method
fn generate_async_promise_binding(method: &ApiMethod) -> TokenStream2 {
    let js_name = &method.js_name;
    let internal_name = format!("_{}Start", js_name);
    let rust_name = &method.rust_name;

    // Filter out callback_id from JS-exposed parameters (it's generated internally)
    let js_params: Vec<_> = method
        .params
        .iter()
        .filter(|(name, _)| name != "callback_id")
        .collect();

    let js_param_names: Vec<_> = js_params.iter().map(|(name, _)| name).collect();
    let js_param_types: Vec<_> = js_params.iter().map(|(_, ty)| ty).collect();

    // For calling the trait method, we pass callback_id first, then other params
    let call_param_names: Vec<_> = js_params.iter().map(|(name, _)| name).collect();

    if js_param_names.is_empty() {
        quote! {
            {
                let api_impl = api_impl.clone();
                let request_id = request_id.clone();
                editor.set(#internal_name, rquickjs::Function::new(ctx.clone(), move || -> u64 {
                    let id = {
                        let mut id_ref = request_id.borrow_mut();
                        let id = *id_ref;
                        *id_ref += 1;
                        id
                    };
                    api_impl.borrow().#rust_name(id);
                    id
                })?)?;
            }
            _async_promise_methods.push(#js_name);
        }
    } else {
        quote! {
            {
                let api_impl = api_impl.clone();
                let request_id = request_id.clone();
                editor.set(#internal_name, rquickjs::Function::new(ctx.clone(), move |#(#js_param_names: #js_param_types),*| -> u64 {
                    let id = {
                        let mut id_ref = request_id.borrow_mut();
                        let id = *id_ref;
                        *id_ref += 1;
                        id
                    };
                    api_impl.borrow().#rust_name(id, #(#call_param_names),*);
                    id
                })?)?;
            }
            _async_promise_methods.push(#js_name);
        }
    }
}

/// Generate QuickJS binding code for an async Thenable method
fn generate_async_thenable_binding(method: &ApiMethod) -> TokenStream2 {
    let js_name = &method.js_name;
    let internal_name = format!("_{}Start", js_name);
    let rust_name = &method.rust_name;

    // Filter out callback_id from JS-exposed parameters (it's generated internally)
    let js_params: Vec<_> = method
        .params
        .iter()
        .filter(|(name, _)| name != "callback_id")
        .collect();

    let js_param_names: Vec<_> = js_params.iter().map(|(name, _)| name).collect();
    let js_param_types: Vec<_> = js_params.iter().map(|(_, ty)| ty).collect();

    // For calling the trait method, we pass callback_id first, then other params
    let call_param_names: Vec<_> = js_params.iter().map(|(name, _)| name).collect();

    if js_param_names.is_empty() {
        quote! {
            {
                let api_impl = api_impl.clone();
                let request_id = request_id.clone();
                editor.set(#internal_name, rquickjs::Function::new(ctx.clone(), move || -> u64 {
                    let id = {
                        let mut id_ref = request_id.borrow_mut();
                        let id = *id_ref;
                        *id_ref += 1;
                        id
                    };
                    api_impl.borrow().#rust_name(id);
                    id
                })?)?;
            }
            _async_thenable_methods.push(#js_name);
        }
    } else {
        quote! {
            {
                let api_impl = api_impl.clone();
                let request_id = request_id.clone();
                editor.set(#internal_name, rquickjs::Function::new(ctx.clone(), move |#(#js_param_names: #js_param_types),*| -> u64 {
                    let id = {
                        let mut id_ref = request_id.borrow_mut();
                        let id = *id_ref;
                        *id_ref += 1;
                        id
                    };
                    api_impl.borrow().#rust_name(id, #(#call_param_names),*);
                    id
                })?)?;
            }
            _async_thenable_methods.push(#js_name);
        }
    }
}

/// Main proc macro implementation
#[proc_macro_attribute]
pub fn plugin_api(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemTrait);
    let trait_name = &input.ident;
    let trait_vis = &input.vis;
    let ts_const_name = format_ident!("{}_TYPESCRIPT_DEFINITIONS", trait_name.to_string().to_uppercase());
    let methods_const_name = format_ident!("{}_JS_METHODS", trait_name.to_string().to_uppercase());

    // Parse all methods
    let methods: Vec<ApiMethod> = input
        .items
        .iter()
        .filter_map(parse_method)
        .collect();

    // Generate TypeScript definitions
    let ts_methods: Vec<String> = methods.iter().map(generate_ts_method).collect();
    let ts_interface = format!(
        r#"/**
 * Fresh Editor TypeScript Plugin API
 *
 * AUTO-GENERATED FILE - DO NOT EDIT MANUALLY
 * Generated by fresh-plugin-api-macros
 */

/**
 * Get the editor API instance.
 * Plugins must call this at the top of their file to get a scoped editor object.
 */
declare function getEditor(): EditorAPI;

/** Handle for a cancellable async operation */
interface ProcessHandle<T> extends PromiseLike<T> {{
  /** Promise that resolves to the result when complete */
  readonly result: Promise<T>;
  /** Cancel/kill the operation. Returns true if cancelled, false if already completed */
  kill(): Promise<boolean>;
}}

/** Buffer identifier */
type BufferId = number;

/** Split identifier */
type SplitId = number;

/** Buffer information */
interface BufferInfo {{
  id: number;
  path: string;
  modified: boolean;
  length: number;
}}

/** Cursor information with optional selection */
interface CursorInfo {{
  position: number;
  selection?: {{ start: number; end: number }} | null;
}}

/** Viewport information */
interface ViewportInfo {{
  top_byte: number;
  left_column: number;
  width: number;
  height: number;
}}

/** Result from spawnProcess */
interface SpawnResult {{
  stdout: string;
  stderr: string;
  exit_code: number;
}}

/** Result from spawnBackgroundProcess */
interface BackgroundProcessResult {{
  process_id: number;
}}

/** Directory entry */
interface DirEntry {{
  name: string;
  is_file: boolean;
  is_dir: boolean;
}}

/** File stat information */
interface FileStat {{
  exists: boolean;
  is_file: boolean;
  is_dir: boolean;
  size: number;
  readonly: boolean;
}}

/** Prompt suggestion */
interface PromptSuggestion {{
  text: string;
  description?: string | null;
  value?: string | null;
  disabled?: boolean | null;
  keybinding?: string | null;
}}

/** Text property entry for virtual buffers */
interface TextPropertyEntry {{
  text: string;
  properties: Record<string, unknown>;
}}

/** Result from createVirtualBufferInSplit */
interface CreateVirtualBufferResult {{
  buffer_id: number;
  split_id?: number | null;
}}

/**
 * Main editor API interface
 */
interface EditorAPI {{
{}
}}
"#,
        ts_methods.join("\n\n")
    );

    // Generate JS wrapper code for async methods
    let async_promise_wrappers: Vec<String> = methods
        .iter()
        .filter(|m| m.kind == ApiKind::AsyncPromise)
        .map(|m| {
            format!(
                "_editorCore.{name} = _wrapAsync(_editorCore._{name}Start, \"{name}\");",
                name = m.js_name
            )
        })
        .collect();

    let async_thenable_wrappers: Vec<String> = methods
        .iter()
        .filter(|m| m.kind == ApiKind::AsyncThenable)
        .map(|m| {
            format!(
                "_editorCore.{name} = _wrapAsyncThenable(_editorCore._{name}Start, \"{name}\");",
                name = m.js_name
            )
        })
        .collect();

    let js_wrappers_code = [async_promise_wrappers, async_thenable_wrappers]
        .concat()
        .join("\n");

    // Collect JS method names for verification
    let js_method_names: Vec<String> = methods.iter().map(|m| m.js_name.clone()).collect();

    // Generate the output
    let expanded = quote! {
        // Original trait definition (preserved as-is)
        #input

        /// TypeScript definitions for the plugin API
        ///
        /// This constant contains the full .d.ts content that should be written
        /// to `plugins/lib/fresh.d.ts`.
        #trait_vis const #ts_const_name: &str = #ts_interface;

        /// JavaScript wrapper code for async methods
        ///
        /// This code should be executed after setting up the editor object
        /// to wrap the internal _*Start functions with Promise wrappers.
        #trait_vis const JS_ASYNC_WRAPPERS: &str = #js_wrappers_code;

        /// List of all JavaScript method names from the API trait
        ///
        /// Use this to verify that all methods have corresponding QuickJS bindings.
        #trait_vis const #methods_const_name: &[&str] = &[#(#js_method_names),*];
    };

    TokenStream::from(expanded)
}

/// Marker attribute for API method customization
///
/// Usage:
/// - `#[api(skip)]` - Don't expose this method to JS
/// - `#[api(js_name = "customName")]` - Use a custom JS method name
#[proc_macro_attribute]
pub fn api(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // This is a marker attribute, just pass through the item
    item
}
