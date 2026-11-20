//! The system clipboard, read through Qt so pasting never blocks on an
//! external process (see `src/cpp/hermes_clipboard.h`). GUI thread only.

#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("hermes_clipboard.h");

        fn clipboard_formats() -> Vec<String>;
        fn clipboard_data(format: &str) -> Vec<u8>;
        fn clipboard_text() -> String;
    }
}

pub use ffi::{clipboard_data, clipboard_formats, clipboard_text};
