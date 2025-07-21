use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ashpd::desktop::screenshot::Screenshot;
use ashpd::WindowIdentifier;
use crate::cosmic_window_info::{CosmicWindowManager, WindowGeometry};
use crate::cosmic_toplevel_protocol::CosmicToplevelProtocol;

#[derive(Debug, Clone)]
pub struct PortalScreenshot {
    pub window_id: String,
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: std::time::Instant,
}

#[derive(Clone)]
pub struct PortalManager {
    cache: HashMap<String, PortalScreenshot>,
    max_cache_size: usize,
    cache_ttl: std::time::Duration,
    cosmic_window_manager: Arc<Mutex<CosmicWindowManager>>,
    cosmic_protocol: Option<Arc<Mutex<CosmicToplevelProtocol>>>,
}

impl PortalManager {
    pub fn new() -> Self {
        let window_manager = Arc::new(Mutex::new(CosmicWindowManager::new()));
        let cosmic_protocol = CosmicToplevelProtocol::new(Arc::clone(&window_manager));
        
        Self {
            cache: HashMap::new(),
            max_cache_size: 20,
            cache_ttl: std::time::Duration::from_secs(5),
            cosmic_window_manager: window_manager,
            cosmic_protocol: Some(Arc::new(Mutex::new(cosmic_protocol))),
        }
    }

    pub fn initialize_cosmic_protocol(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(ref protocol_arc) = self.cosmic_protocol {
            if let Ok(mut protocol) = protocol_arc.lock() {
                protocol.connect()?;
                println!("✅ COSMIC toplevel protocol initialized");
                
                // Spawn a thread to handle protocol events
                let protocol_clone = Arc::clone(protocol_arc);
                std::thread::spawn(move || {
                    if let Ok(mut protocol_guard) = protocol_clone.lock() {
                        if let Err(e) = protocol_guard.run_event_loop() {
                            eprintln!("❌ COSMIC protocol event loop error: {}", e);
                        }
                    }
                });
            }
        }
        Ok(())
    }

    pub async fn capture_screen(&mut self) -> Result<PortalScreenshot, Box<dyn std::error::Error>> {
        println!("🔐 Requesting screenshot permission via XDG Portal...");
        
        // Capture the screen using the builder pattern - this will show a permission dialog
        let request = Screenshot::request()
            .interactive(false)  // Don't show customization dialog
            .modal(true)         // Make it modal
            .send()
            .await?;
        
        let response = request.response()?;
        
        // The response contains a file path to the screenshot
        let screenshot_path = response.uri().to_file_path()
            .map_err(|_| "Failed to convert URI to file path")?;
        
        println!("✅ Screenshot saved to: {:?}", screenshot_path);
        
        // Read the screenshot file
        let image_data = std::fs::read(&screenshot_path)?;
        
        // Load image to get dimensions
        let image = image::load_from_memory(&image_data)?;
        
        Ok(PortalScreenshot {
            window_id: "portal_screen".to_string(),
            image_data,
            width: image.width(),
            height: image.height(),
            timestamp: std::time::Instant::now(),
        })
    }

    pub async fn capture_window_by_title(&mut self, title: &str) -> Result<PortalScreenshot, Box<dyn std::error::Error>> {
        println!("🔐 Portal screenshot requested for: '{}'", title);
        
        // Get the base full-screen screenshot
        let base_screenshot = if let Some(cached) = self.get_cached_screenshot("portal_screen") {
            println!("📋 Using cached portal screenshot");
            cached.clone()
        } else {
            let screenshot = self.capture_screen().await?;
            self.cache_screenshot(screenshot.clone());
            screenshot
        };
        
        // Try to get individual window bounds and crop the screenshot
        match self.crop_screenshot_for_window(&base_screenshot, title).await {
            Ok(cropped) => {
                println!("✂️  Cropped screenshot for: '{}'", title);
                Ok(cropped)
            }
            Err(_) => {
                println!("⚠️  Cropping failed, using full screenshot for: '{}'", title);
                // Fallback to full screenshot with unique window_id
                Ok(PortalScreenshot {
                    window_id: title.to_string(),
                    image_data: base_screenshot.image_data,
                    width: base_screenshot.width,
                    height: base_screenshot.height,
                    timestamp: base_screenshot.timestamp,
                })
            }
        }
    }
    
    async fn crop_screenshot_for_window(&self, base_screenshot: &PortalScreenshot, title: &str) -> Result<PortalScreenshot, Box<dyn std::error::Error>> {
        // Try to get window geometry using xcap first
        let windows = xcap::Window::all()?;
        
        for window in windows {
            let window_title = window.title();
            let app_name = window.app_name();
            
            // Try to match this window to our title
            let matches = !window_title.is_empty() && (
                window_title.contains(title) || 
                title.contains(&window_title)
            ) || !app_name.is_empty() && (
                app_name.to_lowercase().contains(&title.to_lowercase()) ||
                title.to_lowercase().contains(&app_name.to_lowercase()) ||
                (app_name == "discord" && title.to_lowercase().contains("discord")) ||
                (app_name == "mattermost" && title.to_lowercase().contains("mattermost"))
            );
            
            if matches {
                println!("🎯 Found window match for '{}': app='{}', title='{}'", title, app_name, window_title);
                
                // Try to get window bounds (this might not work on Wayland but worth trying)
                if let Ok(window_image) = window.capture_image() {
                    // If we can capture the window directly, let's use its dimensions as a hint
                    let window_width = window_image.width();
                    let window_height = window_image.height();
                    
                    println!("📐 Window dimensions: {}x{}", window_width, window_height);
                    
                    // Create a cropped version (for now, just resize to window dimensions)
                    return self.resize_screenshot(base_screenshot, window_width, window_height, title);
                }
            }
        }
        
        Err("No matching window found for cropping".into())
    }
    
