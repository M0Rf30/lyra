// SPDX-License-Identifier: GPL-3.0

//! ProjectM visualizer integration (behind `visualizer` feature flag).
//!
//! Provides an offscreen-rendered music visualizer using the projectM library.
//! Renders to an FBO via a headless EGL context, reads pixels back, and sends
//! frames as icon handles for display in the expanded now-playing view.

use projectm::core::ProjectM;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Render resolution for the visualizer.
const RENDER_WIDTH: usize = 800;
const RENDER_HEIGHT: usize = 600;

/// The offscreen projectM renderer.
///
/// Owns a headless EGL/GL context, a projectM instance, and an FBO.
/// All rendering happens on a dedicated thread. Frames are read back
/// as RGBA pixels and sent to the UI.
pub struct ProjectMRenderer {
    projectm: ProjectM,
    /// OpenGL framebuffer object for offscreen rendering.
    _fbo: u32,
    /// OpenGL renderbuffer for color attachment.
    _color_rb: u32,
    /// Whether we successfully set up the GL context.
    _gl_ready: bool,
}

// SAFETY: ProjectM implements Send + Sync. The GL context is only used
// from the renderer thread.
unsafe impl Send for ProjectMRenderer {}

impl ProjectMRenderer {
    /// Create a new offscreen projectM renderer.
    ///
    /// Sets up a headless EGL context, creates an FBO, and initializes
    /// projectM with the given preset directory.
    pub fn new(preset_dir: Option<PathBuf>) -> Result<Self, String> {
        // --- EGL device-based headless context ---
        use glutin::api::egl::device::Device;
        use glutin::api::egl::display::Display;
        use glutin::config::{ConfigSurfaceTypes, ConfigTemplateBuilder};
        use glutin::context::{ContextApi, ContextAttributesBuilder};
        use glutin::display::GlDisplay;

        // Query EGL devices
        let devices: Vec<_> = Device::query_devices()
            .map_err(|e| format!("Failed to query EGL devices: {e}"))?
            .collect();

        if devices.is_empty() {
            return Err("No EGL devices available for headless rendering".to_string());
        }

        let device = &devices[0];

        // Create display from device (no windowing system)
        let display = unsafe {
            Display::with_device(device, None)
                .map_err(|e| format!("Failed to create EGL display: {e}"))?
        };

        // Configure for surfaceless offscreen rendering
        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_depth_size(24)
            .with_surface_type(ConfigSurfaceTypes::empty())
            .build();

        let config = unsafe {
            display
                .find_configs(template)
                .map_err(|e| format!("Failed to find EGL configs: {e}"))?
                .next()
                .ok_or("No suitable EGL config found")?
        };

        // Create context (OpenGL, no window handle)
        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(None))
            .build(None);

        let context = unsafe {
            display
                .create_context(&config, &context_attrs)
                .map_err(|e| format!("Failed to create GL context: {e}"))?
        };

        // Make current without a surface (surfaceless)
        let _context = context
            .make_current_surfaceless()
            .map_err(|e| format!("Failed to make context current (surfaceless): {e}"))?;

        // Load GL function pointers
        gl::load_with(|symbol| {
            let cstr = std::ffi::CString::new(symbol).unwrap();
            display.get_proc_address(cstr.as_c_str()) as *const _
        });

        // Create FBO + renderbuffer for offscreen rendering
        let (fbo, color_rb) = unsafe {
            let mut fbo = 0u32;
            let mut rb = 0u32;
            gl::GenFramebuffers(1, &mut fbo);
            gl::GenRenderbuffers(1, &mut rb);

            gl::BindFramebuffer(gl::FRAMEBUFFER, fbo);
            gl::BindRenderbuffer(gl::RENDERBUFFER, rb);
            gl::RenderbufferStorage(
                gl::RENDERBUFFER,
                gl::RGBA8,
                RENDER_WIDTH as i32,
                RENDER_HEIGHT as i32,
            );
            gl::FramebufferRenderbuffer(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::RENDERBUFFER,
                rb,
            );

            let status = gl::CheckFramebufferStatus(gl::FRAMEBUFFER);
            if status != gl::FRAMEBUFFER_COMPLETE {
                return Err(format!("Framebuffer incomplete: {status:#x}"));
            }

            gl::Viewport(0, 0, RENDER_WIDTH as i32, RENDER_HEIGHT as i32);

            (fbo, rb)
        };

        // Initialize projectM
        let pm = ProjectM::create();
        pm.set_window_size(RENDER_WIDTH, RENDER_HEIGHT);
        pm.set_fps(30);

        // Load presets if directory is provided
        if let Some(dir) = preset_dir
            && dir.exists()
        {
            let search_paths = vec![dir.to_string_lossy().to_string()];
            pm.set_texture_search_paths(&search_paths, search_paths.len());
        }

