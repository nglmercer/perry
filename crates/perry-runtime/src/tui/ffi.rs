//! C ABI surface called by perry-codegen.

use std::sync::Mutex;
use std::sync::OnceLock;

use crate::string::StringHeader;
use crate::value::js_nanbox_pointer;

use super::cell::Grid;
use super::color::Color;
use super::render;
use super::tree::{box_add_child, paint, register, Node};

/// Singleton grid — sized to the current terminal at first render.
static GRID: OnceLock<Mutex<Grid>> = OnceLock::new();

fn grid() -> &'static Mutex<Grid> {
    GRID.get_or_init(|| {
        let (w, h) = current_term_size();
        Mutex::new(Grid::new(w, h))
    })
}

/// Read the current terminal size via TIOCGWINSZ. Falls back to 80x24
/// when stdout isn't a TTY.
fn current_term_size() -> (u16, u16) {
    #[cfg(unix)]
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
            return (ws.ws_col, ws.ws_row);
        }
    }
    (80, 24)
}

// ---------------------------------------------------------------------------
// Widget factories
// ---------------------------------------------------------------------------

/// `Text(content)` — single-line text widget. Returns a NaN-boxed
/// POINTER handle.
#[no_mangle]
pub extern "C" fn js_perry_tui_text(content_ptr: *const StringHeader) -> f64 {
    let content = unsafe { read_string(content_ptr) };
    let h = register(Node::Text {
        content,
        fg: Color::Default,
        bg: Color::Default,
        style: super::cell::Style::default(),
    });
    js_nanbox_pointer(h)
}

/// `Box()` — empty container. Children are added via
/// `js_perry_tui_box_add_child`.
#[no_mangle]
pub extern "C" fn js_perry_tui_box() -> f64 {
    let h = register(Node::Box {
        children: Vec::new(),
        fg: Color::Default,
        bg: Color::Default,
    });
    js_nanbox_pointer(h)
}

/// Append a child to a Box. Both args are unboxed POINTER handles.
#[no_mangle]
pub extern "C" fn js_perry_tui_box_add_child(parent: i64, child: i64) -> f64 {
    box_add_child(parent, child);
    f64::from_bits(0x7FFC_0000_0000_0001) // TAG_UNDEFINED
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// `render(root)` — paint one frame.
#[no_mangle]
pub extern "C" fn js_perry_tui_render(root: i64) -> f64 {
    let (w, h) = current_term_size();
    let mut g = grid().lock().unwrap();
    g.resize(w, h);
    g.clear_back();
    let _ = paint(&mut g, root, 0, 0);
    render::flush(&mut g);
    f64::from_bits(0x7FFC_0000_0000_0001)
}

/// Initialize the renderer — clear screen and home the cursor.
#[no_mangle]
pub extern "C" fn js_perry_tui_enter() -> f64 {
    render::enter();
    f64::from_bits(0x7FFC_0000_0000_0001)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

unsafe fn read_string(ptr: *const StringHeader) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let len = (*ptr).byte_len as usize;
    let data = (ptr as *const u8).add(std::mem::size_of::<StringHeader>());
    let slice = std::slice::from_raw_parts(data, len);
    String::from_utf8_lossy(slice).into_owned()
}
