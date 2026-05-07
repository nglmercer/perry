//! Issue #535 — `perry/ui` `state<T>` desugar pass for non-HarmonyOS targets.
//!
//! Pre-fix: `state<T>(initial)` had no codegen lowering on macOS / iOS / Android
//! / GTK4 / Windows; the strict-API gate raised a hard compile error
//! ("'state' is not a known function"). Only `--target harmonyos` worked,
//! because `perry-codegen-arkts` runs its own harvest that rewrites these
//! shapes into `setText` calls before LLVM ever sees them. The d.ts comment
//! at `types/perry/ui/index.d.ts:263-265` documented this as "deferred to
//! v6.5", which blocked any multi-screen native app per #535.
//!
//! What this pass does (mirrors arkts's collect_state_bindings +
//! rewrite_state_calls_in_stmts at `crates/perry-codegen-arkts/src/lib.rs:1233`
//! and `:3725`, but emits target-agnostic HIR so the LLVM backend can
//! lower it):
//!
//! 1. Walk `module.init` for `let x = state(initial)` declarations. Assign
//!    each a synthetic id (`__state_<N>`).
//!
//! 2. Replace the declaration's initializer with `Expr::Undefined` so the
//!    local survives any not-yet-rewritten escape (function-arg pass, array
//!    push, etc.) without crashing codegen, then prepend a synthetic
//!    `__state_init("synth_id", initial)` call to the init list so the
//!    runtime registry has the initial value before any reader fires.
//!
//! 3. Rewrite every `x.set(v)` / `x.get()` / `x.value` / `x.text()` use
//!    across `module.init`, every function body, and every nested closure
//!    body to the synth-id-keyed runtime equivalent:
//!
//!    - `x.set(v)` → `__state_set("synth_id", v)` — runtime updates the
//!      registry slot AND fires `perry_arkts_set_text` so any text widget
//!      registered under `synth_id` re-renders.
//!    - `x.get()` / `x.value` → `__state_get("synth_id")` — reads from the
//!      registry. Returns `undefined` for unknown ids (matches the JS
//!      `State<T>` semantics for an uninitialized cell).
//!    - `x.text()` → `Text("<initial-as-string>", "synth_id")`. The 2-arg
//!      `Text` form already routes through `perry_ui_text_create_with_id`
//!      which calls `perry_arkts_register_text_id`, so the widget joins
//!      the setText dispatch table automatically.
//!
//! Limitations (v0.5.617):
//!
//! - Only matches `state(...)` declared via `let`/`const` at the top level
//!   of `module.init`. Declarations inside function bodies aren't tracked
//!   yet; they would compile-error the same as today. Real-world apps
//!   (per #535's repro) use top-level state, so this is the right first cut.
//! - Only matches the canonical method-call shapes — `x.set(v)`, `x.get()`,
//!   `x.value`, `x.text()`. If a state escapes through a function arg /
//!   array / object property, the call site there has no `LocalGet(x)`
//!   anchor and the rewrite skips it. Today: that's a follow-up; the
//!   `let x = undefined` shim at least keeps the program compilable.
//! - `.text()` snapshots the initial value at compile time using the
//!   literal initializer. Computed-initial states get an empty initial
//!   string — the first `.set()` corrects it at runtime.
//!
//! HarmonyOS: this pass is gated OFF in `collect_modules.rs` so
//! `perry-codegen-arkts`'s harvest stays the source of truth there.

use perry_hir::walker::walk_expr_children_mut;
use perry_hir::{Expr, Module, Stmt};
use perry_types::LocalId;
use std::collections::HashMap;

/// One `state<T>` declaration the pass has decided to rewrite.
struct StateBinding {
    /// Synthetic id baked into all rewritten call sites. Format
    /// `__state_<N>` (zero-based, declaration order in `module.init`).
    /// Stable across re-runs because the iteration order is deterministic.
    synth_id: String,
    /// The original initial-value expression. Used by `.text()` rewrites
    /// to compute the literal string the bound `Text` widget displays
    /// before the first `.set()` call updates it.
    initial: Expr,
}

/// Run the desugar. No-op when the module has no `state<T>` declarations.
pub fn run(module: &mut Module) {
    let bindings = collect_state_bindings(&module.init);
    if bindings.is_empty() {
        return;
    }
    rewrite_init_decls(&mut module.init, &bindings);
    rewrite_stmts(&mut module.init, &bindings);
    for func in module.functions.iter_mut() {
        rewrite_stmts(&mut func.body, &bindings);
    }
}

