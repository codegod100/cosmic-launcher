// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

//! Simple wayland subscription for cosmic-launcher
//! 
//! This provides basic toplevel tracking and screenshot capture functionality
//! for the Alt+Tab feature.

use cosmic::{
    cctk::{
        screencopy::{CaptureFrame, CaptureOptions, CaptureSession, CaptureSource, Capturer, FailureReason, 
                     Formats, Frame, ScreencopyFrameData, ScreencopyFrameDataExt, ScreencopyHandler,
                     ScreencopySessionData, ScreencopySessionDataExt, ScreencopyState},
        toplevel_info::{ToplevelInfo, ToplevelInfoHandler, ToplevelInfoState},
        wayland_client::{
            globals::registry_queue_init,
            protocol::{wl_output::WlOutput, wl_buffer, wl_shm, wl_shm_pool},
            Connection, QueueHandle, Dispatch, WEnum,
        },
        wayland_protocols::ext::{
            foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
            workspace::v1::client::ext_workspace_handle_v1::ExtWorkspaceHandleV1,
        },
        sctk::{
            registry::{ProvidesRegistryState, RegistryState},
            seat::{SeatHandler, SeatState},
            shm::{Shm, ShmHandler},
        },
    },
    iced::{self, stream, Subscription},
    iced_core::image::Bytes,
};
use futures::{
    channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender},
    SinkExt, StreamExt,
};
use image::EncodableLayout;
use once_cell::sync::Lazy;
use std::{
    fmt::Debug,
    os::fd::{AsFd, FromRawFd, RawFd},
    sync::{Arc, Condvar, Mutex, MutexGuard},
};
use tokio::sync::Mutex as TokioMutex;

pub static WAYLAND_RX: Lazy<TokioMutex<Option<UnboundedReceiver<WaylandUpdate>>>> =
    Lazy::new(|| TokioMutex::new(None));

#[derive(Debug, Clone)]
pub struct WaylandImage {
    pub img: Bytes,
    pub width: u32,
    pub height: u32,
}

impl WaylandImage {
    pub fn new(img: image::RgbaImage) -> Self {
        Self {
            // TODO avoid copy?
            img: Bytes::copy_from_slice(img.as_bytes()),
            width: img.width(),
            height: img.height(),
        }
    }
}

impl AsRef<[u8]> for WaylandImage {
    fn as_ref(&self) -> &[u8] {
        &self.img
    }
}

#[derive(Clone, Debug)]
pub enum WaylandUpdate {
    Init,
    Finished,
    Toplevel(ToplevelUpdate),
    Image(ExtForeignToplevelHandleV1, WaylandImage),
}

#[derive(Clone, Debug)]
pub enum ToplevelUpdate {
    Add(ToplevelInfo),
    Update(ToplevelInfo),
    Remove(ExtForeignToplevelHandleV1),
}

pub fn wayland_subscription() -> iced::Subscription<WaylandUpdate> {
    Subscription::run_with_id(
        std::any::TypeId::of::<WaylandUpdate>(),
        stream::channel(50, move |mut output| async move {
            let mut state = State::Waiting;

            loop {
                state = start_listening(state, &mut output).await;
            }
        }),
    )
}

pub enum State {
    Waiting,
    Finished,
}

async fn start_listening(
    state: State,
    output: &mut futures::channel::mpsc::Sender<WaylandUpdate>,
) -> State {
    match state {
        State::Waiting => {
            let mut guard = WAYLAND_RX.lock().await;
            let rx = {
                if guard.is_none() {
                    let (toplevel_tx, toplevel_rx) = unbounded();
                    let _ = std::thread::spawn(move || {
                        wayland_handler(toplevel_tx);
                    });
                    *guard = Some(toplevel_rx);
                    _ = output.send(WaylandUpdate::Init).await;
                }
                guard.as_mut().unwrap()
            };
            match rx.next().await {
                Some(u) => {
                    _ = output.send(u).await;
                    State::Waiting
                }
                None => {
                    _ = output.send(WaylandUpdate::Finished).await;
                    tracing::error!("Wayland handler thread died");
                    State::Finished
                }
            }
        }
        State::Finished => iced::futures::future::pending().await,
    }
}

struct AppData {
    exit: bool,
    tx: UnboundedSender<WaylandUpdate>,
    toplevel_info_state: ToplevelInfoState,
    registry_state: RegistryState,
    seat_state: SeatState,
    shm: Shm,
    screencopy_state: ScreencopyState,
    conn: Connection,
    qh: QueueHandle<Self>,
}

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    cosmic::cctk::sctk::registry_handlers!();
}

impl SeatHandler for AppData {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: cosmic::cctk::wayland_client::protocol::wl_seat::WlSeat,
        _capability: cosmic::cctk::sctk::seat::Capability,
    ) {
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: cosmic::cctk::wayland_client::protocol::wl_seat::WlSeat,
        _capability: cosmic::cctk::sctk::seat::Capability,
    ) {
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: cosmic::cctk::wayland_client::protocol::wl_seat::WlSeat) {}

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: cosmic::cctk::wayland_client::protocol::wl_seat::WlSeat) {}
}

