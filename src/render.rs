//! mpv render context + the OpenGL plumbing that puts decoded frames into an egui paint
//! callback.
//!
//! mpv renders into a private FBO of the video area's pixel size; the FBO is then blitted
//! onto the framebuffer egui is drawing into. egui does not restore framebuffer bindings
//! or the viewport after a paint callback, so both are saved and restored here.

#![allow(clippy::missing_safety_doc, unsafe_code)]

#[cfg(target_os = "macos")]
use std::ffi::CString;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glow::HasContext;

use crate::ffi::{
    Api, MPV_RENDER_API_TYPE_OPENGL, MPV_RENDER_PARAM_API_TYPE, MPV_RENDER_PARAM_FLIP_Y,
    MPV_RENDER_PARAM_INVALID, MPV_RENDER_PARAM_OPENGL_FBO, MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
    mpv_handle, mpv_opengl_fbo, mpv_opengl_init_params, mpv_render_context, mpv_render_param,
};

/// `get_proc_address` for mpv: try the global symbol table first (all of OpenGL.framework
/// lands there once loaded), then fall back to looking the symbol up in the OpenGL bundle.
unsafe extern "C" fn get_proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    let symbol = unsafe { CStr::from_ptr(name) };

    // GL 1.x/2.x symbols are exported process-wide after the framework is loaded. The
    // RTLD_DEFAULT sentinel is platform-specific: -2 on Darwin and NULL on Linux/glibc.
    let handle = rtld_default();
    if let Some(found) = unsafe { lookup_global_symbol(handle, name) } {
        return found;
    }

    // Everything else (extensions) lives in the bundle's symbol table on macOS.
    #[cfg(target_os = "macos")]
    {
        unsafe { bundle_symbol(symbol) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = symbol;
        std::ptr::null_mut()
    }
}

#[cfg(target_os = "macos")]
fn rtld_default() -> *mut c_void {
    -2isize as *mut c_void
}

#[cfg(not(target_os = "macos"))]
fn rtld_default() -> *mut c_void {
    std::ptr::null_mut()
}

/// # Safety
/// `handle` must be a valid `dlopen` handle (`RTLD_DEFAULT` is fine) and `name` NUL-terminated.
unsafe fn lookup_global_symbol(handle: *mut c_void, name: *const c_char) -> Option<*mut c_void> {
    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    let found = unsafe { dlsym(handle, name) };
    if found.is_null() { None } else { Some(found) }
}

/// Look a GL symbol up in Apple's OpenGL bundle, the way glutin/winit do.
///
/// # Safety
/// `name` must be NUL-terminated.
#[cfg(target_os = "macos")]
unsafe fn bundle_symbol(name: &CStr) -> *mut c_void {
    #[repr(C)]
    struct __CFString(std::ffi::c_void);
    type CFStringRef = *const __CFString;
    type CFBundleRef = *mut std::ffi::c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFBundleGetBundleWithIdentifier(bundle_id: CFStringRef) -> CFBundleRef;
        fn CFBundleGetFunctionPointerForName(
            bundle: CFBundleRef,
            function_name: CFStringRef,
        ) -> *mut c_void;
        fn CFStringCreateWithCString(
            alloc: *const std::ffi::c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: *const std::ffi::c_void);
    }
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    unsafe {
        let identifier = match CString::new("com.apple.opengl") {
            Ok(v) => v,
            Err(_) => return std::ptr::null_mut(),
        };
        let identifier = CFStringCreateWithCString(
            std::ptr::null(),
            identifier.as_ptr(),
            K_CF_STRING_ENCODING_UTF8,
        );
        if identifier.is_null() {
            return std::ptr::null_mut();
        }
        let bundle = CFBundleGetBundleWithIdentifier(identifier);
        CFRelease(identifier as *const std::ffi::c_void);
        if bundle.is_null() {
            return std::ptr::null_mut();
        }
        let function_name =
            CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), K_CF_STRING_ENCODING_UTF8);
        if function_name.is_null() {
            return std::ptr::null_mut();
        }
        let pointer = CFBundleGetFunctionPointerForName(bundle, function_name);
        CFRelease(function_name as *const std::ffi::c_void);
        pointer
    }
}

/// Owns the mpv render context and the FBO mpv draws into.
pub struct Renderer {
    api: Api,
    mpv: mpv_handle,
    context: Option<mpv_render_context>,
    fbo: Option<glow::Framebuffer>,
    texture: Option<glow::Texture>,
    fbo_width: i32,
    fbo_height: i32,
    /// Set by mpv's update callback; the UI polls and clears it.
    frame_available: Arc<AtomicBool>,
    /// Kept alive because the update callback holds a raw pointer to it.
    _repaint_hook: Box<RepaintHook>,
    /// Last reported GL error; a persistent failure only logs once.
    last_error: String,
    gl_ptr: usize,
}

