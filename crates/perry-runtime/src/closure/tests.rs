use super::*;

extern "C" fn test_closure_func(closure: *const ClosureHeader) -> f64 {
    unsafe {
        let captured = js_closure_get_capture_f64(closure, 0);
        captured * 2.0
    }
}

#[test]
fn test_closure_basic() {
    let closure = js_closure_alloc(test_closure_func as *const u8, 1);
    js_closure_set_capture_f64(closure, 0, 21.0);
    let result = js_closure_call0(closure);
    assert_eq!(result, 42.0);
}

// #5437 (Next.js W6) regression: a module-default-export WRAPPER closure whose
// body returns `module.exports` must be recognized after registration, and a
// property read off the captured wrapper must resolve against the exports
// object (not the closure itself). Mirrors `new uw.SharedCacheControls` where
// `uw` is a captured wrapper closure for the CJS shared-cache-controls module.
extern "C" fn test_wrapper_returns_capture0(closure: *const ClosureHeader) -> f64 {
    // Mimics `__perry_wrap_perry_fn_<src>__default` forwarding to the default
    // getter: returns the wrapped module.exports value (stored as capture 0).
    unsafe { js_closure_get_capture_f64(closure, 0) }
}

#[test]
fn module_default_wrapper_property_read_resolves_exports() {
    // Build a fake `module.exports` object carrying a `SharedCacheControls`
    // property (the number 7.0 stands in for the class ref).
    let exports_ptr = crate::object::js_object_alloc(0, 0);
    assert!(!exports_ptr.is_null());
    // `js_nanbox_pointer` returns the NaN-boxed POINTER value as raw f64 bits.
    let exports: f64 = crate::value::js_nanbox_pointer(exports_ptr as i64);
    let key_str = crate::string::js_string_from_bytes(b"SharedCacheControls".as_ptr(), 19);
    crate::object::js_object_set_field_by_name(exports_ptr, key_str, 7.0);

    // A wrapper closure that returns `module.exports` (the W6 shape: the default
    // getter yields this wrapper rather than the exports object).
    let wrapper = js_closure_alloc(test_wrapper_returns_capture0 as *const u8, 1);
    js_closure_set_capture_f64(wrapper, 0, exports);
    let wrapper_value: f64 = crate::value::js_nanbox_pointer(wrapper as i64);

    // Before registration the wrapper is not recognized and a property read off
    // it (as a function value) is `undefined` — the pre-fix W6 failure.
    assert!(!is_module_default_wrapper(wrapper as usize));
    let pre = crate::object::js_object_get_field_by_name(
        wrapper as *const crate::object::ObjectHeader,
        key_str,
    );
    assert!(
        pre.is_undefined(),
        "unregistered wrapper must not auto-resolve exports"
    );

    // Register the wrapper VALUE (the codegen-emitted registrar at the default
    // getter site). It must record the closure and return the value unchanged.
    let returned = js_register_module_default_wrapper_value(wrapper_value);
    assert_eq!(returned.to_bits(), wrapper_value.to_bits());
    assert!(is_module_default_wrapper(wrapper as usize));
    assert_eq!(
        module_default_wrapper_exports(wrapper as usize).map(|v| v.to_bits()),
        Some(exports.to_bits())
    );

    // After registration the property-read fallback calls the wrapper and reads
    // `SharedCacheControls` off `module.exports` → resolves to 7.0.
    let post = crate::object::js_object_get_field_by_name(
        wrapper as *const crate::object::ObjectHeader,
        key_str,
    );
    assert!(
        !post.is_undefined(),
        "registered wrapper must resolve exports"
    );
    assert_eq!(post.to_number(), 7.0);

    // A non-closure value passed to the registrar is returned untouched and not
    // registered (the common `let v = HONE_VERSION` getter case).
    let number_val = 123.0_f64;
    let n = js_register_module_default_wrapper_value(number_val);
    assert_eq!(n.to_bits(), number_val.to_bits());
}