/// Walk `module.init` for `let x = state(initial)` and assign each a
/// synth id. Mirrors `perry-codegen-arkts::collect_state_bindings`.
fn collect_state_bindings(init: &[Stmt]) -> HashMap<LocalId, StateBinding> {
    let mut map = HashMap::new();
    let mut counter: usize = 0;
    for stmt in init {
        if let Stmt::Let {
            id,
            init: Some(call_expr),
            ..
        } = stmt
        {
            if let Expr::NativeMethodCall {
                module,
                method,
                object: None,
                args,
                ..
            } = call_expr
            {
                if module == "perry/ui" && method == "state" && args.len() == 1 {
                    let synth_id = format!("__state_{}", counter);
                    counter += 1;
                    map.insert(
                        *id,
                        StateBinding {
                            synth_id,
                            initial: args[0].clone(),
                        },
                    );
                }
            }
        }
    }
    map
}

/// Replace every matched `let x = state(initial)` statement with a pair:
/// (1) `let x = undefined` to keep the LocalId alive for any not-rewritten
/// escape, and (2) a synthetic `__state_init("synth_id", initial)` call
/// that primes the runtime registry. The init must run BEFORE any
/// rewritten reader (`__state_get(...)`) downstream, so we place it
/// immediately after the declaration in source order.
fn rewrite_init_decls(init: &mut Vec<Stmt>, bindings: &HashMap<LocalId, StateBinding>) {
    let mut new_stmts: Vec<Stmt> = Vec::with_capacity(init.len() + bindings.len());
    for stmt in init.drain(..) {
        match stmt {
            Stmt::Let {
                id,
                ref name,
                ref ty,
                mutable,
                init: Some(_),
            } if bindings.contains_key(&id) => {
                let binding = &bindings[&id];
                new_stmts.push(Stmt::Let {
                    id,
                    name: name.clone(),
                    ty: ty.clone(),
                    mutable,
                    init: Some(Expr::Undefined),
                });
                new_stmts.push(Stmt::Expr(state_init_call(
                    &binding.synth_id,
                    binding.initial.clone(),
                )));
            }
            other => new_stmts.push(other),
        }
    }
    *init = new_stmts;
}

/// Recursively rewrite every `Stmt` in `stmts`. Descends into block-shaped
/// children (if/while/for/etc.) so closures buried in `Button(label, () =>
/// state.set(...))` are visited.
fn rewrite_stmts(stmts: &mut Vec<Stmt>, bindings: &HashMap<LocalId, StateBinding>) {
    for stmt in stmts.iter_mut() {
        rewrite_stmt(stmt, bindings);
    }
}

fn rewrite_stmt(stmt: &mut Stmt, bindings: &HashMap<LocalId, StateBinding>) {
    match stmt {
        Stmt::Expr(e) => rewrite_expr(e, bindings),
        Stmt::Return(Some(e)) => rewrite_expr(e, bindings),
        Stmt::Throw(e) => rewrite_expr(e, bindings),
        Stmt::Let { init: Some(e), .. } => rewrite_expr(e, bindings),
        Stmt::If {
            condition,
            then_branch,
            else_branch,
        } => {
            rewrite_expr(condition, bindings);
            rewrite_stmts(then_branch, bindings);
            if let Some(eb) = else_branch {
                rewrite_stmts(eb, bindings);
            }
        }
        Stmt::While { condition, body } | Stmt::DoWhile { body, condition } => {
            rewrite_expr(condition, bindings);
            rewrite_stmts(body, bindings);
        }
        Stmt::For {
            init,
            condition,
            update,
            body,
        } => {
            if let Some(i) = init {
                rewrite_stmt(i.as_mut(), bindings);
            }
            if let Some(c) = condition {
                rewrite_expr(c, bindings);
            }
            if let Some(u) = update {
                rewrite_expr(u, bindings);
            }
            rewrite_stmts(body, bindings);
        }
        Stmt::Try {
            body,
            catch,
            finally,
        } => {
            rewrite_stmts(body, bindings);
            if let Some(c) = catch {
                rewrite_stmts(&mut c.body, bindings);
            }
            if let Some(f) = finally {
                rewrite_stmts(f, bindings);
            }
        }
        Stmt::Switch { discriminant, cases } => {
            rewrite_expr(discriminant, bindings);
            for case in cases {
                if let Some(t) = &mut case.test {
                    rewrite_expr(t, bindings);
                }
                rewrite_stmts(&mut case.body, bindings);
            }
        }
        Stmt::Labeled { body, .. } => rewrite_stmt(body.as_mut(), bindings),
        _ => {}
    }
}

