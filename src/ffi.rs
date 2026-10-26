//! Hand-written FFI for libmpv (client API + OpenGL render API).
//!
//! Every constant and struct layout mirrors the mpv 0.41 headers that Homebrew ships in
//! `/opt/homebrew/include/mpv/*.h`, which is the ABI we bind to at run time.

// The module deliberately mirrors the complete mpv ABI, including entry points and
// event constants the player does not call yet, so layout drift is caught by the
// ABI tests instead of by a crash.
#![allow(non_camel_case_types, non_snake_case, dead_code, unsafe_code)]

use std::os::raw::{c_char, c_int, c_ulong, c_void};

use libloading::Library;

pub type mpv_handle = *mut c_void;
pub type mpv_render_context = *mut c_void;

// -- mpv_format ------------------------------------------------------------------------------------
pub const MPV_FORMAT_NONE: c_int = 0;
pub const MPV_FORMAT_STRING: c_int = 1;
pub const MPV_FORMAT_OSD_STRING: c_int = 2;
pub const MPV_FORMAT_FLAG: c_int = 3;
pub const MPV_FORMAT_INT64: c_int = 4;
pub const MPV_FORMAT_DOUBLE: c_int = 5;
pub const MPV_FORMAT_NODE: c_int = 6;
pub const MPV_FORMAT_NODE_ARRAY: c_int = 7;
pub const MPV_FORMAT_NODE_MAP: c_int = 8;

// -- events ----------------------------------------------------------------------------------------
pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_LOG_MESSAGE: c_int = 2;
pub const MPV_EVENT_COMMAND_REPLY: c_int = 5;
pub const MPV_EVENT_START_FILE: c_int = 6;
pub const MPV_EVENT_END_FILE: c_int = 7;
pub const MPV_EVENT_FILE_LOADED: c_int = 8;
pub const MPV_EVENT_IDLE: c_int = 11;
pub const MPV_EVENT_CLIENT_MESSAGE: c_int = 16;
pub const MPV_EVENT_VIDEO_RECONFIG: c_int = 17;
pub const MPV_EVENT_SEEK: c_int = 20;
pub const MPV_EVENT_PLAYBACK_RESTART: c_int = 21;
pub const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;

pub const MPV_END_FILE_REASON_EOF: c_int = 0;
pub const MPV_END_FILE_REASON_STOP: c_int = 2;
pub const MPV_END_FILE_REASON_ERROR: c_int = 4;