    fn resize_screenshot(&self, base_screenshot: &PortalScreenshot, target_width: u32, target_height: u32, title: &str) -> Result<PortalScreenshot, Box<dyn std::error::Error>> {
        // Load the base image
        let base_image = image::load_from_memory(&base_screenshot.image_data)?;
        
        // Create different crops for different windows to provide variety
        let crop_region = self.get_crop_region_for_window(title, base_image.width(), base_image.height());
        
        println!("🔍 Crop region for '{}': x={}, y={}, w={}, h={}", 
            title, crop_region.0, crop_region.1, crop_region.2, crop_region.3);
        
        // Crop the image to the specific region
        let cropped_image = base_image.crop_imm(crop_region.0, crop_region.1, crop_region.2, crop_region.3);
        
        // Resize to target window dimensions
        let final_image = cropped_image.resize(target_width, target_height, image::imageops::FilterType::Lanczos3);
        
        // Convert back to bytes
        let mut final_data = Vec::new();
        final_image.write_to(&mut std::io::Cursor::new(&mut final_data), image::ImageFormat::Png)?;
        
        Ok(PortalScreenshot {
            window_id: title.to_string(),
            image_data: final_data,
            width: target_width,
            height: target_height,
            timestamp: base_screenshot.timestamp,
        })
    }
    
    fn get_crop_region_for_window(&self, title: &str, screen_width: u32, screen_height: u32) -> (u32, u32, u32, u32) {
        // Try to get real window geometry from COSMIC first
        let geometry = if let Ok(window_manager) = self.cosmic_window_manager.lock() {
            window_manager.get_window_geometry(title).cloned()
        } else {
            None
        };
        
        if let Some(geometry) = geometry {
            println!("🎯 Using real window geometry for '{}': {}x{} at ({}, {})", 
                title, geometry.width, geometry.height, geometry.x, geometry.y);
            
            // Ensure coordinates are within screen bounds
            let x = (geometry.x.max(0) as u32).min(screen_width.saturating_sub(100));
            let y = (geometry.y.max(0) as u32).min(screen_height.saturating_sub(100));
            let width = geometry.width.min(screen_width - x);
            let height = geometry.height.min(screen_height - y);
            
            return (x, y, width, height);
        }
        
        println!("⚠️  No real geometry available for '{}', using fallback regions", title);
        
        // Fallback to hardcoded regions when COSMIC geometry not available
        let crop_width = screen_width / 2;
        let crop_height = screen_height / 2;
        
        // Assign different screen regions to different window types
        if title.to_lowercase().contains("firefox") || title.to_lowercase().contains("mozilla") {
            // Top-left quadrant for browsers
            (0, 0, crop_width, crop_height)
        } else if title.to_lowercase().contains("discord") {
            // Top-right quadrant for chat apps
            (crop_width, 0, crop_width, crop_height)
        } else if title.to_lowercase().contains("terminal") || title.to_lowercase().contains("cosmic terminal") {
            // Bottom-left quadrant for terminals
            (0, crop_height, crop_width, crop_height)
        } else if title.to_lowercase().contains("files") || title.to_lowercase().contains("cosmic files") {
            // Bottom-right quadrant for file managers
            (crop_width, crop_height, crop_width, crop_height)
        } else if title.to_lowercase().contains("mattermost") {
            // Center region for mattermost
            (crop_width / 2, crop_height / 2, crop_width, crop_height)
        } else {
            // Default: slightly offset center for other apps
            let offset_x = (title.len() as u32 * 50) % (screen_width / 4);
            let offset_y = (title.len() as u32 * 30) % (screen_height / 4);
            (offset_x, offset_y, crop_width, crop_height)
        }
    }

    pub fn get_cached_screenshot(&self, window_id: &str) -> Option<&PortalScreenshot> {
        if let Some(screenshot) = self.cache.get(window_id) {
            if screenshot.timestamp.elapsed() <= self.cache_ttl {
                return Some(screenshot);
            }
        }
        None
    }

    pub fn cache_screenshot(&mut self, screenshot: PortalScreenshot) {
        if self.cache.len() >= self.max_cache_size {
            self.cleanup_old_cache();
        }
        
        self.cache.insert(screenshot.window_id.clone(), screenshot);
    }

    fn cleanup_old_cache(&mut self) {
        let now = std::time::Instant::now();
        self.cache.retain(|_, screenshot| {
            now.duration_since(screenshot.timestamp) <= self.cache_ttl
        });
        
        if self.cache.len() >= self.max_cache_size {
            let oldest_key = self.cache
                .iter()
                .min_by_key(|(_, screenshot)| screenshot.timestamp)
                .map(|(key, _)| key.clone());
            
            if let Some(key) = oldest_key {
                self.cache.remove(&key);
            }
        }
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    pub fn update_window_geometry(&mut self, title: String, x: i32, y: i32, width: u32, height: u32) {
        if let Ok(mut window_manager) = self.cosmic_window_manager.lock() {
            window_manager.update_window_geometry(title, x, y, width, height);
        }
    }

    pub fn get_window_manager(&self) -> Arc<Mutex<CosmicWindowManager>> {
        Arc::clone(&self.cosmic_window_manager)
    }
}

impl Default for PortalManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_cosmic_image_handle(screenshot: &PortalScreenshot) -> Result<cosmic::widget::image::Handle, Box<dyn std::error::Error>> {
    Ok(cosmic::widget::image::Handle::from_bytes(screenshot.image_data.clone()))
}