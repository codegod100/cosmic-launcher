use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct CosmicWindowManager {
    window_geometries: HashMap<String, WindowGeometry>,
}

impl CosmicWindowManager {
    pub fn new() -> Self {
        Self {
            window_geometries: HashMap::new(),
        }
    }

    pub fn get_window_geometry(&self, title: &str) -> Option<&WindowGeometry> {
        // Try exact match first
        if let Some(geometry) = self.window_geometries.get(title) {
            return Some(geometry);
        }

        // Try fuzzy matching
        for (window_title, geometry) in &self.window_geometries {
            if self.titles_match(title, window_title) {
                return Some(geometry);
            }
        }

        None
    }

    fn titles_match(&self, target: &str, window_title: &str) -> bool {
        let target_lower = target.to_lowercase();
        let window_lower = window_title.to_lowercase();
        
        // Various matching strategies
        window_lower.contains(&target_lower) ||
        target_lower.contains(&window_lower) ||
        self.app_specific_matches(&target_lower, &window_lower)
    }

    fn app_specific_matches(&self, target: &str, window_title: &str) -> bool {
        // App-specific matching logic
        (target.contains("discord") && window_title.contains("discord")) ||
        (target.contains("firefox") && (window_title.contains("firefox") || window_title.contains("mozilla"))) ||
        (target.contains("terminal") && window_title.contains("terminal")) ||
        (target.contains("files") && window_title.contains("files")) ||
        (target.contains("mattermost") && window_title.contains("mattermost"))
    }

    pub fn update_window_geometry(&mut self, title: String, x: i32, y: i32, width: u32, height: u32) {
        let geometry = WindowGeometry {
            x,
            y,
            width,
            height,
            title: title.clone(),
        };
        
        println!("📐 Updated window geometry for '{}': {}x{} at ({}, {})", title, width, height, x, y);
        self.window_geometries.insert(title, geometry);
    }

    pub fn list_known_windows(&self) -> Vec<&WindowGeometry> {
        self.window_geometries.values().collect()
    }

    pub fn clear(&mut self) {
        self.window_geometries.clear();
    }
}

impl Default for CosmicWindowManager {
    fn default() -> Self {
        Self::new()
    }
}