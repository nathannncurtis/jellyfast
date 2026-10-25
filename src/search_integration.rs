use crate::*;

pub fn open_searx_dialog() {
    crate::platform::appkit::open_searx_prompt();
}

// The NSAlert body lives in platform::appkit next to the other AppKit code.
