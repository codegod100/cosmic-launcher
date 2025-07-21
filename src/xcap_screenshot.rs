use std::collections::HashMap;
use xcap::Window;
use pop_launcher::SearchResult;

#[derive(Debug, Clone)]
pub struct XcapScreenshot {
    pub window_id: String,
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: std::time::Instant,
}

#[derive(Clone)]
pub struct XcapManager {
    cache: HashMap<String, XcapScreenshot>,
    max_cache_size: usize,
    cache_ttl: std::time::Duration,
    window_rotation_index: usize,
}

impl XcapManager {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            max_cache_size: 20,
            cache_ttl: std::time::Duration::from_secs(5),
            window_rotation_index: 0,
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

    pub fn capture_window_by_title(&mut self, title: &str) -> Result<XcapScreenshot, Box<dyn std::error::Error>> {
        println!("Trying to capture window with title: '{}'", title);
        
        // Quick platform check for first run only
        static PLATFORM_LOGGED: std::sync::Once = std::sync::Once::new();
        PLATFORM_LOGGED.call_once(|| {
            if std::env::var("WAYLAND_DISPLAY").is_ok() {
                println!("🔍 COSMIC Wayland session detected - window enumeration limited for security");
            }
        });
        
        let windows = match Window::all() {
            Ok(windows) => {
                println!("✓ Window::all() succeeded, found {} windows", windows.len());
                windows
            }
            Err(e) => {
                println!("✗ Window::all() failed: {}", e);
                return Err(format!("Failed to enumerate windows: {}", e).into());
            }
        };
        
        if windows.is_empty() {
            println!("⚠️  No windows detected at all - this suggests Wayland compositor restrictions");
            return Err("No windows available for enumeration".into());
        }
        
        // Simple window listing
        println!("Available windows: {}", windows.len());
        for (i, window) in windows.iter().enumerate() {
            let app_name = window.app_name();
            println!("  #{}: app='{}' ({}x{})", 
                i, app_name,
                window.capture_image().map(|img| img.width()).unwrap_or(0),
                window.capture_image().map(|img| img.height()).unwrap_or(0)
            );
        }
        
        // Try exact title match first
        for window in &windows {
            if window.is_minimized() {
                continue;
            }
            
            let window_title = window.title();
            
            if !window_title.is_empty() && window_title == title {
                println!("Found exact match for title: '{}'", title);
                return self.capture_window(&window, title);
            }
        }
        
        // Try partial title match (only if title is not empty)
        for window in &windows {
            if window.is_minimized() {
                continue;
            }
            
            let window_title = window.title();
            if !window_title.is_empty() && (window_title.contains(title) || title.contains(window_title)) {
                println!("Found partial match for title: '{}' -> '{}'", title, window_title);
                return self.capture_window(&window, title);
            }
        }
        
        // Try app name match with smart mapping
        for window in &windows {
            if window.is_minimized() {
                continue;
            }
            
            let app_name = window.app_name();
            
            // Smart app name matching for common applications
            let matches = if !app_name.is_empty() {
                // Direct app name match
                app_name.to_lowercase().contains(&title.to_lowercase()) ||
                title.to_lowercase().contains(&app_name.to_lowercase()) ||
                // Specific app mappings for common cases
                (app_name == "discord" && title.to_lowercase().contains("discord")) ||
                (app_name == "mattermost" && title.to_lowercase().contains("mattermost")) ||
                (app_name == "firefox" && title.to_lowercase().contains("firefox")) ||
                (app_name == "firefox" && title.to_lowercase().contains("mozilla")) ||
                // Add more mappings as needed
                false
            } else {
                false
            };
            
            if matches {
                println!("✓ Found app name match for title: '{}' -> '{}'", title, app_name);
                return self.capture_window(&window, title);
            }
        }
        
        // Fallback: rotate through available windows (COSMIC Wayland limitation)
        println!("⚙️  Using window rotation fallback (COSMIC only exposes {} windows)", windows.len());
        let non_minimized_windows: Vec<_> = windows.iter()
            .filter(|w| !w.is_minimized())
            .collect();
        
        if !non_minimized_windows.is_empty() {
            let window_index = self.window_rotation_index % non_minimized_windows.len();
            let window = non_minimized_windows[window_index];
            self.window_rotation_index += 1;
            println!("Rotating to window {} (index {} of {} available windows)", 
                self.window_rotation_index - 1, window_index, non_minimized_windows.len());
            return self.capture_window(&window, title);
        }
        
        Err(format!("No windows available for capture (all {} windows are minimized or inaccessible)", windows.len()).into())
    }

