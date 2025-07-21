use std::collections::HashMap;
use libwayshot::WayshotConnection;
use pop_launcher::SearchResult;
// Remove the format import, just use the method without explicit format

#[derive(Debug, Clone)]
pub struct WayshotScreenshot {
    pub window_id: String,
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: std::time::Instant,
}

pub struct WayshotManager {
    cache: HashMap<String, WayshotScreenshot>,
    max_cache_size: usize,
    cache_ttl: std::time::Duration,
    wayshot_conn: Option<WayshotConnection>,
}

impl WayshotManager {
    pub fn new() -> Self {
        let wayshot_conn = match WayshotConnection::new() {
            Ok(conn) => {
                println!("Wayshot connection established successfully");
                Some(conn)
            }
            Err(e) => {
                println!("Failed to establish wayshot connection: {}", e);
                None
            }
        };

        Self {
            cache: HashMap::new(),
            max_cache_size: 20,
            cache_ttl: std::time::Duration::from_secs(5),
            wayshot_conn,
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

    pub fn capture_window_by_index(&mut self, index: usize) -> Result<WayshotScreenshot, Box<dyn std::error::Error>> {
        let Some(ref mut wayshot) = self.wayshot_conn else {
            return Err("Wayshot connection not available".into());
        };

        // Get all outputs
        let outputs = wayshot.get_all_outputs();
        
        if index >= outputs.len() {
            return Err(format!("Window index {} out of range (have {} outputs)", index, outputs.len()).into());
        }

        let output = &outputs[index];
        println!("Capturing output: {}", output.name);

        // Capture the entire output - use the actual libwayshot API
        let image = wayshot.screenshot_single_output(output, false)?;
        
        // Convert image to PNG bytes by saving to a temp file and reading back
        let temp_file = tempfile::Builder::new().suffix(".png").tempfile()?;
        image.save(temp_file.path())?;
        let image_data = std::fs::read(temp_file.path())?;

        Ok(WayshotScreenshot {
            window_id: format!("output_{}", index),
            image_data,
            width: image.width(),
            height: image.height(),
            timestamp: std::time::Instant::now(),
        })
    }

    pub fn capture_all_windows(&mut self) -> Result<Vec<WayshotScreenshot>, Box<dyn std::error::Error>> {
        let Some(ref mut wayshot) = self.wayshot_conn else {
            return Err("Wayshot connection not available".into());
        };

        let outputs = wayshot.get_all_outputs();
        let mut screenshots = Vec::new();

        println!("Capturing {} outputs with wayshot", outputs.len());

        for (index, output) in outputs.iter().enumerate() {
            match wayshot.screenshot_single_output(output, false) {
                Ok(image) => {
                    let mut image_data: Vec<u8> = Vec::new();
                    let temp_file = tempfile::Builder::new().suffix(".png").tempfile();
                    if let Ok(temp_file) = temp_file {
                        if image.save(temp_file.path()).is_ok() {
                            if let Ok(file_data) = std::fs::read(temp_file.path()) {
                                image_data = file_data;
                        let screenshot = WayshotScreenshot {
                            window_id: format!("output_{}", index),
                            image_data,
                            width: image.width(),
                            height: image.height(),
                            timestamp: std::time::Instant::now(),
                        };
                                screenshots.push(screenshot);
                                println!("Successfully captured output: {}", output.name);
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("Failed to capture output {}: {}", output.name, e);
                }
            }
        }

        Ok(screenshots)
    }

    pub fn get_output_count(&self) -> usize {
        if let Some(ref wayshot) = self.wayshot_conn {
            wayshot.get_all_outputs().len()
        } else {
            0
        }
    }

    pub fn get_cached_screenshot(&self, window_id: &str) -> Option<&WayshotScreenshot> {
        if let Some(screenshot) = self.cache.get(window_id) {
            if screenshot.timestamp.elapsed() <= self.cache_ttl {
                return Some(screenshot);
            }
        }
        None
    }

    pub fn cache_screenshot(&mut self, screenshot: WayshotScreenshot) {
        if self.cache.len() >= self.max_cache_size {
            self.cleanup_old_cache();
        }
        
        self.cache.insert(screenshot.window_id.clone(), screenshot);
    }

    pub fn get_or_capture_screenshot_by_index(&mut self, window_id: &str, index: usize) -> Result<WayshotScreenshot, Box<dyn std::error::Error>> {
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

    pub fn update_screenshots_for_results(&mut self, results: &[SearchResult]) -> HashMap<(u32, u32), WayshotScreenshot> {
        let mut updated_screenshots = HashMap::new();
        
        for result in results {
            if let Some(window_id) = result.window {
                let title = if result.description.is_empty() { 
                    &result.name 
                } else { 
                    &result.description 
                };
                
                // For now, use index-based capture since wayshot works with outputs
                // In a real implementation, you'd map window IDs to outputs properly
                if let Ok(screenshot) = self.capture_window_by_index(0) {
                    updated_screenshots.insert(window_id, screenshot);
                }
            }
        }
        
        updated_screenshots
    }
}

impl Default for WayshotManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_cosmic_image_handle(screenshot: &WayshotScreenshot) -> Result<cosmic::widget::image::Handle, Box<dyn std::error::Error>> {
    Ok(cosmic::widget::image::Handle::from_bytes(screenshot.image_data.clone()))
}