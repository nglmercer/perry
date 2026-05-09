//! CommonJS-to-ESM source-level transformation for `compilePackages`.
//!
//! Closes the React-class blocker for issue #348 (ink-as-compilePackages).
//!
//! React 18 ships as CommonJS — `node_modules/react/index.js` does
//! `module.exports = require('./cjs/react.production.min.js')`, and the
//! actual implementation file uses `exports.useState = function() {...}`
//! patterns. Perry's native pipeline is ESM-only — `module`/`require` lower
//! to bare-identifier-zero, so the entire react module compiles to a no-op
//! and every downstream `import { useState } from "react"` link-fails with
//! `Undefined symbols: _perry_fn_node_modules_react_index_js__useState`.
//!
//! This module detects CJS at module-read time and rewrites the source to
//! ESM-shaped code before SWC parses it. The wrap pattern (modeled after
//! `perry-jsruntime/src/modules.rs:481` which already does this for the V8
//! fallback) is:
//!
//!   1. Hoist every `require('X')` call as `import _req_N from 'X';`.
//!   2. Wrap the CJS body in an IIFE that defines `module = { exports: {} }`,
//!      a synchronous `require(specifier)` that dispatches to the hoisted
//!      `_req_N` bindings, runs the original code, and returns
//!      `module.exports`. The IIFE result is bound to `_cjs`.
//!   3. Emit `export default _cjs;` plus `export const X = _cjs.X;` for each
//!      detected named export.
//!
//! Two named-export sources are unioned:
//!
//!   - `exports.X = ...` patterns *in this file* (regex; the existing
//!     jsruntime heuristic).
//!   - For "trivial re-export wrappers" (`module.exports = require('./X')`,
//!     optionally inside a `process.env.NODE_ENV` conditional), the
//!     `exports.X = ...` patterns of the recursively-required *target* file.
//!     Without this, react/index.js — whose only meaningful statements are
//!     two conditional `module.exports = require(...)` calls — produces zero
//!     named exports of its own and the link still fails. The recursion
//!     follows up to a small depth (2 levels) to handle one level of env
//!     switching; deeper indirection is rare and gets the no-op fallback.

use std::path::Path;

/// Heuristic CJS detection. Same shape as
/// `perry-jsruntime/src/modules.rs::is_commonjs`. False negatives are
/// acceptable (the file just falls through to the existing ESM-only
/// pipeline); false positives on a real ESM file would be more painful but
/// require a file that uses neither `module.exports` nor `exports.` nor
/// `require(` — i.e., an ESM file that *also* contains those tokens. Real
/// hybrid cases are rare and would need a `"type": "module"` package.json
/// override, which is the next refinement if this trips a real package.
pub(super) fn is_commonjs(source: &str) -> bool {
    source.contains("module.exports")
        || source.contains("exports.")
        || (source.contains("require(") && !source.contains("import "))
}