// -- render API ------------------------------------------------------------------------------------
pub const MPV_RENDER_PARAM_INVALID: c_int = 0;
pub const MPV_RENDER_PARAM_API_TYPE: c_int = 1;
pub const MPV_RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
pub const MPV_RENDER_PARAM_OPENGL_FBO: c_int = 3;
pub const MPV_RENDER_PARAM_FLIP_Y: c_int = 4;
pub const MPV_RENDER_API_TYPE_OPENGL: &[u8] = b"opengl\0";
/// The ABI used by the mpv 0.41 headers this crate mirrors.
pub const MPV_CLIENT_API_VERSION: c_ulong = (2 << 16) | 5;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct mpv_event {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct mpv_event_property {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct mpv_event_log_message {
    pub prefix: *const c_char,
    pub level: *const c_char,
    pub text: *const c_char,
    pub log_level: c_int,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct mpv_event_end_file {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union mpv_node_u {
    pub string: *mut c_char,
    pub flag: c_int,
    pub int64: i64,
    pub double_: f64,
    pub list: *mut mpv_node_list,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mpv_node_list {
    pub num: c_int,
    pub values: *mut mpv_node,
    pub keys: *mut *mut c_char,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mpv_node {
    pub u: mpv_node_u,
    pub format: c_int,
}

impl mpv_node {
    pub fn none() -> Self {
        Self {
            u: mpv_node_u { int64: 0 },
            format: MPV_FORMAT_NONE,
        }
    }
}

#[repr(C)]
pub struct mpv_opengl_init_params {
    pub get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    pub get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct mpv_opengl_fbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct mpv_render_param {
    pub type_: c_int,
    pub data: *mut c_void,
}

// -- function types --------------------------------------------------------------------------------
pub type FnClientApiVersion = unsafe extern "C" fn() -> c_ulong;
pub type FnCreate = unsafe extern "C" fn() -> mpv_handle;
pub type FnInitialize = unsafe extern "C" fn(mpv_handle) -> c_int;
pub type FnTerminateDestroy = unsafe extern "C" fn(mpv_handle);
pub type FnErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
pub type FnFree = unsafe extern "C" fn(*mut c_void);
pub type FnFreeNodeContents = unsafe extern "C" fn(*mut mpv_node);
pub type FnSetOptionString =
    unsafe extern "C" fn(mpv_handle, *const c_char, *const c_char) -> c_int;
pub type FnSetPropertyString =
    unsafe extern "C" fn(mpv_handle, *const c_char, *const c_char) -> c_int;
pub type FnGetProperty =
    unsafe extern "C" fn(mpv_handle, *const c_char, c_int, *mut c_void) -> c_int;
pub type FnSetProperty =
    unsafe extern "C" fn(mpv_handle, *const c_char, c_int, *mut c_void) -> c_int;
pub type FnObserveProperty = unsafe extern "C" fn(mpv_handle, u64, *const c_char, c_int) -> c_int;
pub type FnUnobserveProperty = unsafe extern "C" fn(mpv_handle, u64) -> c_int;
pub type FnCommand = unsafe extern "C" fn(mpv_handle, *const *const c_char) -> c_int;
pub type FnCommandRet =
    unsafe extern "C" fn(mpv_handle, *const *const c_char, *mut c_void) -> c_int;
pub type FnCommandAsync = unsafe extern "C" fn(mpv_handle, u64, *const *const c_char) -> c_int;
pub type FnGetPropertyString = unsafe extern "C" fn(mpv_handle, *const c_char) -> *mut c_char;
pub type FnWaitEvent = unsafe extern "C" fn(mpv_handle, f64) -> *mut mpv_event;
pub type FnSetWakeupCallback =
    unsafe extern "C" fn(mpv_handle, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void);
pub type FnRequestLogMessages = unsafe extern "C" fn(mpv_handle, *const c_char) -> c_int;
pub type FnRenderContextCreate =
    unsafe extern "C" fn(*mut mpv_render_context, mpv_handle, *const mpv_render_param) -> c_int;
pub type FnRenderContextFree = unsafe extern "C" fn(mpv_render_context);
pub type FnRenderContextRender =
    unsafe extern "C" fn(mpv_render_context, *const mpv_render_param) -> c_int;
pub type FnRenderContextReportSwap = unsafe extern "C" fn(mpv_render_context);
pub type FnRenderContextSetUpdateCallback = unsafe extern "C" fn(
    mpv_render_context,
    Option<unsafe extern "C" fn(*mut c_void)>,
    *mut c_void,
);

/// Every libmpv entry point we use, as plain `extern "C"` function pointers.
#[derive(Clone, Copy)]
pub struct Api {
    pub client_api_version: FnClientApiVersion,
    pub create: FnCreate,
    pub initialize: FnInitialize,
    pub terminate_destroy: FnTerminateDestroy,
    pub error_string: FnErrorString,
    pub free: FnFree,
    pub free_node_contents: FnFreeNodeContents,
    pub set_option_string: FnSetOptionString,
    pub set_property_string: FnSetPropertyString,
    pub get_property: FnGetProperty,
    pub set_property: FnSetProperty,
    pub observe_property: FnObserveProperty,
    pub unobserve_property: FnUnobserveProperty,
    pub command: FnCommand,
    pub command_ret: FnCommandRet,
    pub command_async: FnCommandAsync,
    pub get_property_string: FnGetPropertyString,
    pub wait_event: FnWaitEvent,
    pub set_wakeup_callback: FnSetWakeupCallback,
    pub request_log_messages: FnRequestLogMessages,
    pub render_context_create: FnRenderContextCreate,
    pub render_context_free: FnRenderContextFree,
    pub render_context_render: FnRenderContextRender,
    pub render_context_report_swap: FnRenderContextReportSwap,
    pub render_context_set_update_callback: FnRenderContextSetUpdateCallback,
}

/// Candidate locations of `libmpv`, most preferred first.
pub fn candidate_paths() -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(custom) = std::env::var("MACPLAYER_LIBMPV")
        && !custom.is_empty()
    {
        paths.push(custom);
    }
    // A packaged `.app` keeps its private runtime libraries in Contents/Frameworks.
    // Resolve this from the executable instead of embedding a machine-specific path.
    if let Ok(executable) = std::env::current_exe()
        && let Some(contents) = executable.parent().and_then(|macos| macos.parent())
    {
        for name in ["libmpv.2.dylib", "libmpv.dylib"] {
            paths.push(contents.join("Frameworks").join(name).display().to_string());
        }
    }
    for p in [
        "/opt/homebrew/lib/libmpv.2.dylib",
        "/opt/homebrew/lib/libmpv.dylib",
        "/opt/homebrew/opt/mpv/lib/libmpv.2.dylib",
        "/usr/local/lib/libmpv.2.dylib",
        "/usr/local/opt/mpv/lib/libmpv.2.dylib",
        "/Applications/IINA.app/Contents/Frameworks/libmpv.2.dylib",
        "/usr/lib/x86_64-linux-gnu/libmpv.so.2",
        "/usr/lib/aarch64-linux-gnu/libmpv.so.2",
        "/usr/lib/libmpv.so.2",
        "libmpv.2.dylib",
        "libmpv.dylib",
        "libmpv.so.2",
        "libmpv.so",
    ] {
        paths.push(p.to_string());
    }
    paths
}

/// Load libmpv from disk and resolve every symbol we need.
///
/// The successfully selected [`Library`] is leaked on purpose: the player keeps it for the whole
/// process lifetime, and leaking lets the resolved function pointers remain valid.
pub fn load() -> Result<(&'static Library, Api), String> {
    let mut errors: Vec<String> = Vec::new();
    for path in candidate_paths() {
        let lib = match unsafe { Library::new(&path) } {
            Ok(lib) => lib,
            Err(err) => {
                errors.push(format!("{path}: {err}"));
                continue;
            }
        };
        match unsafe { load_from(&lib) } {
            Ok(api) => {
                let actual = unsafe { (api.client_api_version)() };
                let actual_major = actual >> 16;
                if actual_major != MPV_CLIENT_API_VERSION >> 16 || actual < MPV_CLIENT_API_VERSION {
                    errors.push(format!(
                        "{path}: unsupported libmpv client API version {actual:#x} (need at least {MPV_CLIENT_API_VERSION:#x} with major {:#x})",
                        MPV_CLIENT_API_VERSION >> 16
                    ));
                    continue;
                }
                let lib: &'static Library = Box::leak(Box::new(lib));
                return Ok((lib, api));
            }
            Err(err) => errors.push(format!("{path}: {err}")),
        }
    }
    Err(format!(
        "libmpv could not be loaded.\nInstall libmpv with your platform package \
         manager, or point MACPLAYER_LIBMPV at the library file.\n\n{}",
        errors.join("\n")
    ))
}

unsafe fn sym<T: Copy>(lib: &Library, name: &'static [u8]) -> Result<T, String> {
    let symbol = unsafe { lib.get::<T>(name) }
        .map_err(|e| format!("{}: {e}", String::from_utf8_lossy(name)))?;
    Ok(*symbol)
}

unsafe fn load_from(lib: &Library) -> Result<Api, String> {
    unsafe {
        Ok(Api {
            client_api_version: sym(lib, b"mpv_client_api_version\0")?,
            create: sym(lib, b"mpv_create\0")?,
            initialize: sym(lib, b"mpv_initialize\0")?,
            terminate_destroy: sym(lib, b"mpv_terminate_destroy\0")?,
            error_string: sym(lib, b"mpv_error_string\0")?,
            free: sym(lib, b"mpv_free\0")?,
            free_node_contents: sym(lib, b"mpv_free_node_contents\0")?,
            set_option_string: sym(lib, b"mpv_set_option_string\0")?,
            set_property_string: sym(lib, b"mpv_set_property_string\0")?,
            get_property: sym(lib, b"mpv_get_property\0")?,
            set_property: sym(lib, b"mpv_set_property\0")?,
            observe_property: sym(lib, b"mpv_observe_property\0")?,
            unobserve_property: sym(lib, b"mpv_unobserve_property\0")?,
            command: sym(lib, b"mpv_command\0")?,
            command_ret: sym(lib, b"mpv_command_ret\0")?,
            command_async: sym(lib, b"mpv_command_async\0")?,
            get_property_string: sym(lib, b"mpv_get_property_string\0")?,
            wait_event: sym(lib, b"mpv_wait_event\0")?,
            set_wakeup_callback: sym(lib, b"mpv_set_wakeup_callback\0")?,
            request_log_messages: sym(lib, b"mpv_request_log_messages\0")?,
            render_context_create: sym(lib, b"mpv_render_context_create\0")?,
            render_context_free: sym(lib, b"mpv_render_context_free\0")?,
            render_context_render: sym(lib, b"mpv_render_context_render\0")?,
            render_context_report_swap: sym(lib, b"mpv_render_context_report_swap\0")?,
            render_context_set_update_callback: sym(
                lib,
                b"mpv_render_context_set_update_callback\0",
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_init_params_match_mpv_041_layout() {
        assert_eq!(
            std::mem::size_of::<mpv_opengl_init_params>(),
            2 * std::mem::size_of::<*mut c_void>()
        );
    }

    #[test]
    fn end_file_layout_includes_playlist_redirect_fields() {
        assert_eq!(
            std::mem::size_of::<mpv_event_end_file>(),
            32,
            "mpv 0.41 is a 64-bit ABI here"
        );
    }
}
