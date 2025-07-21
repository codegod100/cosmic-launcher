use std::sync::{Arc, Mutex};
use wayland_client::{
    protocol::{wl_registry, wl_display::WlDisplay},
    Connection, Dispatch, QueueHandle, EventQueue,
};
use cosmic_protocols::toplevel_info::v1::client::{
    zcosmic_toplevel_info_v1::{self, ZcosmicToplevelInfoV1},
    zcosmic_toplevel_handle_v1::{self, ZcosmicToplevelHandleV1, Event as ToplevelEvent},
};
use crate::cosmic_window_info::CosmicWindowManager;

pub struct CosmicToplevelProtocol {
    window_manager: Arc<Mutex<CosmicWindowManager>>,
    connection: Option<Connection>,
    event_queue: Option<EventQueue<AppData>>,
}

#[derive(Debug)]
pub struct AppData {
    window_manager: Arc<Mutex<CosmicWindowManager>>,
    toplevel_info: Option<ZcosmicToplevelInfoV1>,
}

impl CosmicToplevelProtocol {
    pub fn new(window_manager: Arc<Mutex<CosmicWindowManager>>) -> Self {
        Self {
            window_manager,
            connection: None,
            event_queue: None,
        }
    }

    pub fn connect(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("🔗 Connecting to COSMIC toplevel info protocol...");
        
        let connection = Connection::connect_to_env()?;
        let display = connection.display();
        
        let event_queue = connection.new_event_queue();
        let qh = event_queue.handle();
        
        let app_data = AppData {
            window_manager: Arc::clone(&self.window_manager),
            toplevel_info: None,
        };
        
        // Get the registry and bind to cosmic toplevel info
        let _registry = display.get_registry(&qh, app_data);
        
        self.connection = Some(connection);
        self.event_queue = Some(event_queue);
        
        println!("✅ Connected to Wayland, waiting for cosmic-toplevel-info...");
        Ok(())
    }

    pub fn process_events(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(ref mut event_queue) = self.event_queue {
            event_queue.blocking_dispatch(&mut AppData {
                window_manager: Arc::clone(&self.window_manager),
                toplevel_info: None,
            })?;
        }
        Ok(())
    }

    pub fn run_event_loop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(ref mut event_queue) = self.event_queue {
            loop {
                event_queue.blocking_dispatch(&mut AppData {
                    window_manager: Arc::clone(&self.window_manager),
                    toplevel_info: None,
                })?;
            }
        }
        Ok(())
    }
}

// Implement Dispatch for the registry to bind to cosmic toplevel info
impl Dispatch<wl_registry::WlRegistry, AppData> for AppData {
    fn event(
        state: &mut AppData,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &AppData,
        _: &Connection,
        qh: &QueueHandle<AppData>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } => {
                println!("🌐 Found global: {} v{} ({})", interface, version, name);
                
                if interface == "zcosmic_toplevel_info_v1" {
                    println!("🎯 Binding to cosmic-toplevel-info-v1...");
                    let toplevel_info = registry.bind::<ZcosmicToplevelInfoV1, _, _>(
                        name,
                        version.min(2), // Version 2 supports geometry events
                        qh,
                        state.clone(),
                    );
                    state.toplevel_info = Some(toplevel_info);
                }
            }
            wl_registry::Event::GlobalRemove { name } => {
                println!("🗑️  Global removed: {}", name);
            }
            _ => {}
        }
    }
}

// Implement Dispatch for cosmic toplevel info
impl Dispatch<ZcosmicToplevelInfoV1, AppData> for AppData {
    fn event(
        state: &mut AppData,
        _: &ZcosmicToplevelInfoV1,
        event: zcosmic_toplevel_info_v1::Event,
        _: &AppData,
        _: &Connection,
        _qh: &QueueHandle<AppData>,
    ) {
        match event {
            zcosmic_toplevel_info_v1::Event::Toplevel { toplevel } => {
                println!("📱 New toplevel created: {:?}", toplevel);
            }
            zcosmic_toplevel_info_v1::Event::Finished => {
                println!("🏁 Toplevel info finished");
            }
            _ => {}
        }
    }
}

// Implement Dispatch for toplevel handles to get geometry
impl Dispatch<ZcosmicToplevelHandleV1, AppData> for AppData {
    fn event(
        state: &mut AppData,
        _: &ZcosmicToplevelHandleV1,
        event: ToplevelEvent,
        _: &AppData,
        _: &Connection,
        _qh: &QueueHandle<AppData>,
    ) {
        match event {
            ToplevelEvent::Title { title } => {
                println!("📝 Toplevel title: {}", title);
            }
            ToplevelEvent::AppId { app_id } => {
                println!("🆔 Toplevel app_id: {}", app_id);
            }
            ToplevelEvent::Geometry { x, y, width, height, output: _ } => {
                if let Ok(mut window_manager) = state.window_manager.lock() {
                    // We need to associate this geometry with a title/app_id
                    // For now, use a placeholder - in a real implementation we'd track
                    // which handle corresponds to which title
                    let title = format!("window_{}_{}", x, y); // Temporary identifier
                    
                    println!("📐 Toplevel geometry: {}x{} at ({}, {})", width, height, x, y);
                    window_manager.update_window_geometry(title, x, y, width as u32, height as u32);
                }
            }
            ToplevelEvent::State { state: _ } => {
                // Window state changed (minimized, maximized, etc.)
            }
            ToplevelEvent::Done => {
                // All properties for this toplevel have been sent
            }
            ToplevelEvent::Closed => {
                println!("❌ Toplevel closed");
            }
            _ => {}
        }
    }
}

impl Clone for AppData {
    fn clone(&self) -> Self {
        Self {
            window_manager: Arc::clone(&self.window_manager),
            toplevel_info: self.toplevel_info.clone(),
        }
    }
}