impl ShmHandler for AppData {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ToplevelInfoHandler for AppData {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info_state
    }

    fn new_toplevel(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        toplevel: &ExtForeignToplevelHandleV1,
    ) {
        if let Some(info) = self.toplevel_info_state.info(toplevel) {
            let _ = self
                .tx
                .unbounded_send(WaylandUpdate::Toplevel(ToplevelUpdate::Add(info.clone())));
            
            // Trigger screenshot capture for new toplevel
            self.capture_toplevel_screenshot(toplevel.clone());
        }
    }

    fn update_toplevel(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        toplevel: &ExtForeignToplevelHandleV1,
    ) {
        if let Some(info) = self.toplevel_info_state.info(toplevel) {
            let _ = self
                .tx
                .unbounded_send(WaylandUpdate::Toplevel(ToplevelUpdate::Update(info.clone())));
        }
    }

    fn toplevel_closed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        toplevel: &ExtForeignToplevelHandleV1,
    ) {
        let _ = self
            .tx
            .unbounded_send(WaylandUpdate::Toplevel(ToplevelUpdate::Remove(toplevel.clone())));
    }
}

cosmic::cctk::sctk::delegate_seat!(AppData);
cosmic::cctk::sctk::delegate_registry!(AppData);
cosmic::cctk::sctk::delegate_shm!(AppData);
cosmic::cctk::delegate_toplevel_info!(AppData);
cosmic::cctk::delegate_screencopy!(AppData, session: [SessionData], frame: [FrameData]);

// Screenshot capture data structures
#[derive(Default)]
struct SessionInner {
    formats: Option<Formats>,
    res: Option<Result<(), WEnum<FailureReason>>>,
}

#[derive(Default)]
struct Session {
    condvar: Condvar,
    inner: Mutex<SessionInner>,
}

#[derive(Default)]
struct SessionData {
    session: Arc<Session>,
    session_data: ScreencopySessionData,
}

struct FrameData {
    frame_data: ScreencopyFrameData,
    session: CaptureSession,
}

impl Session {
    pub fn for_session(session: &CaptureSession) -> Option<&Self> {
        Some(&session.data::<SessionData>()?.session)
    }

    fn update<F: FnOnce(&mut SessionInner)>(&self, f: F) {
        f(&mut self.inner.lock().unwrap());
        self.condvar.notify_all();
    }

    fn wait_while<F: FnMut(&SessionInner) -> bool>(&self, mut f: F) -> MutexGuard<SessionInner> {
        self.condvar
            .wait_while(self.inner.lock().unwrap(), |data| f(data))
            .unwrap()
    }
}

impl ScreencopySessionDataExt for SessionData {
    fn screencopy_session_data(&self) -> &ScreencopySessionData {
        &self.session_data
    }
}

impl ScreencopyFrameDataExt for FrameData {
    fn screencopy_frame_data(&self) -> &ScreencopyFrameData {
        &self.frame_data
    }
}

impl ScreencopyHandler for AppData {
    fn screencopy_state(&mut self) -> &mut ScreencopyState {
        &mut self.screencopy_state
    }

    fn init_done(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        session: &CaptureSession,
        formats: &Formats,
    ) {
        Session::for_session(session).unwrap().update(|data| {
            data.formats = Some(formats.clone());
        });
    }

    fn ready(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        screencopy_frame: &CaptureFrame,
        _frame: Frame,
    ) {
        let session = &screencopy_frame.data::<FrameData>().unwrap().session;
        Session::for_session(session).unwrap().update(|data| {
            data.res = Some(Ok(()));
        });
    }

