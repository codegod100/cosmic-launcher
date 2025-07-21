// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

pub mod buffer;
pub mod capture;
pub mod dmabuf;
pub mod gbm_devices;
pub mod screencopy;
pub mod toplevel;
pub mod workspace;

use cctk::{
    cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
    screencopy::{CaptureSource, ScreencopyState},
    sctk::{
        dmabuf::{DmabufFeedback, DmabufState},
        registry::{ProvidesRegistryState, RegistryState},
        seat::{SeatHandler, SeatState},
        shm::{Shm, ShmHandler},
    },
    toplevel_info::{ToplevelInfo as CctkToplevelInfo, ToplevelInfoState},
    toplevel_management::ToplevelManagerState,
    wayland_client::{
        globals::{registry_queue_init, GlobalListContents},
        protocol::{wl_output, wl_seat, wl_surface},
        Connection, Dispatch, QueueHandle,
    },
    workspace::{WorkspaceHandler, WorkspaceState},
};
use cosmic::{
    cctk,
    iced::{
        self,
        futures::{channel::mpsc, executor::block_on, SinkExt, StreamExt},
        Subscription,
    },
    iced_winit::platform_specific::wayland::subsurface_widget::SubsurfaceBuffer,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;

use super::{CaptureImage, Event, ToplevelInfo, Workspace};
use crate::config;

pub use buffer::Buffer;
pub use capture::Capture;
pub use screencopy::{ScreencopySession, SessionData};

// Buffers type alias for the collection
pub type Buffers = Vec<Buffer>;

// Re-export subscription function  
pub fn subscription(connection: Connection) -> Subscription<Event> {
    Subscription::run_with_id(
        0,
        cosmic::iced_futures::stream::channel(1, |mut output| async move {
            // This is a placeholder implementation - just send empty workspaces event
            let _res = output.send(Event::Workspaces(Vec::new())).await;
        }),
    )
}

pub struct AppData {
    registry_state: RegistryState,
    seat_state: SeatState,
    shm: Shm,
    dmabuf_state: Option<DmabufState>,
    dmabuf_feedback: Option<DmabufFeedback>,
    screencopy_state: Option<ScreencopyState>,
    toplevel_info_state: ToplevelInfoState,
    workspace_state: WorkspaceState,
    _toplevel_manager_state: ToplevelManagerState,
    buffers: Buffers,
    screenshot: Arc<Mutex<Option<SubsurfaceBuffer>>>,
    capture_sources: Vec<CaptureSource>,
}

impl AppData {
    fn new(globals: &GlobalListContents, qh: &QueueHandle<Self>) -> Self {
        let (globals_list, _) = registry_queue_init(&Connection::connect_to_env().unwrap()).unwrap();
        let registry_state = RegistryState::new(&globals_list);
        
        Self {
            seat_state: SeatState::new(&globals_list, qh),
            shm: Shm::bind(&globals_list, qh).unwrap(),
            dmabuf_state: Some(DmabufState::new(&globals_list, qh)),
            dmabuf_feedback: None,
            screencopy_state: Some(ScreencopyState::new(&globals_list, qh)),
            toplevel_info_state: ToplevelInfoState::new(&registry_state, qh),
            workspace_state: WorkspaceState::new(&registry_state, qh),
            _toplevel_manager_state: ToplevelManagerState::new(&registry_state, qh),
            buffers: Vec::new(),
            screenshot: Arc::new(Mutex::new(None)),
            capture_sources: Vec::new(),
            registry_state,
        }
    }

    fn send_event(&self, event: Event) {
        // Placeholder - in a real implementation this would send via a channel
    }

    fn add_capture_source(&mut self, source: CaptureSource) {
        self.capture_sources.push(source);
    }
}

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    cctk::sctk::registry_handlers!();
}

impl SeatHandler for AppData {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        _capability: cctk::sctk::seat::Capability,
    ) {
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        _capability: cctk::sctk::seat::Capability,
    ) {
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}
}

impl ShmHandler for AppData {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

cctk::sctk::delegate_shm!(AppData);
cctk::sctk::delegate_seat!(AppData);
cctk::sctk::delegate_registry!(AppData);
