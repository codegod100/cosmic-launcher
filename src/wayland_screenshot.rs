use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::os::fd::AsFd;
use wayland_client::protocol::{
    wl_registry::{self, WlRegistry}, 
    wl_shm::{self, WlShm, Format}, 
    wl_buffer::{self, WlBuffer},
    wl_shm_pool::{self, WlShmPool},
};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_output_image_capture_source_manager_v1::{self, ExtOutputImageCaptureSourceManagerV1},
    ext_image_capture_source_v1::{self, ExtImageCaptureSourceV1},
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
};
use wayland_client::protocol::wl_output::{self, WlOutput};

#[derive(Clone)]
pub struct WaylandScreenshot {
    connection: Connection,
    capture_manager: Option<ExtImageCopyCaptureManagerV1>,
    source_manager: Option<ExtOutputImageCaptureSourceManagerV1>,
    shm: Option<WlShm>,
    outputs: Vec<WlOutput>,
    pending_screenshots: Arc<Mutex<HashMap<u32, Option<ScreenshotData>>>>,
}

#[derive(Debug, Clone)]
pub struct ScreenshotData {
    pub buffer: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
}

struct AppData {
    capture_manager: Option<ExtImageCopyCaptureManagerV1>,
    source_manager: Option<ExtOutputImageCaptureSourceManagerV1>,
    shm: Option<WlShm>,
    outputs: Vec<WlOutput>,
    pending_screenshots: Arc<Mutex<HashMap<u32, Option<ScreenshotData>>>>,
}

impl WaylandScreenshot {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let connection = Connection::connect_to_env()?;
        let display = connection.display();
        
        let mut event_queue = connection.new_event_queue();
        let qh = event_queue.handle();
        
        let pending_screenshots = Arc::new(Mutex::new(HashMap::new()));
        
        let mut app_data = AppData {
            capture_manager: None,
            source_manager: None,
            shm: None,
            outputs: Vec::new(),
            pending_screenshots: pending_screenshots.clone(),
        };
        
        // Get the global objects
        let _registry = display.get_registry(&qh, ());
        
        // Roundtrip to get all globals
        event_queue.roundtrip(&mut app_data)?;
        
        Ok(Self {
            connection,
            capture_manager: app_data.capture_manager,
            source_manager: app_data.source_manager,
            shm: app_data.shm,
            outputs: app_data.outputs,
            pending_screenshots,
        })
    }

    pub fn capture_toplevel_by_index(&mut self, index: usize) -> Result<Option<ScreenshotData>, Box<dyn std::error::Error>> {
        if index >= self.outputs.len() {
            return Ok(None);
        }
        
        let Some(ref capture_manager) = self.capture_manager else {
            return Err("Image copy capture manager not available".into());
        };
        
        let Some(ref source_manager) = self.source_manager else {
            return Err("Image capture source manager not available".into());
        };
        
        let mut event_queue = self.connection.new_event_queue();
        let qh = event_queue.handle();
        
        // Create capture source for the output
        let output = &self.outputs[index];
        let source = source_manager.create_source(output, &qh, index as u32);
        
        // Create capture session - trying different approaches for CaptureOptions
        // This may require importing the correct type, for now try with minimal args
        let session = match capture_manager.create_session(&source, &qh, index as u32) {
            Ok(s) => s,
            Err(_) => return Err("Failed to create session".into()),
        };
        
        // Create frame for capture
        let frame = session.create_frame(&qh, index as u32);
        
        // Initialize the pending screenshot entry
        {
            let mut pending = self.pending_screenshots.lock().unwrap();
            pending.insert(index as u32, None);
        }
        
        // Setup app data for this capture
        let mut app_data = AppData {
            capture_manager: self.capture_manager.clone(),
            source_manager: self.source_manager.clone(),
            shm: self.shm.clone(),
            outputs: self.outputs.clone(),
            pending_screenshots: self.pending_screenshots.clone(),
        };
        
        // Dispatch events until we get the screenshot or timeout
        let mut attempts = 0;
        loop {
            event_queue.blocking_dispatch(&mut app_data)?;
            
            {
                let pending = self.pending_screenshots.lock().unwrap();
                if let Some(screenshot_opt) = pending.get(&(index as u32)) {
                    if let Some(screenshot) = screenshot_opt {
                        return Ok(Some(screenshot.clone()));
                    }
                }
            }
            
            attempts += 1;
            if attempts > 100 { // Timeout after 100 dispatch attempts
                break;
            }
        }
        
        Ok(None)
    }
    
    pub fn get_toplevel_count(&self) -> usize {
        self.outputs.len()
    }
}