        Ok(Self {
            projectm: pm,
            _fbo: fbo,
            _color_rb: color_rb,
            _gl_ready: true,
        })
    }

    /// Render one frame and return RGBA pixel bytes.
    ///
    /// Feed PCM audio data to projectM, render a frame into the FBO,
    /// and read pixels back.
    pub fn render_frame(&self, pcm: &[f32]) -> Vec<u8> {
        // Feed audio samples if available
        if !pcm.is_empty() {
            self.projectm.pcm_add_float(pcm, projectm::core::STEREO);
        }

        // Render the visualization
        self.projectm.render_frame();

        // Read pixels from the FBO
        let mut pixels = vec![0u8; RENDER_WIDTH * RENDER_HEIGHT * 4];
        unsafe {
            gl::Finish();
            gl::ReadPixels(
                0,
                0,
                RENDER_WIDTH as i32,
                RENDER_HEIGHT as i32,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                pixels.as_mut_ptr() as *mut std::ffi::c_void,
            );
        }

        // OpenGL reads bottom-to-top, flip vertically for correct orientation
        let row_size = RENDER_WIDTH * 4;
        let mut flipped = vec![0u8; pixels.len()];
        for y in 0..RENDER_HEIGHT {
            let src_start = y * row_size;
            let dst_start = (RENDER_HEIGHT - 1 - y) * row_size;
            flipped[dst_start..dst_start + row_size]
                .copy_from_slice(&pixels[src_start..src_start + row_size]);
        }

        flipped
    }

    /// Select the next preset with a smooth transition.
    pub fn next_preset(&self) {
        // ProjectM handles preset cycling internally.
        // We trigger it by temporarily unlocking and relocking.
        let was_locked = self.projectm.get_preset_locked();
        self.projectm.set_preset_locked(false);
        // Force a switch by setting a very short duration briefly
        let old_duration = self.projectm.get_preset_duration();
        self.projectm.set_preset_duration(0.01);
        // Restore after a tiny render to trigger the switch
        self.projectm.render_frame();
        self.projectm.set_preset_duration(old_duration);
        self.projectm.set_preset_locked(was_locked);
    }

    /// Load a preset file directly.
    pub fn load_preset(&self, path: &str) {
        self.projectm.load_preset_file(path, true);
    }
}

/// Shared PCM ring buffer for audio tapping.
///
/// The audio thread writes samples into this buffer, and the visualizer
/// thread reads them out for feeding to projectM.
pub struct PcmBuffer {
    /// Circular buffer of interleaved stereo f32 samples.
    buffer: Vec<f32>,
    /// Write position in the buffer.
    write_pos: usize,
    /// Total capacity (number of f32 samples).
    capacity: usize,
}

impl PcmBuffer {
    /// Create a new PCM buffer with the given capacity in samples.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0.0; capacity],
            write_pos: 0,
            capacity,
        }
    }

    /// Write samples into the ring buffer.
    pub fn write(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.buffer[self.write_pos] = sample;
            self.write_pos = (self.write_pos + 1) % self.capacity;
        }
    }

    /// Read the most recent `count` samples from the buffer.
    pub fn read_recent(&self, count: usize) -> Vec<f32> {
        let count = count.min(self.capacity);
        let mut result = Vec::with_capacity(count);
        let start = if self.write_pos >= count {
            self.write_pos - count
        } else {
            self.capacity - (count - self.write_pos)
        };
        for i in 0..count {
            result.push(self.buffer[(start + i) % self.capacity]);
        }
        result
    }
}

/// Convert raw RGBA pixels to PNG bytes suitable for `icon::from_raster_bytes`.
pub fn rgba_to_png(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    use image::{ImageBuffer, RgbaImage};
    use std::io::Cursor;

    let img: RgbaImage = ImageBuffer::from_raw(width, height, pixels.to_vec())?;
    let mut output = Vec::new();
    let mut cursor = Cursor::new(&mut output);
    img.write_to(&mut cursor, image::ImageFormat::Png).ok()?;
    Some(output)
}

/// State holder for the visualizer render thread.
///
/// This is used by the subscription in `app.rs` to manage the render loop.
pub struct VisualizerState {
    /// The renderer (created on the render thread).
    pub renderer: Option<ProjectMRenderer>,
    /// Shared PCM buffer.
    pub pcm_buffer: Arc<Mutex<PcmBuffer>>,
    /// Whether the visualizer is actively rendering.
    pub active: Arc<AtomicBool>,
}

impl VisualizerState {
    /// Create a new visualizer state with a shared PCM buffer.
    pub fn new(pcm_buffer: Arc<Mutex<PcmBuffer>>) -> Self {
        Self {
            renderer: None,
            pcm_buffer,
            active: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Initialize the renderer on the current thread.
    pub fn init(&mut self, preset_dir: Option<PathBuf>) -> Result<(), String> {
        let renderer = ProjectMRenderer::new(preset_dir)?;
        self.renderer = Some(renderer);
        self.active.store(true, Ordering::Release);
        Ok(())
    }

    /// Render one frame, reading PCM from the shared buffer.
    pub fn render_one_frame(&self) -> Option<Vec<u8>> {
        let renderer = self.renderer.as_ref()?;
        let pcm = self
            .pcm_buffer
            .lock()
            .ok()
            .map(|buf| buf.read_recent(2048))
            .unwrap_or_default();
        let rgba = renderer.render_frame(&pcm);
        rgba_to_png(&rgba, RENDER_WIDTH as u32, RENDER_HEIGHT as u32)
    }

    /// Request the next preset.
    pub fn next_preset(&self) {
        if let Some(renderer) = &self.renderer {
            renderer.next_preset();
        }
    }

    /// Stop rendering.
    pub fn stop(&mut self) {
        self.active.store(false, Ordering::Release);
    }

    /// Check if actively rendering.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}
