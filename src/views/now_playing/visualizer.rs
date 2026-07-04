// SPDX-License-Identifier: GPL-3.0

//! ProjectM visualizer integration (behind `visualizer` feature flag).
//!
//! Provides an offscreen-rendered music visualizer using the projectM library.
//! Renders to an FBO via a headless EGL context, reads pixels back, and sends
//! frames as iced `image::Handle` for display in the expanded now-playing view.
//!
//! ## Known limitation — texture churn
//!
//! Each frame creates a new `image::Handle::from_rgba()` with a unique ID
//! (iced's API does not support updating pixel data for an existing handle).
//! This causes iced's raster cache to upload a new GPU texture and evict the
//! previous one every frame. In practice the upload+trim cycle completes within
//! a single render pass so visible flickering is unlikely, but the GPU memory
//! churn is sub-optimal. A stable-ID RGBA handle would require upstream changes
//! to iced's `image::Handle` / `image::Id` API.

use projectm::core::ProjectM;
use projectm::playlist::Playlist;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Render resolution for the visualizer (16:9).
/// Balanced for crispness when scaled to fullscreen vs. GPU→CPU readback cost
/// and texture upload overhead at 30fps.
const RENDER_WIDTH: usize = 960;
const RENDER_HEIGHT: usize = 540;

/// The offscreen projectM renderer.
///
/// Owns a headless EGL/GL context, a projectM instance, and an FBO.
/// All rendering happens on a dedicated thread. Frames are read back
/// as RGBA pixels and sent to the UI.
pub struct ProjectMRenderer {
    projectm: ProjectM,
    /// Playlist manager for preset cycling.
    playlist: Option<Playlist>,
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

        // Search for preset directories in common locations.
        // The caller-supplied dir is checked first, then system paths.
        let mut search_dirs: Vec<PathBuf> = Vec::new();
        if let Some(ref dir) = preset_dir {
            search_dirs.push(dir.clone());
        }
        // Common system-wide preset locations
        search_dirs.extend([
            PathBuf::from("/usr/share/projectM/presets"),
            PathBuf::from("/usr/local/share/projectM/presets"),
            PathBuf::from("/usr/share/projectm/presets"),
        ]);
        // Flatpak location
        if let Some(data) = dirs::data_dir() {
            search_dirs.push(data.join("projectM").join("presets"));
        }

        let mut pl = Playlist::create(&pm);
        let mut texture_paths = Vec::new();
        for dir in &search_dirs {
            if dir.exists() {
                let dir_str = dir.to_string_lossy().to_string();
                pl.add_path(&dir_str, true);
                texture_paths.push(dir_str);
            }
        }
        if !texture_paths.is_empty() {
            pm.set_texture_search_paths(&texture_paths, texture_paths.len());
        }

        let count = pl.len();
        tracing::info!("ProjectM: loaded {count} presets from {search_dirs:?}");
        let playlist = if count > 0 {
            pl.set_shuffle(true);
            pl.play_next();
            Some(pl)
        } else {
            tracing::warn!("ProjectM: no presets found in any search directory");
            None
        };

        Ok(Self {
            projectm: pm,
            playlist,
            _fbo: fbo,
            _color_rb: color_rb,
            _gl_ready: true,
        })
    }

    /// Render one frame and return RGBA pixel bytes.
    ///
    /// Feed PCM audio data to projectM, render a frame into the FBO,
    /// and read pixels back. The returned bytes are ready for direct use
    /// with `widget::icon::from_raster_pixels()` — no PNG encoding needed.
    pub fn render_frame(&self, pcm: &[f32]) -> Vec<u8> {
        // Feed audio samples if available, clamped to projectM's max buffer size
        if !pcm.is_empty() {
            let max = ProjectM::pcm_get_max_samples() as usize;
            let clamped = if pcm.len() > max { &pcm[..max] } else { pcm };
            self.projectm.pcm_add_float(clamped, projectm::core::STEREO);
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

        // OpenGL reads bottom-to-top — flip vertically in-place
        let row_size = RENDER_WIDTH * 4;
        for y in 0..RENDER_HEIGHT / 2 {
            let top = y * row_size;
            let bot = (RENDER_HEIGHT - 1 - y) * row_size;
            // Swap rows using split_at_mut to satisfy borrow checker
            let (first, second) = pixels.split_at_mut(bot);
            first[top..top + row_size].swap_with_slice(&mut second[..row_size]);
        }

        // Force alpha to fully opaque. projectM often renders with alpha < 255
        // which causes washed-out/transparent-looking colors when the image is
        // composited onto the UI background.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }

        pixels
    }

    /// Return the render resolution (width, height).
    pub const fn resolution() -> (u32, u32) {
        (RENDER_WIDTH as u32, RENDER_HEIGHT as u32)
    }

    /// Switch to the next preset via the playlist (hard cut).
    pub fn next_preset(&mut self) {
        if let Some(ref mut pl) = self.playlist {
            pl.play_next();
        }
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
    pub fn next_preset(&mut self) {
        if let Some(renderer) = &mut self.renderer {
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
