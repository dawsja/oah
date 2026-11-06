//! Raw declarations for the subset of `opentui.h` (C ABI version 2, OpenTUI v0.8.2)
//! that this crate wraps. Layouts mirror the header exactly; see
//! `packages/native/include/opentui.h` in the OpenTUI repository.
#![allow(non_camel_case_types, dead_code)]

use std::os::raw::c_void;

pub type ot_status = i32;
pub const OT_CONTEXT_ABI_VERSION: u32 = 2;

pub const OT_OK: ot_status = 0;
pub const OT_INVALID_ARGUMENT: ot_status = -1;
pub const OT_OUTPUT_BACKPRESSURE: ot_status = -11;
pub const OT_SESSION_CLOSED: ot_status = -12;
pub const OT_OUTPUT_BUSY: ot_status = -13;
pub const OT_UNSUPPORTED_RESOURCE: ot_status = -24;
pub const OT_INVALID_PHASE: ot_status = -19;

#[repr(C)]
pub struct ot_context {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ot_handle {
    pub context_id: u64,
    pub slot: u32,
    pub generation: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_context_options {
    pub struct_size: u32,
    pub abi_version: u32,
    pub flags: u32,
    pub object_capacity: u32,
    pub render_cells_max: u32,
    pub reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_context_error {
    pub struct_size: u32,
    pub abi_version: u32,
    pub status: ot_status,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_session_options {
    pub struct_size: u32,
    pub abi_version: u32,
    pub chunk_size: u32,
    pub span_capacity: u32,
    pub max_bytes: u64,
    pub control_capacity: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_session_renderer_options {
    pub struct_size: u32,
    pub abi_version: u32,
    pub width: u32,
    pub height: u32,
    pub remote: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_session_terminal_options {
    pub struct_size: u32,
    pub abi_version: u32,
    pub flags: u32,
    pub kitty_keyboard_flags: u32,
}

pub const OT_TERMINAL_ALTERNATE_SCREEN: u32 = 1;
pub const OT_TERMINAL_MOUSE: u32 = 2;
pub const OT_TERMINAL_MOUSE_MOVEMENT: u32 = 4;
pub const OT_TERMINAL_CLEAR_ON_CLOSE: u32 = 8;
pub const OT_TERMINAL_REPLY_SENTINEL: u32 = 16;
pub const OT_TERMINAL_SYNC_ALWAYS: u32 = 32;
pub const OT_TERMINAL_SYNC_NEVER: u32 = 64;
pub const OT_TERMINAL_HOST_MULTIPLEXER: u32 = 128;

pub const OT_TERMINAL_UNINITIALIZED: u32 = 0;
pub const OT_TERMINAL_SETTING_UP: u32 = 1;
pub const OT_TERMINAL_ACTIVE: u32 = 2;
pub const OT_TERMINAL_SUSPENDING: u32 = 3;
pub const OT_TERMINAL_SUSPENDED: u32 = 4;
pub const OT_TERMINAL_RESUMING: u32 = 5;
pub const OT_TERMINAL_CLOSING: u32 = 6;
pub const OT_TERMINAL_RESTORED: u32 = 7;
pub const OT_TERMINAL_FAILED: u32 = 8;
pub const OT_TERMINAL_CANCELLED: u32 = 9;

pub const OT_SESSION_OPEN: u32 = 0;
pub const OT_SESSION_CLOSING: u32 = 1;
pub const OT_SESSION_CLOSED_STATE: u32 = 2;
pub const OT_SESSION_FAILED: u32 = 3;
pub const OT_SESSION_CANCELLED_STATE: u32 = 4;

pub const OT_CONTROL_CAPABILITY_RESPONSE: u32 = 1;
pub const OT_CONTROL_TITLE: u32 = 2;
pub const OT_CONTROL_MOUSE: u32 = 3;
pub const OT_CONTROL_CURSOR: u32 = 8;
pub const OT_CONTROL_PROGRAM_STATUS: u32 = 11;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_session_control_options {
    pub struct_size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub argument: u32,
    pub reserved: u32,
}

pub const OT_CURSOR_POSITION: u32 = 1;
pub const OT_CURSOR_STYLE: u32 = 2;
pub const OT_CURSOR_BLINKING: u32 = 4;
pub const OT_CURSOR_COLOR: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_session_cursor_update {
    pub fields: u32,
    pub x: i32,
    pub y: i32,
    pub visible: u8,
    pub style: u8,
    pub blinking: u8,
    pub mouse_pointer: u8,
    pub color: [u16; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ot_session_capabilities {
    pub struct_size: u32,
    pub abi_version: u32,
    pub flags: u32,
    pub width_method: u32,
    pub multiplexer: u32,
    pub image_protocol: u32,
    pub osc52_support: u32,
    pub kitty_keyboard_flags: u32,
    pub term_name_len: u32,
    pub term_version_len: u32,
    pub term_from_xtversion: u32,
    pub reserved: u32,
    pub term_name: [u8; 64],
    pub term_version: [u8; 32],
}

pub const OT_PUMP_IDLE: u32 = 0;
pub const OT_PUMP_AGAIN: u32 = 1;
pub const OT_PUMP_OUTPUT_PENDING: u32 = 2;
pub const OT_PUMP_WAIT_UNTIL: u32 = 3;
pub const OT_PUMP_CLOSED: u32 = 4;

pub const OT_STDOUT_IDLE: u32 = 0;
pub const OT_STDOUT_IN_FLIGHT: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_session_pump_result {
    pub struct_size: u32,
    pub abi_version: u32,
    pub status: u32,
    pub reserved: u32,
    pub deadline_ns: u64,
}

pub const OT_RENDER_PRESENTED: u32 = 0;
pub const OT_RENDER_PENDING: u32 = 1;
pub const OT_RENDER_SKIPPED: u32 = 2;
pub const OT_RENDER_FAILED: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_session_renderer_state {
    pub struct_size: u32,
    pub abi_version: u32,
    pub width: u32,
    pub height: u32,
    pub frame_count: u64,
    pub frame_pending: u32,
    pub image_uploads_pending: u32,
    pub dropped_frames: u32,
    pub dropped_frame_bytes: u32,
}

// ---- input ---------------------------------------------------------------

pub const OT_INPUT_KEY: u8 = 1;
pub const OT_INPUT_MOUSE: u8 = 2;
pub const OT_INPUT_PASTE: u8 = 3;
pub const OT_INPUT_FOCUS: u8 = 4;
pub const OT_INPUT_REPLY: u8 = 5;

pub const OT_INPUT_KEY_PRESS: u8 = 1;
pub const OT_INPUT_KEY_REPEAT: u8 = 2;
pub const OT_INPUT_KEY_RELEASE: u8 = 3;
pub const OT_INPUT_MOUSE_DOWN: u8 = 1;
pub const OT_INPUT_MOUSE_UP: u8 = 2;
pub const OT_INPUT_MOUSE_MOVE: u8 = 3;
pub const OT_INPUT_MOUSE_DRAG: u8 = 4;
pub const OT_INPUT_MOUSE_SCROLL: u8 = 5;

pub const OT_INPUT_MOD_SHIFT: u8 = 1;
pub const OT_INPUT_MOD_ALT: u8 = 2;
pub const OT_INPUT_MOD_CTRL: u8 = 4;
pub const OT_INPUT_MOD_SUPER: u8 = 8;

pub const OT_INPUT_PASTE_START: u8 = 1;
pub const OT_INPUT_PASTE_END: u8 = 2;

pub const OT_INPUT_REPLY_CSI: u32 = 1;

pub const OT_INPUT_EXPECT_REPLIES: u32 = 1;
pub const OT_INPUT_EXPECT_KITTY_KEYBOARD: u32 = 2;
pub const OT_INPUT_EXPECT_CURSOR_POSITION: u32 = 4;
pub const OT_INPUT_EXPECT_SENTINEL: u32 = 8;

pub const OT_INPUT_EVENTS_MIN: u32 = 4;
pub const OT_INPUT_PAYLOAD_BYTES_MIN: u32 = 4384;
pub const OT_INPUT_TIMEOUT_NS: u64 = 20_000_000;

pub const OT_INPUT_KEY_ESCAPE: u32 = 27;
pub const OT_INPUT_KEY_ENTER: u32 = 13;
pub const OT_INPUT_KEY_TAB: u32 = 9;
pub const OT_INPUT_KEY_BACKSPACE: u32 = 127;
pub const OT_INPUT_KEY_INSERT: u32 = 57348;
pub const OT_INPUT_KEY_DELETE: u32 = 57349;
pub const OT_INPUT_KEY_LEFT: u32 = 57350;
pub const OT_INPUT_KEY_RIGHT: u32 = 57351;
pub const OT_INPUT_KEY_UP: u32 = 57352;
pub const OT_INPUT_KEY_DOWN: u32 = 57353;
pub const OT_INPUT_KEY_PAGE_UP: u32 = 57354;
pub const OT_INPUT_KEY_PAGE_DOWN: u32 = 57355;
pub const OT_INPUT_KEY_HOME: u32 = 57356;
pub const OT_INPUT_KEY_END: u32 = 57357;
pub const OT_INPUT_KEY_KP_ENTER: u32 = 57414;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ot_input_event {
    pub kind: u8,
    pub action: u8,
    pub modifiers: u8,
    pub flags: u8,
    pub code: u32,
    pub base_code: u32,
    pub x: u16,
    pub y: u16,
    pub raw_offset: u32,
    pub raw_len: u32,
    pub text_offset: u32,
    pub text_len: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_input_drain {
    pub struct_size: u32,
    pub abi_version: u32,
    pub consumed: u32,
    pub count: u32,
    pub payload_len: u32,
    pub discarded: u32,
    pub deadline_ns: u64,
}

// ---- buffers -------------------------------------------------------------

pub const OT_WIDTH_METHOD_WCWIDTH: u32 = 0;
pub const OT_WIDTH_METHOD_UNICODE: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_buffer_options {
    pub struct_size: u32,
    pub abi_version: u32,
    pub width: u32,
    pub height: u32,
    pub width_method: u32,
    pub flags: u32,
}

pub const OT_BUFFER_DRAW_CLEAR: u32 = 0;
pub const OT_BUFFER_DRAW_FILL: u32 = 1;
pub const OT_BUFFER_DRAW_TEXT: u32 = 2;
pub const OT_BUFFER_DRAW_CELL: u32 = 3;
pub const OT_BUFFER_DRAW_CELL_BLEND: u32 = 4;
pub const OT_BUFFER_DRAW_CHAR: u32 = 5;
pub const OT_BUFFER_DRAW_BOX: u32 = 6;
pub const OT_BUFFER_DRAW_HAS_BACKGROUND: u32 = 1;
pub const OT_BUFFER_DRAW_FIT_ELLIPSIS: u32 = 16;

pub const OT_BUFFER_STACK_PUSH_SCISSOR: u32 = 1;
pub const OT_BUFFER_STACK_POP_SCISSOR: u32 = 2;
pub const OT_BUFFER_STACK_CLEAR_SCISSORS: u32 = 3;

pub const OT_BORDER_LEFT: u32 = 1;
pub const OT_BORDER_BOTTOM: u32 = 2;
pub const OT_BORDER_RIGHT: u32 = 4;
pub const OT_BORDER_TOP: u32 = 8;
pub const OT_BORDER_ALL: u32 = 15;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_buffer_draw_header {
    pub struct_size: u32,
    pub abi_version: u32,
    pub operation: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_buffer_draw_clear {
    pub header: ot_buffer_draw_header,
    pub background: [u16; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_buffer_draw_fill {
    pub header: ot_buffer_draw_header,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub background: [u16; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_buffer_draw_text_record {
    pub header: ot_buffer_draw_header,
    pub x: i32,
    pub y: i32,
    pub attributes: u32,
    pub foreground: [u16; 4],
    pub background: [u16; 4],
    pub max_width: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_buffer_draw_cell {
    pub header: ot_buffer_draw_header,
    pub x: i32,
    pub y: i32,
    pub character: u32,
    pub attributes: u32,
    pub foreground: [u16; 4],
    pub background: [u16; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ot_buffer_draw_box {
    pub header: ot_buffer_draw_header,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub packed_options: u32,
    pub foreground: [u16; 4],
    pub background: [u16; 4],
    pub title_color: [u16; 4],
    pub border_chars: [u32; 11],
}

#[repr(C)]
pub struct ot_scene_frame_request {
    _opaque: [u8; 0],
}

pub type ot_output_write_callback =
    Option<unsafe extern "C" fn(user_data: *mut c_void, bytes: *const u8, byte_count: u32) -> i64>;

extern "C" {
    pub fn ot_context_abi_version() -> u32;
    pub fn ot_context_create(options: *const ot_context_options, out: *mut *mut ot_context) -> ot_status;
    pub fn ot_context_destroy(ctx: *mut ot_context) -> ot_status;
    pub fn ot_context_get_last_error(ctx: *mut ot_context, out: *mut ot_context_error) -> ot_status;

    pub fn ot_buffer_create(ctx: *mut ot_context, options: *const ot_buffer_options, out: *mut ot_handle) -> ot_status;
    pub fn ot_buffer_destroy(ctx: *mut ot_context, buffer: *const ot_handle) -> ot_status;
    pub fn ot_buffer_resize(ctx: *mut ot_context, buffer: *const ot_handle, width: u32, height: u32) -> ot_status;
    pub fn ot_buffer_draw(
        ctx: *mut ot_context,
        target: *const ot_handle,
        frame: *const ot_scene_frame_request,
        options: *const ot_buffer_draw_header,
        source: *const ot_handle,
        text: *const u8,
        text_len: u32,
        bottom_title: *const u8,
        bottom_title_len: u32,
    ) -> ot_status;
    pub fn ot_buffer_stack(
        ctx: *mut ot_context,
        target: *const ot_handle,
        frame: *const ot_scene_frame_request,
        operation: u32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        opacity: *const f32,
        out_opacity: *mut f32,
    ) -> ot_status;

    pub fn ot_session_create(ctx: *mut ot_context, options: *const ot_session_options, out: *mut ot_handle) -> ot_status;
    pub fn ot_session_attach_renderer(
        ctx: *mut ot_context,
        session: *const ot_handle,
        options: *const ot_session_renderer_options,
    ) -> ot_status;
    pub fn ot_session_setup_terminal(
        ctx: *mut ot_context,
        session: *const ot_handle,
        options: *const ot_session_terminal_options,
    ) -> ot_status;
    pub fn ot_session_suspend(ctx: *mut ot_context, session: *const ot_handle) -> ot_status;
    pub fn ot_session_resume(ctx: *mut ot_context, session: *const ot_handle) -> ot_status;
    pub fn ot_session_get_terminal_state(ctx: *mut ot_context, session: *const ot_handle, out: *mut u32) -> ot_status;
    pub fn ot_session_control(
        ctx: *mut ot_context,
        session: *const ot_handle,
        options: *const ot_session_control_options,
        bytes: *const u8,
        byte_count: u32,
    ) -> ot_status;
    pub fn ot_session_get_capabilities(
        ctx: *mut ot_context,
        session: *const ot_handle,
        out: *mut ot_session_capabilities,
    ) -> ot_status;
    pub fn ot_session_clipboard(
        ctx: *mut ot_context,
        session: *const ot_handle,
        target: u32,
        bytes: *const u8,
        byte_count: u32,
        out_written: *mut u32,
    ) -> ot_status;
    pub fn ot_session_notification(
        ctx: *mut ot_context,
        session: *const ot_handle,
        message: *const u8,
        message_len: u32,
        title: *const u8,
        title_len: u32,
        out_written: *mut u32,
    ) -> ot_status;
    pub fn ot_session_pump(
        ctx: *mut ot_context,
        session: *const ot_handle,
        now_ns: u64,
        work_budget: u32,
        out: *mut ot_session_pump_result,
    ) -> ot_status;
    pub fn ot_session_input_feed(
        ctx: *mut ot_context,
        session: *const ot_handle,
        bytes: *const u8,
        byte_count: u32,
        now_ns: u64,
        records: *mut ot_input_event,
        capacity: u32,
        payload: *mut u8,
        payload_capacity: u32,
        out_drain: *mut ot_input_drain,
    ) -> ot_status;
    pub fn ot_session_input_expect(ctx: *mut ot_context, session: *const ot_handle, flags: u32) -> ot_status;
    pub fn ot_session_render(ctx: *mut ot_context, session: *const ot_handle, force: u32, out: *mut u32) -> ot_status;
    pub fn ot_session_resize_renderer(ctx: *mut ot_context, session: *const ot_handle, width: u32, height: u32) -> ot_status;
    pub fn ot_session_draw_buffer(
        ctx: *mut ot_context,
        session: *const ot_handle,
        source: *const ot_handle,
        x: i32,
        y: i32,
    ) -> ot_status;
    pub fn ot_session_get_renderer_state(
        ctx: *mut ot_context,
        session: *const ot_handle,
        out: *mut ot_session_renderer_state,
    ) -> ot_status;
    pub fn ot_session_write(ctx: *mut ot_context, session: *const ot_handle, bytes: *const u8, byte_count: u32) -> ot_status;
    pub fn ot_session_drain_output(
        ctx: *mut ot_context,
        session: *const ot_handle,
        max_bytes: u32,
        out_written: *mut u32,
        user_data: *mut c_void,
        writer: ot_output_write_callback,
    ) -> ot_status;
    pub fn ot_session_drain_stdout(
        ctx: *mut ot_context,
        session: *const ot_handle,
        max_bytes: u32,
        out_written: *mut u32,
    ) -> ot_status;
    pub fn ot_session_close(ctx: *mut ot_context, session: *const ot_handle) -> ot_status;
    pub fn ot_session_drop_output(ctx: *mut ot_context, session: *const ot_handle) -> ot_status;
    pub fn ot_session_pump_exit(ctx: *mut ot_context, session: *const ot_handle, out_status: *mut u32) -> ot_status;
    pub fn ot_terminal_flush_input() -> ot_status;
    pub fn ot_session_cancel(ctx: *mut ot_context, session: *const ot_handle) -> ot_status;
    pub fn ot_session_get_state(ctx: *mut ot_context, session: *const ot_handle, out: *mut u32) -> ot_status;
    pub fn ot_session_destroy(ctx: *mut ot_context, session: *const ot_handle) -> ot_status;
}

#[cfg(test)]
mod layout {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn record_sizes_match_header() {
        assert_eq!(size_of::<ot_handle>(), 16);
        assert_eq!(size_of::<ot_context_options>(), 32);
        assert_eq!(size_of::<ot_session_options>(), 32);
        assert_eq!(size_of::<ot_input_event>(), 32);
        assert_eq!(size_of::<ot_input_drain>(), 32);
        assert_eq!(size_of::<ot_buffer_draw_text_record>(), 48);
        assert_eq!(size_of::<ot_buffer_draw_box>(), 104);
        assert_eq!(size_of::<ot_buffer_draw_cell>(), 48);
        assert_eq!(size_of::<ot_session_cursor_update>(), 24);
        assert_eq!(size_of::<ot_session_capabilities>(), 144);
        // Sizes below were verified with the C compiler against opentui.h v0.8.2.
        assert_eq!(size_of::<ot_session_pump_result>(), 24);
        assert_eq!(size_of::<ot_session_renderer_options>(), 24);
        assert_eq!(size_of::<ot_session_terminal_options>(), 16);
        assert_eq!(size_of::<ot_buffer_options>(), 24);
        assert_eq!(size_of::<ot_buffer_draw_fill>(), 40);
        assert_eq!(size_of::<ot_buffer_draw_clear>(), 24);
        assert_eq!(size_of::<ot_session_control_options>(), 20);
        assert_eq!(size_of::<ot_session_renderer_state>(), 40);
        assert_eq!(size_of::<ot_context_error>(), 16);
    }
}
