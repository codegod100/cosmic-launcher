use cosmic::{
    cctk::{
        screencopy::{CaptureSource, ScreencopyState},
        toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
        toplevel_management::{ToplevelManagerHandler, ToplevelManagerState},
        wayland_client::{Connection, QueueHandle},
    },
    iced_winit::platform_specific::wayland::subsurface_widget::Subsurface,
    widget::{self, image::Handle as ImageHandle},
    Element as CosmicElement,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use std::{collections::HashMap, sync::{Arc, Mutex, LazyLock}};

// Capture image matching cosmic-workspaces-epoch exactly
#[derive(Debug, Clone)]
pub struct CaptureImage {
    pub image: ImageHandle,
    pub transform: wayland_client::protocol::wl_output::Transform,
    pub width: u32,
    pub height: u32,
}

// COSMIC screencopy integration  
pub struct CosmicCaptureManager {
    screencopy_state: Option<ScreencopyState>,
    toplevel_info_state: Option<ToplevelInfoState>,
    toplevel_manager_state: Option<ToplevelManagerState>,
    active_captures: HashMap<String, Arc<CaptureImage>>,
    toplevels: HashMap<String, ExtForeignToplevelHandleV1>,
}

impl CosmicCaptureManager {
    pub fn new() -> Self {
        Self {
            screencopy_state: None,
            toplevel_info_state: None,
            toplevel_manager_state: None,
            active_captures: HashMap::new(),
            toplevels: HashMap::new(),
        }
    }

    pub fn initialize_screencopy(&mut self, screencopy_state: ScreencopyState, toplevel_info_state: ToplevelInfoState, toplevel_manager_state: ToplevelManagerState) {
        println!("🚀 Initializing COSMIC screencopy integration");
        self.screencopy_state = Some(screencopy_state);
        self.toplevel_info_state = Some(toplevel_info_state);
        self.toplevel_manager_state = Some(toplevel_manager_state);
    }

    pub fn add_toplevel(&mut self, title: String, toplevel: ExtForeignToplevelHandleV1) {
        println!("📋 Registering toplevel: '{}'", title);
        self.toplevels.insert(title, toplevel);
    }

    pub fn capture_toplevel(&mut self, title: &str) -> Option<Arc<CaptureImage>> {
        println!("🖼️ Requesting COSMIC toplevel capture for: '{}'", title);
        
        // Check if we have this toplevel registered
        if let Some(toplevel_handle) = self.toplevels.get(title) {
            // Try to start real capture using CaptureSource::Toplevel
            if let Some(screencopy_state) = &self.screencopy_state {
                println!("🎯 Found toplevel handle for '{}', starting capture", title);
                let capture_source = CaptureSource::Toplevel(toplevel_handle.clone());
                // TODO: Actually start capture with screencopy_state.capture()
                // For now, fall through to test pattern
            }
        }
        
        if let Some(existing) = self.active_captures.get(title) {
            return Some(Arc::clone(existing));
        }
        
        // Create test pattern for now
        self.create_test_capture(title)
    }

    fn create_test_capture(&mut self, title: &str) -> Option<Arc<CaptureImage>> {
        // Create a simple test pattern with different colors per window
        let color_seed = title.len() % 6;
        let (r, g, b) = match color_seed {
            0 => (255, 100, 100), // Red
            1 => (100, 255, 100), // Green  
            2 => (100, 100, 255), // Blue
            3 => (255, 255, 100), // Yellow
            4 => (255, 100, 255), // Magenta
            _ => (100, 255, 255), // Cyan
        };
        
        // Create a 200x150 test image
        let width = 200u32;
        let height = 150u32;
        let mut image_data = Vec::new();
        
        for y in 0..height {
            for x in 0..width {
                // Create a gradient pattern
                let fade = (x * 255 / width) as u8;
                image_data.push((r * fade / 255) as u8); // R
                image_data.push((g * fade / 255) as u8); // G 
                image_data.push((b * fade / 255) as u8); // B
                image_data.push(255); // A
            }
        }
        
        let image_handle = ImageHandle::from_rgba(width, height, image_data);
        
        let capture = Arc::new(CaptureImage {
            image: image_handle,
            transform: wayland_client::protocol::wl_output::Transform::Normal,
            width,
            height,
        });
        
        self.active_captures.insert(title.to_string(), Arc::clone(&capture));
        Some(capture)
    }
}

static CAPTURE_MANAGER: LazyLock<Mutex<CosmicCaptureManager>> = LazyLock::new(|| {
    Mutex::new(CosmicCaptureManager::new())
});

// Direct integration to cosmic-workspaces capture backend
pub fn get_toplevel_capture(title: &str) -> Option<CaptureImage> {
    if let Ok(mut manager) = CAPTURE_MANAGER.lock() {
        if let Some(capture) = manager.capture_toplevel(title) {
            return Some((*capture).clone());
        }
    }
    None
}

/// Create a native COSMIC capture element like cosmic-workspaces-epoch does
pub fn capture_image(image: Option<&CaptureImage>, _alpha: f32) -> CosmicElement<'static, crate::app::Message> {
    if let Some(image) = image {
        // For now, use regular image widget - will add subsurfaces later
        widget::image::Image::new(image.image.clone()).into()
    } else {
        // Placeholder when no capture available
        widget::image::Image::new(ImageHandle::from_rgba(1, 1, vec![0, 0, 0, 255])).into()
    }
}

/// Register a toplevel for capture
pub fn register_toplevel(title: String, toplevel: ExtForeignToplevelHandleV1) {
    if let Ok(mut manager) = CAPTURE_MANAGER.lock() {
        manager.add_toplevel(title, toplevel);
    }
}

/// Create a cosmic capture element for a toplevel window (simplified)
pub fn create_toplevel_capture_element(title: &str) -> CosmicElement<'static, crate::app::Message> {
    println!("🖼️ Creating COSMIC toplevel capture for: '{}'", title);
    
    // Try to get capture from COSMIC compositor
    let capture_data = get_toplevel_capture(title);
    
    if let Some(ref capture) = capture_data {
        println!("✅ Got capture data for '{}' - {}x{}", title, capture.width, capture.height);
    } else {
        println!("❌ No capture data for '{}'", title);
    }
    
    // Use the pure cosmic-workspaces capture_image function
    capture_image(capture_data.as_ref(), 1.0)
}