/// Passed to `mpv_render_context_set_update_callback`; points at the egui context.
struct RepaintHook {
    ctx: egui::Context,
    frame_available: Arc<AtomicBool>,
}

unsafe extern "C" fn on_frame_available(userdata: *mut c_void) {
    if userdata.is_null() {
        return;
    }
    let hook = unsafe { &*(userdata as *const RepaintHook) };
    hook.frame_available.store(true, Ordering::Release);
    hook.ctx.request_repaint();
}

// SAFETY: the render context is only ever touched from the paint callback, i.e. the UI
// thread with the GL context current; the raw pointers inside stay valid for the process.
unsafe impl Send for Renderer {}

impl Renderer {
    pub fn new(api: Api, mpv: mpv_handle, egui_ctx: egui::Context) -> Self {
        let frame_available = Arc::new(AtomicBool::new(false));
        let hook = Box::new(RepaintHook {
            ctx: egui_ctx,
            frame_available: Arc::clone(&frame_available),
        });
        Self {
            api,
            mpv,
            context: None,
            fbo: None,
            texture: None,
            fbo_width: 0,
            fbo_height: 0,
            frame_available,
            _repaint_hook: hook,
            last_error: String::new(),
            gl_ptr: 0,
        }
    }

    /// `true` when `message` differs from the previously reported error, so a
    /// persistent GL failure does not flood the log once per frame. A successful
    /// render clears the memory, letting a recurring failure log again.
    pub fn error_changed(&mut self, message: &str) -> bool {
        if self.last_error == message {
            false
        } else {
            self.last_error = message.to_string();
            true
        }
    }

    /// Forget the last error after a successful frame.
    pub fn note_ok(&mut self) {
        self.last_error.clear();
    }

    /// `true` when mpv signalled that a new frame (or a reconfiguration) is ready.
    pub fn take_frame_available(&self) -> bool {
        self.frame_available.swap(false, Ordering::AcqRel)
    }

    /// Ask for one more render pass even without an mpv update signal. Used when the
    /// player goes idle: without it the last decoded frame stays visible in the FBO
    /// forever, bleeding under the idle screen.
    pub fn request_render(&self) {
        self.frame_available.store(true, Ordering::Release);
    }