/// Detect the four state-method shapes most-specific first, then fall
/// through to a generic recursion over child exprs. The recursion uses
/// `walk_expr_children_mut` plus an explicit closure-body descent
/// (`walk_expr_children_mut` intentionally doesn't enter Closure bodies —
/// see `crates/perry-hir/src/walker.rs:23-25`).
///
/// Children are rewritten BEFORE the outer match. This matters for
/// `state.set(state.get() + 1)` and similar nested patterns: without
/// the inner-first walk, the outer `.set` rewrite would clone the arg
/// expression containing the un-rewritten inner `.get()`, leaving it
/// as a plain `LocalGet(state).get()` on a holder that's now `undefined`.
fn rewrite_expr(e: &mut Expr, bindings: &HashMap<LocalId, StateBinding>) {
    walk_expr_children_mut(e, &mut |child| rewrite_expr(child, bindings));

    if let Expr::Closure { body, .. } = e {
        rewrite_stmts(body, bindings);
    }

    if let Some(replacement) = try_rewrite_state_access(e, bindings) {
        *e = replacement;
    }
}

/// Attempt to rewrite `e` if it matches a state access on a known
/// state-bound local. Returns `Some(new_expr)` for a match, `None`
/// otherwise. Does not recurse into children — the caller does that
/// after this returns `None`.
fn try_rewrite_state_access(
    e: &Expr,
    bindings: &HashMap<LocalId, StateBinding>,
) -> Option<Expr> {
    if let Expr::Call { callee, args, .. } = e {
        if let Expr::PropertyGet { object, property } = callee.as_ref() {
            if let Expr::LocalGet(state_id) = object.as_ref() {
                if let Some(binding) = bindings.get(state_id) {
                    return match property.as_str() {
                        "get" if args.is_empty() => Some(state_get_call(&binding.synth_id)),
                        "set" if args.len() == 1 => {
                            Some(state_set_call(&binding.synth_id, args[0].clone()))
                        }
                        "text" if args.is_empty() => Some(state_text_call(binding)),
                        _ => None,
                    };
                }
            }
        }
    }
    if let Expr::PropertyGet { object, property } = e {
        if property == "value" {
            if let Expr::LocalGet(state_id) = object.as_ref() {
                if let Some(binding) = bindings.get(state_id) {
                    return Some(state_get_call(&binding.synth_id));
                }
            }
        }
    }
    None
}

fn state_init_call(synth_id: &str, initial: Expr) -> Expr {
    Expr::NativeMethodCall {
        module: "perry/ui".to_string(),
        class_name: None,
        object: None,
        method: "__state_init".to_string(),
        args: vec![Expr::String(synth_id.to_string()), initial],
    }
}

fn state_get_call(synth_id: &str) -> Expr {
    Expr::NativeMethodCall {
        module: "perry/ui".to_string(),
        class_name: None,
        object: None,
        method: "__state_get".to_string(),
        args: vec![Expr::String(synth_id.to_string())],
    }
}

fn state_set_call(synth_id: &str, value: Expr) -> Expr {
    Expr::NativeMethodCall {
        module: "perry/ui".to_string(),
        class_name: None,
        object: None,
        method: "__state_set".to_string(),
        args: vec![Expr::String(synth_id.to_string()), value],
    }
}

fn state_text_call(binding: &StateBinding) -> Expr {
    let initial_str = match &binding.initial {
        Expr::String(s) => s.clone(),
        Expr::Number(n) => format_number(*n),
        Expr::Integer(n) => n.to_string(),
        Expr::Bool(b) => b.to_string(),
        _ => String::new(),
    };
    Expr::NativeMethodCall {
        module: "perry/ui".to_string(),
        class_name: None,
        object: None,
        method: "Text".to_string(),
        args: vec![
            Expr::String(initial_str),
            Expr::String(binding.synth_id.clone()),
        ],
    }
}

/// Render a numeric literal the way JS's `String(n)` would for typical
/// initials — integers without a decimal point, fractions with one. Avoids
/// pulling in a heavier formatter for the v0.5.617 first cut.
fn format_number(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}