    fn failed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        screencopy_frame: &CaptureFrame,
        reason: WEnum<FailureReason>,
    ) {
        let session = &screencopy_frame.data::<FrameData>().unwrap().session;
        Session::for_session(session).unwrap().update(|data| {
            data.res = Some(Err(reason));
        });
    }

    fn stopped(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _session: &CaptureSession) {}
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for AppData {
    fn event(
        _app_data: &mut Self,
        _buffer: &wl_shm_pool::WlShmPool,
        _event: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for AppData {
    fn event(
        _app_data: &mut Self,
        _buffer: &wl_buffer::WlBuffer,
        _event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

struct CaptureData {
    qh: QueueHandle<AppData>,
    conn: Connection,
    wl_shm: wl_shm::WlShm,
    capturer: Capturer,
}

impl CaptureData {
    pub fn capture_source_shm_fd<Fd: AsFd>(
        &self,
        source: &ExtForeignToplevelHandleV1,
        fd: Fd,
        len: Option<u32>,
    ) -> Option<ShmImage<Fd>> {
        let session = Arc::new(Session::default());
        let capture_session = self
            .capturer
            .create_session(
                &CaptureSource::Toplevel(source.clone()),
                CaptureOptions::empty(),
                &self.qh,
                SessionData {
                    session: session.clone(),
                    session_data: Default::default(),
                },
            )
            .ok()?;
        
        self.conn.flush().ok()?;

        let formats = session
            .wait_while(|data| data.formats.is_none())
            .formats
            .take()?;
        let (width, height) = formats.buffer_size;

        if width == 0 || height == 0 {
            return None;
        }

        if !formats
            .shm_formats
            .contains(&wl_shm::Format::Abgr8888.into())
        {
            tracing::error!("No suitable buffer format found");
            tracing::warn!("Available formats: {:#?}", formats);
            return None;
        };

        let buf_len = width * height * 4;
        if let Some(len) = len {
            if len != buf_len {
                return None;
            }
        } else if rustix::fs::ftruncate(&fd, buf_len as _).is_err() {
            return None;
        }
        
        let pool = self
            .wl_shm
            .create_pool(fd.as_fd(), buf_len as i32, &self.qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            width as i32 * 4,
            wl_shm::Format::Abgr8888,
            &self.qh,
            (),
        );

        capture_session.capture(
            &buffer,
            &[],
            &self.qh,
            FrameData {
                frame_data: Default::default(),
                session: capture_session.clone(),
            },
        );
        self.conn.flush().ok()?;

        let res = session
            .wait_while(|data| data.res.is_none())
            .res
            .take()?;
        pool.destroy();
        buffer.destroy();

        if res.is_ok() {
            Some(ShmImage { fd, width, height })
        } else {
            None
        }
    }
}

pub struct ShmImage<T: AsFd> {
    fd: T,
    pub width: u32,
    pub height: u32,
}

impl<T: AsFd> ShmImage<T> {
    pub fn image(&self) -> Result<image::RgbaImage, Box<dyn std::error::Error + Send + Sync>> {
        let mmap = unsafe { memmap2::Mmap::map(&self.fd.as_fd())? };
        
        // Convert ABGR to RGBA
        let mut rgba_data = vec![0u8; (self.width * self.height * 4) as usize];
        for i in 0..(self.width * self.height) as usize {
            let base = i * 4;
            // ABGR -> RGBA
            rgba_data[base] = mmap[base + 2];     // R = B
            rgba_data[base + 1] = mmap[base + 1]; // G = G
            rgba_data[base + 2] = mmap[base];     // B = R
            rgba_data[base + 3] = mmap[base + 3]; // A = A
        }
        
        image::RgbaImage::from_raw(self.width, self.height, rgba_data)
            .ok_or_else(|| "ShmImage had incorrect size".into())
    }
}

impl AppData {
    fn capture_toplevel_screenshot(&self, handle: ExtForeignToplevelHandleV1) {
        let tx = self.tx.clone();
        let capture_data = CaptureData {
            qh: self.qh.clone(),
            conn: self.conn.clone(),
            wl_shm: self.shm.wl_shm().clone(),
            capturer: self.screencopy_state.capturer().clone(),
        };
        
        std::thread::spawn(move || {
            use std::ffi::CStr;
            let name = unsafe { CStr::from_bytes_with_nul_unchecked(b"cosmic-launcher-screenshot\0") };
            let Ok(fd) = rustix::fs::memfd_create(name, rustix::fs::MemfdFlags::CLOEXEC) else {
                tracing::error!("Failed to get fd for capture");
                return;
            };

            let img = capture_data.capture_source_shm_fd(&handle, fd, None);
            if let Some(img) = img {
                let Ok(mut img) = img.image() else {
                    tracing::error!("Failed to get RgbaImage");
                    return;
                };

                // Resize to 128x128 for thumbnail
                let max = img.width().max(img.height());
                let ratio = max as f32 / 128.0;

                if ratio > 1.0 {
                    let new_width = (img.width() as f32 / ratio).round();
                    let new_height = (img.height() as f32 / ratio).round();

                    img = image::imageops::resize(
                        &img,
                        new_width as u32,
                        new_height as u32,
                        image::imageops::FilterType::Lanczos3,
                    );
                }

                if let Err(err) =
                    tx.unbounded_send(WaylandUpdate::Image(handle, WaylandImage::new(img)))
                {
                    tracing::error!("Failed to send image event to subscription {err:?}");
                };
            } else {
                tracing::error!("Failed to capture image");
            }
        });
    }
}

fn wayland_handler(tx: UnboundedSender<WaylandUpdate>) {
    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();
    
    let registry_state = RegistryState::new(&globals);
    
    let mut app_data = AppData {
        exit: false,
        tx,
        toplevel_info_state: ToplevelInfoState::new(&registry_state, &qh),
        registry_state,
        seat_state: SeatState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).unwrap(),
        screencopy_state: ScreencopyState::new(&globals, &qh),
        conn,
        qh,
    };

    loop {
        if app_data.exit {
            break;
        }
        if let Err(e) = event_queue.blocking_dispatch(&mut app_data) {
            tracing::error!("Wayland event dispatch failed: {}", e);
            break;
        }
    }
}