    /// Create the render context. Must run on the thread that owns the GL context.
    ///
    /// # Safety
    /// `gl` must be current on the calling thread.
    pub unsafe fn ensure_context(&mut self, gl: &glow::Context) -> Result<(), String> {
        let gl_ptr = gl as *const glow::Context as usize;
        if self.context.is_some() && self.gl_ptr == gl_ptr {
            return Ok(());
        }
        // A different `glow::Context` means the GL context was recreated (e.g. a GL
        // reload); the old render context and FBO belong to the dead context.
        if self.context.is_some() {
            unsafe { self.drop_gl_objects() };
        }
        self.gl_ptr = gl_ptr;

        let mut init = mpv_opengl_init_params {
            get_proc_address,
            get_proc_address_ctx: std::ptr::null_mut(),
        };
        // The constant is a pre-terminated byte string ("opengl\0"), so CStr is the right tool.
        let api_type = CStr::from_bytes_with_nul(MPV_RENDER_API_TYPE_OPENGL)
            .map_err(|_| "invalid mpv OpenGL API name".to_string())?;
        let params = [
            mpv_render_param {
                type_: MPV_RENDER_PARAM_API_TYPE,
                data: api_type.as_ptr() as *mut c_void,
            },
            mpv_render_param {
                type_: MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: &mut init as *mut mpv_opengl_init_params as *mut c_void,
            },
            mpv_render_param {
                type_: MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];

        let mut context: mpv_render_context = std::ptr::null_mut();
        let rc =
            unsafe { (self.api.render_context_create)(&mut context, self.mpv, params.as_ptr()) };
        if rc < 0 || context.is_null() {
            if !context.is_null() {
                unsafe { (self.api.render_context_free)(context) };
            }
            return Err(format!(
                "mpv_render_context_create: {}",
                unsafe { CStr::from_ptr((self.api.error_string)(rc)) }.to_string_lossy()
            ));
        }

        let hook_ptr = (&*self._repaint_hook as *const RepaintHook)
            .cast_mut()
            .cast::<c_void>();
        self.frame_available.store(true, Ordering::Release);
        unsafe {
            (self.api.render_context_set_update_callback)(
                context,
                Some(on_frame_available),
                hook_ptr,
            );
        }
        self.context = Some(context);
        Ok(())
    }

    /// Render the current video frame into `rect` (already in pixels) of the framebuffer
    /// that egui is drawing into.
    ///
    /// # Safety
    /// `gl` must be current on the calling thread and the context must exist.
    pub unsafe fn render(&mut self, gl: &glow::Context, rect: RectPx) -> Result<(), String> {
        let Some(context) = self.context else {
            return Ok(());
        };
        if rect.width <= 0 || rect.height <= 0 {
            return Ok(());
        }

        let right = rect
            .x
            .checked_add(rect.width)
            .ok_or_else(|| "render rectangle exceeds i32 bounds".to_string())?;
        let top = rect
            .y
            .checked_add(rect.height)
            .ok_or_else(|| "render rectangle exceeds i32 bounds".to_string())?;

        // FBO allocation also binds framebuffers and a texture, so capture the state before
        // resizing. This must happen before ensure_fbo, not after it.
        let previous_bindings = unsafe { GlBindings::capture(gl) };
        let previous_viewport = {
            let mut viewport = [0_i32; 4];
            unsafe { gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport) };
            viewport
        };
        let previous_pack_alignment = unsafe { gl.get_parameter_i32(glow::PACK_ALIGNMENT) };

        let resized =
            self.fbo.is_none() || self.fbo_width != rect.width || self.fbo_height != rect.height;
        unsafe { self.ensure_fbo(gl, rect.width, rect.height)? };

        let Some(fbo) = self.fbo else {
            return Ok(());
        };

        let frame_available = self.take_frame_available();
        let should_render = resized || frame_available;

        unsafe {
            if should_render {
                let mut fbo_param = mpv_opengl_fbo {
                    fbo: fbo.0.get() as i32,
                    w: rect.width,
                    h: rect.height,
                    internal_format: 0,
                };
                let mut flip_y: c_int = 1;
                let params = [
                    mpv_render_param {
                        type_: MPV_RENDER_PARAM_OPENGL_FBO,
                        data: &mut fbo_param as *mut mpv_opengl_fbo as *mut c_void,
                    },
                    mpv_render_param {
                        type_: MPV_RENDER_PARAM_FLIP_Y,
                        data: &mut flip_y as *mut c_int as *mut c_void,
                    },
                    mpv_render_param {
                        type_: MPV_RENDER_PARAM_INVALID,
                        data: std::ptr::null_mut(),
                    },
                ];

                gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.viewport(0, 0, rect.width, rect.height);

                let rc = (self.api.render_context_render)(context, params.as_ptr());

                gl.pixel_store_i32(glow::PACK_ALIGNMENT, previous_pack_alignment);

                if rc < 0 {
                    self.frame_available.store(true, Ordering::Release);
                    restore_render_state(
                        gl,
                        previous_bindings,
                        previous_viewport,
                        previous_pack_alignment,
                    );
                    return Err(format!(
                        "mpv_render_context_render: {}",
                        CStr::from_ptr((self.api.error_string)(rc)).to_string_lossy()
                    ));
                }
            }

            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
            gl.bind_framebuffer(
                glow::DRAW_FRAMEBUFFER,
                non_zero_framebuffer(previous_bindings.draw),
            );
            gl.blit_framebuffer(
                0,
                0,
                rect.width,
                rect.height,
                rect.x,
                rect.y,
                right,
                top,
                glow::COLOR_BUFFER_BIT,
                glow::LINEAR,
            );

            restore_render_state(
                gl,
                previous_bindings,
                previous_viewport,
                previous_pack_alignment,
            );
        }
        Ok(())
    }

    /// (Re)create the offscreen FBO whenever the video area's pixel size changes.
    ///
    /// # Safety
    /// `gl` must be current on the calling thread.
    unsafe fn ensure_fbo(
        &mut self,
        gl: &glow::Context,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        if self.fbo.is_some() && self.fbo_width == width && self.fbo_height == height {
            return Ok(());
        }

        // Build the replacement before destroying the working FBO. A transient allocation or
        // completeness failure should not leave the renderer without a usable previous frame.
        let previous_bindings = unsafe { GlBindings::capture(gl) };
        let texture = match unsafe { gl.create_texture() } {
            Ok(texture) => texture,
            Err(error) => {
                unsafe { previous_bindings.restore(gl) };
                return Err(format!("create_texture: {error}"));
            }
        };
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width,
                height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
        }

