use pop_launcher::SearchResult;
use std::collections::HashMap;
use xcap::Window;

#[derive(Debug, Clone)]
pub struct WindowScreenshot {
    pub window_id: String,
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: std::time::Instant,
}

#[derive(Clone)]
pub struct ScreenshotManager {
    cache: HashMap<String, WindowScreenshot>,
    max_cache_size: usize,
    cache_ttl: std::time::Duration,
}

impl ScreenshotManager {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            max_cache_size: 20,
            cache_ttl: std::time::Duration::from_secs(5),
        }
    }

    pub fn with_cache_size(mut self, size: usize) -> Self {
        self.max_cache_size = size;
        self
    }

    pub fn with_cache_ttl(mut self, ttl: std::time::Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    pub fn capture_window_by_title(&mut self, title: &str) -> Result<WindowScreenshot, Box<dyn std::error::Error>> {
        let windows = Window::all().map_err(|e| format!("Failed to get windows: {}", e))?;
        
        for window in windows {
            // Skip minimized windows
            if window.is_minimized() {
                continue;
            }
            
            let window_title = window.title().to_string();
            if window_title.contains(title) || title.contains(&window_title) {
                return self.capture_window(&window);
            }
        }
        
        Err(format!("Window with title '{}' not found", title).into())
    }

    pub fn capture_all_windows(&mut self) -> Result<Vec<WindowScreenshot>, Box<dyn std::error::Error>> {
        let windows = Window::all().map_err(|e| format!("Failed to get windows: {}", e))?;
        let mut screenshots = Vec::new();
        
        for window in windows {
            // Skip minimized windows
            if window.is_minimized() {
                continue;
            }
            
            match self.capture_window(&window) {
                Ok(screenshot) => screenshots.push(screenshot),
                Err(e) => {
                    let title = window.title().to_string();
                    tracing::warn!("Failed to capture window '{}': {}", title, e);
                }
            }
        }
        
        Ok(screenshots)
    }

    fn capture_window(&self, window: &Window) -> Result<WindowScreenshot, Box<dyn std::error::Error>> {
        let image = window.capture_image()?;
        let title = window.title().to_string();
        
        let mut image_data = Vec::new();
        image.write_to(&mut std::io::Cursor::new(&mut image_data), image::ImageFormat::Png)?;
        
        Ok(WindowScreenshot {
            window_id: title.clone(),
            image_data,
            width: image.width(),
            height: image.height(),
            timestamp: std::time::Instant::now(),
        })
    }

    pub fn get_cached_screenshot(&self, window_id: &str) -> Option<&WindowScreenshot> {
        if let Some(screenshot) = self.cache.get(window_id) {
            if screenshot.timestamp.elapsed() <= self.cache_ttl {
                return Some(screenshot);
            }
        }
        None
    }

    pub fn cache_screenshot(&mut self, screenshot: WindowScreenshot) {
        if self.cache.len() >= self.max_cache_size {
            self.cleanup_old_cache();
        }
        
        self.cache.insert(screenshot.window_id.clone(), screenshot);
    }

    pub fn get_or_capture_screenshot(&mut self, window_id: &str) -> Result<WindowScreenshot, Box<dyn std::error::Error>> {
        if let Some(cached) = self.get_cached_screenshot(window_id) {
            return Ok(cached.clone());
        }
        
        let screenshot = self.capture_window_by_title(window_id)?;
        self.cache_screenshot(screenshot.clone());
        Ok(screenshot)
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

    pub fn update_screenshots_for_results(&mut self, results: &[SearchResult]) -> HashMap<(u32, u32), WindowScreenshot> {
        let mut updated_screenshots = HashMap::new();
        
        for result in results {
            if let Some(window_id) = result.window {
                let title = if result.description.is_empty() { 
                    &result.name 
                } else { 
                    &result.description 
                };
                
                match self.get_or_capture_screenshot(title) {
                    Ok(screenshot) => {
                        updated_screenshots.insert(window_id, screenshot);
                    }
                    Err(e) => {
                        tracing::debug!("Failed to capture screenshot for '{}': {}", title, e);
                    }
                }
            }
        }
        
        updated_screenshots
    }
}

impl Default for ScreenshotManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_cosmic_image_handle(screenshot: &WindowScreenshot) -> Result<cosmic::widget::image::Handle, Box<dyn std::error::Error>> {
    Ok(cosmic::widget::image::Handle::from_bytes(screenshot.image_data.clone()))
}