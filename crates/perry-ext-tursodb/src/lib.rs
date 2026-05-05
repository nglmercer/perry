//! Native bindings for Tursodb (closes #424).
//!
//! [Tursodb](https://github.com/tursodatabase) is a pure-Rust
//! SQLite-compatible database engine with extras (MVCC,
//! encryption, custom VFS). The wrapper exposes a TypeScript
//! surface modeled on `better-sqlite3`'s async equivalents
//! (since turso's API is async-first).
//!
//! # Status (v0.5.x — first cut)
//!
//! Minimum-viable port: `open` / `exec` / `close`. Full parity
//! with `better-sqlite3` (prepare / run / get / all / pragma)
//! lands in followups — needs JsValue object construction across
//! the spawn_blocking boundary, which is straightforward to add
//! incrementally without changing the FFI signatures here.
//!
//! # Recipe
//!
//! Same as every other async wrapper: register a handle holding
//! the `turso::Connection`; method calls take the handle plus
//! params, spawn blocking onto perry-ffi's tokio runtime, and
//! resolve a `JsPromise` from inside the closure. `tokio::runtime
//! ::Handle::current().block_on(async { ... })` bridges turso's
//! async API to the synchronous `spawn_blocking` closure body.

use perry_ffi::{
    drop_handle, get_handle, read_string, register_handle, spawn_blocking, with_handle, Handle,
    JsPromise, JsString, JsValue, Promise, StringHeader,
};
use turso::{Builder, Connection};

/// Wrapper struct so the registry's downcast is uniquely typed.
pub struct TursoConn {
    pub conn: Connection,
}

unsafe fn read_str(ptr: *const StringHeader) -> Option<String> {
    let handle = JsString::from_raw(ptr as *mut StringHeader);
    read_string(handle).map(String::from)
}

/// `tursodb.open(filename) -> Promise<Handle>` — open a database
/// and return a connection handle. `:memory:` for an in-memory
/// database, otherwise a filesystem path.
///
/// # Safety
///
/// `filename_ptr` must be null or a Perry-runtime `StringHeader`.
#[no_mangle]
pub unsafe extern "C" fn js_turso_open(filename_ptr: *const StringHeader) -> *mut Promise {
    let promise = JsPromise::new();
    let raw = promise.as_raw();
    let path = read_str(filename_ptr).unwrap_or_else(|| ":memory:".to_string());

    spawn_blocking(move || {
        let result = tokio::runtime::Handle::current().block_on(async move {
            let db = Builder::new_local(&path).build().await?;
            let conn = db.connect()?;
            Ok::<Connection, turso::Error>(conn)
        });
        match result {
            Ok(conn) => {
                let handle = register_handle(TursoConn { conn });
                // Handles are i64; ABI for FFI booleans is f64 too,
                // so we encode the handle as a number value. The
                // TS-side wrapper unboxes and stores it.
                promise.resolve(JsValue::from_number(handle as f64));
            }
            Err(e) => promise.reject_string(&format!("tursodb open: {}", e)),
        }
    });
    raw
}

/// `tursodb.exec(handle, sql) -> Promise<number>` — execute a
/// non-query statement (or batch). Resolves with rows affected.
///
/// # Safety
///
/// `sql_ptr` must be null or a Perry-runtime `StringHeader`.
#[no_mangle]
pub unsafe extern "C" fn js_turso_exec(
    db_handle: Handle,
    sql_ptr: *const StringHeader,
) -> *mut Promise {
    let promise = JsPromise::new();
    let raw = promise.as_raw();
    let Some(sql) = read_str(sql_ptr) else {
        promise.reject_string("Invalid SQL string");
        return raw;
    };

    spawn_blocking(move || {
        let result = with_handle::<TursoConn, _, _>(db_handle, |h| {
            tokio::runtime::Handle::current().block_on(async {
                h.conn.execute(&sql, ()).await
            })
        });
        match result {
            Some(Ok(rows_affected)) => {
                promise.resolve(JsValue::from_number(rows_affected as f64));
            }
            Some(Err(e)) => promise.reject_string(&format!("tursodb exec: {}", e)),
            None => promise.reject_string("tursodb: invalid handle"),
        }
    });
    raw
}

/// `tursodb.execBatch(handle, sql) -> Promise<void>` — execute
/// multiple statements separated by `;`.
///
/// # Safety
///
/// `sql_ptr` must be null or a Perry-runtime `StringHeader`.
#[no_mangle]
pub unsafe extern "C" fn js_turso_exec_batch(
    db_handle: Handle,
    sql_ptr: *const StringHeader,
) -> *mut Promise {
    let promise = JsPromise::new();
    let raw = promise.as_raw();
    let Some(sql) = read_str(sql_ptr) else {
        promise.reject_string("Invalid SQL string");
        return raw;
    };

    spawn_blocking(move || {
        let result = with_handle::<TursoConn, _, _>(db_handle, |h| {
            tokio::runtime::Handle::current().block_on(async {
                h.conn.execute_batch(&sql).await
            })
        });
        match result {
            Some(Ok(())) => promise.resolve_undefined(),
            Some(Err(e)) => promise.reject_string(&format!("tursodb execBatch: {}", e)),
            None => promise.reject_string("tursodb: invalid handle"),
        }
    });
    raw
}

/// `tursodb.lastInsertRowid(handle) -> number` — synchronous
/// accessor for the last `INSERT`'s row id. The underlying
/// turso method is sync, no Promise wrapping needed.
#[no_mangle]
pub extern "C" fn js_turso_last_insert_rowid(db_handle: Handle) -> f64 {
    if let Some(h) = get_handle::<TursoConn>(db_handle) {
        h.conn.last_insert_rowid() as f64
    } else {
        0.0
    }
}

/// `tursodb.isAutocommit(handle) -> boolean (1.0 / 0.0)`.
#[no_mangle]
pub extern "C" fn js_turso_is_autocommit(db_handle: Handle) -> f64 {
    if let Some(h) = get_handle::<TursoConn>(db_handle) {
        match h.conn.is_autocommit() {
            Ok(true) => 1.0,
            _ => 0.0,
        }
    } else {
        0.0
    }
}

/// `tursodb.close(handle) -> 1.0 / 0.0` — drop the connection.
#[no_mangle]
pub extern "C" fn js_turso_close(db_handle: Handle) -> f64 {
    if drop_handle(db_handle) {
        1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    // Unit tests for tursodb need a tokio runtime — perry-ffi's
    // spawn_blocking pumps the global runtime which is owned by
    // perry-stdlib's async_bridge. That static isn't initialized
    // in standalone unit tests (no perry-stdlib link). End-to-end
    // smoke testing happens via the TS integration in release
    // mode, where the full link surface is in place.
    //
    // The pure-Rust correctness of the underlying turso crate is
    // covered by upstream tests; our wrapper just plumbs args
    // and resolutions, exercised end-to-end.
}