        let fbo = match unsafe { gl.create_framebuffer() } {
            Ok(fbo) => fbo,
            Err(error) => {
                unsafe {
                    gl.delete_texture(texture);
                    previous_bindings.restore(gl);
                }
                return Err(format!("create_framebuffer: {error}"));
            }
        };
        let status;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        }
        if status != glow::FRAMEBUFFER_COMPLETE {
            unsafe {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(texture);
                previous_bindings.restore(gl);
            }
            return Err(format!("framebuffer incomplete: 0x{status:x}"));
        }

        unsafe {
            if let Some(old_fbo) = self.fbo {
                gl.delete_framebuffer(old_fbo);
            }
            if let Some(old_texture) = self.texture {
                gl.delete_texture(old_texture);
            }
            previous_bindings.restore(gl);
        }

        self.fbo = Some(fbo);
        self.texture = Some(texture);
        self.fbo_width = width;
        self.fbo_height = height;
        Ok(())
    }

    /// # Safety
    /// `gl` must be current on the calling thread.
    unsafe fn destroy_fbo(&mut self, gl: &glow::Context) {
        if let Some(fbo) = self.fbo.take() {
            unsafe { gl.delete_framebuffer(fbo) };
        }
        if let Some(texture) = self.texture.take() {
            unsafe { gl.delete_texture(texture) };
        }
        self.fbo_width = 0;
        self.fbo_height = 0;
    }

    /// # Safety
    /// `gl` must be current on the calling thread.
    unsafe fn drop_gl_objects(&mut self) {
        if let Some(context) = self.context.take() {
            unsafe {
                (self.api.render_context_set_update_callback)(context, None, std::ptr::null_mut());
                (self.api.render_context_free)(context);
            }
        }
        self.fbo = None;
        self.texture = None;
        self.fbo_width = 0;
        self.fbo_height = 0;
    }

    /// Free GL-owned resources. Must run while `gl` is still current.
    ///
    /// # Safety
    /// `gl` must be current on the calling thread.
    pub unsafe fn shutdown(&mut self, gl: &glow::Context) {
        unsafe { self.destroy_fbo(gl) };
        if let Some(context) = self.context.take() {
            unsafe {
                (self.api.render_context_set_update_callback)(context, None, std::ptr::null_mut());
                (self.api.render_context_free)(context);
            }
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // GL objects can only be deleted with the context current; if we get here without
        // one, mpv's core teardown still releases the mpv-side state.
        if let Some(context) = self.context.take() {
            unsafe {
                (self.api.render_context_set_update_callback)(context, None, std::ptr::null_mut());
                (self.api.render_context_free)(context);
            }
        }
    }
}

/// Rectangle in physical pixels, with the y axis pointing up (GL convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectPx {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

fn non_zero_framebuffer(name: i32) -> Option<glow::Framebuffer> {
    u32::try_from(name)
        .ok()
        .and_then(std::num::NonZeroU32::new)
        .map(glow::NativeFramebuffer)
}

#[derive(Clone, Copy)]
struct GlBindings {
    draw: i32,
    read: i32,
    texture_2d: i32,
    active_texture: i32,
}

impl GlBindings {
    /// # Safety
    /// `gl` must be current on the calling thread.
    unsafe fn capture(gl: &glow::Context) -> Self {
        Self {
            draw: unsafe { gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING) },
            read: unsafe { gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING) },
            texture_2d: unsafe { gl.get_parameter_i32(glow::TEXTURE_BINDING_2D) },
            active_texture: unsafe { gl.get_parameter_i32(glow::ACTIVE_TEXTURE) },
        }
    }

    /// # Safety
    /// `gl` must be current on the calling thread.
    unsafe fn restore(self, gl: &glow::Context) {
        unsafe {
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, non_zero_framebuffer(self.draw));
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, non_zero_framebuffer(self.read));
            if let Ok(active_texture) = u32::try_from(self.active_texture) {
                gl.active_texture(active_texture);
            }
            gl.bind_texture(glow::TEXTURE_2D, non_zero_texture(self.texture_2d));
        }
    }
}

fn non_zero_texture(name: i32) -> Option<glow::Texture> {
    u32::try_from(name)
        .ok()
        .and_then(std::num::NonZeroU32::new)
        .map(glow::NativeTexture)
}

/// # Safety
/// `gl` must be current on the calling thread.
unsafe fn restore_render_state(
    gl: &glow::Context,
    bindings: GlBindings,
    viewport: [i32; 4],
    pack_alignment: i32,
) {
    unsafe {
        bindings.restore(gl);
        gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, pack_alignment);
    }
}

#[cfg(test)]
mod tests {
    use super::non_zero_framebuffer;

    #[test]
    fn framebuffer_names_only_accept_valid_positive_names() {
        assert!(non_zero_framebuffer(0).is_none());
        assert!(non_zero_framebuffer(-1).is_none());
        assert_eq!(non_zero_framebuffer(7).unwrap().0.get(), 7);
    }
}