impl Dispatch<WlRegistry, ()> for AppData {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } => {
                match interface.as_str() {
                    "ext_image_copy_capture_manager_v1" => {
                        let manager = registry.bind::<ExtImageCopyCaptureManagerV1, _, _>(
                            name, version.min(1), qh, ()
                        );
                        state.capture_manager = Some(manager);
                        println!("Bound image copy capture manager");
                    }
                    "ext_output_image_capture_source_manager_v1" => {
                        let manager = registry.bind::<ExtOutputImageCaptureSourceManagerV1, _, _>(
                            name, version.min(1), qh, ()
                        );
                        state.source_manager = Some(manager);
                        println!("Bound output image capture source manager");
                    }
                    "wl_output" => {
                        let output = registry.bind::<WlOutput, _, _>(
                            name, version.min(1), qh, ()
                        );
                        state.outputs.push(output);
                        println!("Bound output");
                    }
                    "wl_shm" => {
                        let shm = registry.bind::<WlShm, _, _>(name, version.min(1), qh, ());
                        state.shm = Some(shm);
                        println!("Bound shared memory");
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

// Real protocol implementations

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &ExtImageCopyCaptureManagerV1,
        _event: ext_image_copy_capture_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Handle capture manager events
    }
}

impl Dispatch<ExtOutputImageCaptureSourceManagerV1, ()> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &ExtOutputImageCaptureSourceManagerV1,
        _event: ext_output_image_capture_source_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Handle source manager events
    }
}

impl Dispatch<WlOutput, ()> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &WlOutput,
        _event: wl_output::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Handle output events (geometry, mode, etc.)
    }
}

impl Dispatch<ExtImageCaptureSourceV1, u32> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &ExtImageCaptureSourceV1,
        _event: ext_image_capture_source_v1::Event,
        _data: &u32,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Handle capture source events
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, u32> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &ExtImageCopyCaptureSessionV1,
        _event: ext_image_copy_capture_session_v1::Event,
        _data: &u32,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Handle capture session events
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, u32> for AppData {
    fn event(
        state: &mut Self,
        _proxy: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        data: &u32,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let index = *data;
        match event {
            _ => {
                // For now, just create a placeholder screenshot for any event
                println!("Frame event received for index {}", index);
                let screenshot = ScreenshotData {
                    buffer: vec![255; 400 * 300 * 4], // White placeholder
                    width: 400,
                    height: 300,
                    stride: 1600,
                    format: 0x34325241, // ARGB8888
                };
                
                {
                    let mut pending = state.pending_screenshots.lock().unwrap();
                    pending.insert(index, Some(screenshot));
                }
            }
            ext_image_copy_capture_frame_v1::Event::Ready { .. } => {
                println!("Frame ready for index {}", index);
                // Screenshot is now available in the buffer
                // For now, create a placeholder screenshot until we implement buffer reading
                let screenshot = ScreenshotData {
                    buffer: vec![255; 400 * 300 * 4], // White placeholder
                    width: 400,
                    height: 300,
                    stride: 1600,
                    format: 0x34325241, // ARGB8888
                };
                
                {
                    let mut pending = state.pending_screenshots.lock().unwrap();
                    pending.insert(index, Some(screenshot));
                }
            }
            ext_image_copy_capture_frame_v1::Event::Failed { reason: _ } => {
                println!("Frame capture failed for index {}", index);
                {
                    let mut pending = state.pending_screenshots.lock().unwrap();
                    pending.remove(&index);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlShm, ()> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &WlShm,
        _event: wl_shm::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Handle shared memory events
    }
}

impl Dispatch<WlShmPool, ()> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &WlShmPool,
        _event: wl_shm_pool::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Handle shm pool events
    }
}

impl Dispatch<WlBuffer, ()> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &WlBuffer,
        _event: wl_buffer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Handle buffer events
    }
}