    pub fn capture_window_by_index(&mut self, index: usize) -> Result<XcapScreenshot, Box<dyn std::error::Error>> {
        let windows = Window::all()?;
        let non_minimized_windows: Vec<_> = windows.into_iter()
            .filter(|w| !w.is_minimized())
            .collect();
        
        if index >= non_minimized_windows.len() {
            return Err(format!("Window index {} out of range (have {} windows)", index, non_minimized_windows.len()).into());
        }

        let window = &non_minimized_windows[index];
        let window_id = format!("window_{}", index);
        self.capture_window(window, &window_id)
    }

    fn capture_window(&self, window: &Window, window_id: &str) -> Result<XcapScreenshot, Box<dyn std::error::Error>> {
        println!("Capturing window: '{}'", window.title());
        
        let image = window.capture_image()?;
        
        // Convert image to PNG bytes using tempfile
        let temp_file = tempfile::Builder::new().suffix(".png").tempfile()?;
        image.save(temp_file.path())?;
        let image_data = std::fs::read(temp_file.path())?;

        Ok(XcapScreenshot {
            window_id: window_id.to_string(),
            image_data,
            width: image.width(),
            height: image.height(),
            timestamp: std::time::Instant::now(),
        })
    }

    pub fn capture_all_windows(&mut self) -> Result<Vec<XcapScreenshot>, Box<dyn std::error::Error>> {
        let windows = Window::all()?;
        let mut screenshots = Vec::new();

        println!("Capturing all {} windows with xcap", windows.len());

        for (index, window) in windows.iter().enumerate() {
            if window.is_minimized() {
                println!("Skipping minimized window: {}", window.title());
                continue;
            }

            match self.capture_window(window, &format!("window_{}", index)) {
                Ok(screenshot) => {
                    screenshots.push(screenshot);
                    println!("Successfully captured window: {}", window.title());
                }
                Err(e) => {
                    println!("Failed to capture window {}: {}", window.title(), e);
                }
            }
        }

        Ok(screenshots)
    }

    pub fn get_window_count(&self) -> usize {
        Window::all()
            .map(|windows| windows.iter().filter(|w| !w.is_minimized()).count())
            .unwrap_or(0)
    }

    pub fn get_cached_screenshot(&self, window_id: &str) -> Option<&XcapScreenshot> {
        if let Some(screenshot) = self.cache.get(window_id) {
            if screenshot.timestamp.elapsed() <= self.cache_ttl {
                return Some(screenshot);
            }
        }
        None
    }

    pub fn cache_screenshot(&mut self, screenshot: XcapScreenshot) {
        if self.cache.len() >= self.max_cache_size {
            self.cleanup_old_cache();
        }
        
        self.cache.insert(screenshot.window_id.clone(), screenshot);
    }

    pub fn get_or_capture_screenshot_by_index(&mut self, window_id: &str, index: usize) -> Result<XcapScreenshot, Box<dyn std::error::Error>> {
        if let Some(cached) = self.get_cached_screenshot(window_id) {
            return Ok(cached.clone());
        }
        
        let screenshot = self.capture_window_by_index(index)?;
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

    pub fn update_screenshots_for_results(&mut self, results: &[SearchResult]) -> HashMap<(u32, u32), XcapScreenshot> {
        let mut updated_screenshots = HashMap::new();
        
        for result in results {
            if let Some(window_id) = result.window {
                let title = if result.description.is_empty() { 
                    &result.name 
                } else { 
                    &result.description 
                };
                
                match self.capture_window_by_title(title) {
                    Ok(screenshot) => {
                        updated_screenshots.insert(window_id, screenshot);
                        println!("Successfully captured screenshot for window: '{}'", title);
                    }
                    Err(e) => {
                        println!("Failed to capture screenshot for '{}': {}", title, e);
                    }
                }
            }
        }
        
        updated_screenshots
    }
}

impl Default for XcapManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_cosmic_image_handle(screenshot: &XcapScreenshot) -> Result<cosmic::widget::image::Handle, Box<dyn std::error::Error>> {
    Ok(cosmic::widget::image::Handle::from_bytes(screenshot.image_data.clone()))
}