/// Wrap CJS source as ESM. `source_path` is the absolute path of the file
/// being wrapped — used to resolve `require('./relative')` targets when
/// peeking at re-export wrappers' transitive named exports.
pub(super) fn wrap_commonjs(source: &str, source_path: &Path) -> String {
    let require_specs = extract_require_specifiers(source);

    let imports = require_specs
        .iter()
        .enumerate()
        .map(|(i, spec)| format!("import _req_{} from '{}';", i, spec))
        .collect::<Vec<_>>()
        .join("\n");

    let require_cases = require_specs
        .iter()
        .enumerate()
        .map(|(i, spec)| format!("        if (specifier === '{}') return _req_{};", spec, i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut named_exports = extract_exports_from_source(source);

    // For trivial re-export wrappers (`module.exports = require('./X')`),
    // recursively pull in the target's named exports. Without this,
    // react/index.js — which has zero `exports.X =` patterns of its own —
    // produces zero named exports and downstream `import { useState } from
    // "react"` link-fails.
    let parent = source_path.parent();
    if let Some(parent) = parent {
        for spec in &require_specs {
            if !spec.starts_with("./") && !spec.starts_with("../") {
                continue;
            }
            let target = parent.join(spec);
            if let Ok(target_source) = std::fs::read_to_string(&target) {
                for name in extract_exports_from_source(&target_source) {
                    if !named_exports.contains(&name) {
                        named_exports.push(name);
                    }
                }
            }
        }
    }

    let named_export_decls = if named_exports.is_empty() {
        String::new()
    } else {
        named_exports
            .iter()
            .map(|n| format!("export const {} = _cjs.{};", n, n))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"{imports}
const _cjs = (function() {{
    const module = {{ exports: {{}} }};
    const exports = module.exports;
    function require(specifier) {{
{require_cases}
        throw new Error('require() is not supported: ' + specifier);
    }}

    {source}

    return module.exports;
}})();

export default _cjs;
{named_export_decls}
"#
    )
}

/// Extract `require('X')` / `require("X")` specifiers, preserving order and
/// deduping. Only matches static string literal arguments — dynamic
/// `require(someVar)` is unrepresentable as ESM and the bound `require`
/// inside the IIFE will throw at runtime if hit.
fn extract_require_specifiers(source: &str) -> Vec<String> {
    let re = regex::Regex::new(r#"require\s*\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap();
    let mut specs = Vec::new();
    for cap in re.captures_iter(source) {
        if let Some(m) = cap.get(1) {
            let s = m.as_str().to_string();
            if !specs.contains(&s) {
                specs.push(s);
            }
        }
    }
    specs
}

/// Extract named-export patterns from CJS source. Three shapes are matched:
///
///   1. `exports.X = ...` and `module.exports.X = ...` — the canonical CJS
///      named-export form. Skips `__esModule` (the interop marker injected
///      by Babel/TypeScript that consumers use to detect "this is a CJS
///      module pretending to be ESM" — we don't want to re-export a boolean
///      as if it were a named binding).
///   2. `module.exports = { X, Y, fn: someFn }` — object-literal assignment
///      to `module.exports`. Issue #624: the synthetic-package shape that
///      hand-written CJS code typically uses (and that React's transpiled
///      output occasionally falls back to) was unsupported, so the consumer
///      `import { X } from "pkg"` link-failed because no named export was
///      ever extracted.
fn extract_exports_from_source(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let push_unique = |names: &mut Vec<String>, name: &str| {
        if name == "__esModule" {
            return;
        }
        let owned = name.to_string();
        if !names.contains(&owned) {
            names.push(owned);
        }
    };

    // Shape 1: `exports.X = ...` / `module.exports.X = ...`
    let dot_re =
        regex::Regex::new(r"(?:^|[^A-Za-z0-9_$])(?:module\.)?exports\.([A-Za-z_$][A-Za-z0-9_$]*)\s*=")
            .unwrap();
    for cap in dot_re.captures_iter(source) {
        if let Some(m) = cap.get(1) {
            push_unique(&mut names, m.as_str());
        }
    }

    // Shape 2: `module.exports = { ... }` — extract every key from the
    // object literal body. Brace-balanced scan because the body may contain
    // nested braces (`module.exports = { fn: function() {} }`). Two key
    // forms are recognized:
    //   - `name` (shorthand: `{ createContext }` ≡ `{ createContext: createContext }`)
    //   - `name: <expr>` (explicit: `{ createContext: createContext }` or `{ name: function() {} }`)
    // String-keyed entries (`"name": …`) and computed-key entries
    // (`[expr]: …`) are intentionally skipped — those don't surface as ESM
    // named exports anyway.
    let bytes = source.as_bytes();
    let mut search_from = 0usize;
    while let Some(idx) = source[search_from..].find("module.exports") {
        let abs = search_from + idx;
        // Skip past `module.exports`
        let mut p = abs + "module.exports".len();
        // Skip whitespace
        while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t' || bytes[p] == b'\n') {
            p += 1;
        }
        // Must be `=` (not `.`, `==`, etc.)
        if p >= bytes.len() || bytes[p] != b'=' {
            search_from = abs + 1;
            continue;
        }
        // Reject `==` / `===`
        if p + 1 < bytes.len() && bytes[p + 1] == b'=' {
            search_from = abs + 1;
            continue;
        }
        p += 1;
        // Skip whitespace
        while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t' || bytes[p] == b'\n') {
            p += 1;
        }
        // Must be `{`
        if p >= bytes.len() || bytes[p] != b'{' {
            search_from = abs + 1;
            continue;
        }
        // Brace-balanced scan to find the matching close.
        let body_start = p + 1;
        let mut depth: i32 = 1;
        let mut q = body_start;
        while q < bytes.len() && depth > 0 {
            match bytes[q] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            q += 1;
        }
        if depth != 0 {
            // Unbalanced — bail out, advance and continue scanning.
            search_from = abs + 1;
            continue;
        }
        let body_end = q - 1; // points at the closing `}`
        let body = &source[body_start..body_end];
        extract_object_literal_keys(body, &mut |name| push_unique(&mut names, name));
        search_from = q;
    }

    names
}

/// Extract top-level keys from an object-literal body (the text between
/// `{` and `}`, exclusive). Skips nested braces / brackets / parens so
/// `fn: function() { return 1; }` doesn't pull `return` as a key. Calls
/// `out` with each shorthand or `name:` key encountered at depth 0.
fn extract_object_literal_keys(body: &str, out: &mut dyn FnMut(&str)) {
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut at_entry_start = true;
    let mut depth: i32 = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'{' | b'[' | b'(' => {
                depth += 1;
                at_entry_start = false;
                i += 1;
            }
            b'}' | b']' | b')' => {
                depth -= 1;
                i += 1;
            }
            b',' if depth == 0 => {
                at_entry_start = true;
                i += 1;
            }
            b' ' | b'\t' | b'\n' | b'\r' => {
                i += 1;
            }
            _ if depth == 0 && at_entry_start => {
                // Try to read an identifier at the start of an entry.
                if (b as char).is_ascii_alphabetic() || b == b'_' || b == b'$' {
                    let start = i;
                    while i < bytes.len() {
                        let c = bytes[i];
                        if (c as char).is_ascii_alphanumeric() || c == b'_' || c == b'$' {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    let name = &body[start..i];
                    // Skip whitespace after the name.
                    let mut j = i;
                    while j < bytes.len()
                        && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n')
                    {
                        j += 1;
                    }
                    // Accept shorthand (`,` / end-of-body) or explicit key (`:`).
                    if j == bytes.len() || bytes[j] == b',' || bytes[j] == b':' {
                        out(name);
                    }
                    at_entry_start = false;
                } else {
                    // Non-identifier at entry start (e.g. `"key":` string,
                    // `[expr]:` computed, `...spread`) — skip; not an ESM
                    // exportable name.
                    at_entry_start = false;
                    i += 1;
                }
            }
            _ => {
                at_entry_start = false;
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_module_exports_assignment() {
        assert!(is_commonjs("module.exports = function() {};"));
    }

    #[test]
    fn detects_exports_dot_pattern() {
        assert!(is_commonjs("exports.foo = 1;"));
    }

    #[test]
    fn detects_require_without_import() {
        assert!(is_commonjs("var x = require('foo');"));
    }

    #[test]
    fn does_not_detect_pure_esm() {
        assert!(!is_commonjs("import x from 'foo'; export const y = 1;"));
    }

    #[test]
    fn extracts_named_exports() {
        let src = "exports.foo = 1; exports.bar = function() {}; exports.__esModule = true;";
        let names = extract_exports_from_source(src);
        assert_eq!(names, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn extracts_module_exports_object_literal_shorthand() {
        // Issue #624: `module.exports = { createContext }`
        let src = "function createContext(v){return v;}\nmodule.exports = { createContext };";
        let names = extract_exports_from_source(src);
        assert_eq!(names, vec!["createContext".to_string()]);
    }

    #[test]
    fn extracts_module_exports_object_literal_explicit() {
        // `module.exports = { foo: foo, bar: function(){} }`
        let src = "module.exports = { foo: foo, bar: function(){} };";
        let names = extract_exports_from_source(src);
        assert_eq!(names, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn extracts_module_exports_dot_form() {
        // `module.exports.foo = ...`
        let src = "module.exports.foo = 1; module.exports.bar = 2;";
        let names = extract_exports_from_source(src);
        assert_eq!(names, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn extracts_unions_dot_and_object_literal_forms() {
        let src = "exports.a = 1; module.exports = { b, c };";
        let names = extract_exports_from_source(src);
        assert_eq!(
            names,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn extracts_require_specifiers_dedup() {
        let src = r#"var a = require('./a'); var b = require("./b"); var c = require('./a');"#;
        let specs = extract_require_specifiers(src);
        assert_eq!(specs, vec!["./a".to_string(), "./b".to_string()]);
    }

    #[test]
    fn wraps_simple_cjs_as_esm() {
        let src = "exports.foo = 42;";
        let wrapped = wrap_commonjs(src, &PathBuf::from("/tmp/test.js"));
        assert!(wrapped.contains("export default _cjs;"));
        assert!(wrapped.contains("export const foo = _cjs.foo;"));
        assert!(wrapped.contains("const _cjs = (function()"));
    }

    #[test]
    fn wrap_hoists_require_as_import() {
        let src = "var dep = require('./dep'); module.exports = dep.value;";
        let wrapped = wrap_commonjs(src, &PathBuf::from("/tmp/test.js"));
        assert!(wrapped.contains("import _req_0 from './dep';"));
        assert!(wrapped.contains("if (specifier === './dep') return _req_0;"));
    }
}
