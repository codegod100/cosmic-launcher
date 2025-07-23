use crate::{app::iced::event::listen_raw, subscriptions::launcher};
use crate::wayland_subscription::{WaylandUpdate, ToplevelUpdate, WaylandImage, wayland_subscription};
use cosmic::cctk::toplevel_info::ToplevelInfo;
use cosmic::cctk::wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use clap::Parser;
use cosmic::app::{Core, CosmicFlags, Settings, Task};
use cosmic::dbus_activation::Details;
use cosmic::cctk::sctk;
use cosmic::iced::alignment::Alignment;

use cosmic::iced::event::wayland::OverlapNotifyEvent;
use cosmic::iced::id::Id;
use cosmic::iced::platform_specific::runtime::wayland::{
    layer_surface::SctkLayerSurfaceSettings,
    popup::{SctkPopupSettings, SctkPositioner},
};
use cosmic::iced::platform_specific::shell::commands::{
    self,
    activation::request_token,
    layer_surface::{Anchor, KeyboardInteractivity, destroy_layer_surface, get_layer_surface},
};
use cosmic::iced::widget::{column, container, image::{Handle, Image}};
use cosmic::iced::{self, Length, Size, Subscription};
use cosmic::iced_core::keyboard::key::Named;
use cosmic::iced_core::widget::operation;
use cosmic::iced_core::{Point, Rectangle, window};
use cosmic::iced_runtime::core::event::wayland::LayerEvent;
use cosmic::iced_runtime::core::event::{PlatformSpecific, wayland};
use cosmic::iced_runtime::core::layout::Limits;
use cosmic::iced_runtime::core::window::{Event as WindowEvent, Id as SurfaceId};
use cosmic::iced_widget::row;
use cosmic::iced_widget::scrollable::RelativeOffset;
use cosmic::iced_winit::commands::overlap_notify::overlap_notify;
use cosmic::widget::icon;
use cosmic::widget::{
    mouse_area, text,
    text_input,
};
use cosmic::iced::widget::text::Wrapping;
use cosmic::{Element, keyboard_nav};
use cosmic::iced_runtime;
use iced::keyboard::Key;
use pop_launcher::{ContextOption, GpuPreference, IconSource, SearchResult};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::sync::LazyLock;
use std::{
    collections::{HashMap, VecDeque},
    str::FromStr,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

static INPUT_ID: LazyLock<Id> = LazyLock::new(|| Id::new("input_id"));
static SCROLLABLE: LazyLock<Id> = LazyLock::new(|| Id::new("scrollable"));

pub(crate) static MENU_ID: LazyLock<SurfaceId> = LazyLock::new(SurfaceId::unique);

#[derive(Parser, Debug, Serialize, Deserialize, Clone)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Args {
    #[clap(subcommand)]
    pub subcommand: Option<LauncherTasks>,
}

#[derive(Debug, Serialize, Deserialize, Clone, clap::Subcommand)]
pub enum LauncherTasks {
    #[clap(about = "Toggle the launcher and switch to the alt-tab view")]
    AltTab,
    #[clap(about = "Toggle the launcher and switch to the alt-tab view")]
    ShiftAltTab,
}

impl Display for LauncherTasks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", serde_json::ser::to_string(self).unwrap())
    }
}

impl FromStr for LauncherTasks {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::de::from_str(s)
    }
}

impl CosmicFlags for Args {
    type SubCommand = LauncherTasks;
    type Args = Vec<String>;

    fn action(&self) -> Option<&LauncherTasks> {
        self.subcommand.as_ref()
    }
}

pub fn run() -> cosmic::iced::Result {
    let args = Args::parse();
    cosmic::app::run_single_instance::<CosmicLauncher>(
        Settings::default()
            .antialiasing(true)
            .client_decorations(true)
            .debug(false)
            .default_text_size(16.0)
            .scale_factor(1.0)
            .no_main_window(true)
            .exit_on_close(false),
        args,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceState {
    Visible,
    Hidden,
    WaitingToBeShown,
}



pub struct CosmicLauncher {
    core: Core,
    input_value: String,
    surface_state: SurfaceState,
    launcher_items: Vec<SearchResult>,
    tx: Option<mpsc::Sender<launcher::Request>>,
    menu: Option<(u32, Vec<ContextOption>)>,
    cursor_position: Option<Point<f32>>,
    focused: usize,
    last_hide: Instant,
    alt_tab_mode: bool, // Track if we're in Alt+Tab mode
    super_launcher_mode: bool, // Track if we're in Super key launcher mode (Alt+Tab list with search)
    window_id: window::Id,
    queue: VecDeque<Message>,
    result_ids: Vec<Id>,
    overlap: HashMap<String, Rectangle>,
    margin: f32,
    height: f32,
    needs_clear: bool,
    search_debounce_timer: Option<Instant>, // Timer for debounced search

    toplevel_captures: HashMap<ExtForeignToplevelHandleV1, WaylandImage>,
    screenshot_cache_time: HashMap<ExtForeignToplevelHandleV1, Instant>,
    toplevels: Vec<ToplevelInfo>,
    active: Option<usize>, // For Alt+Tab selected window index
    #[allow(dead_code)]
    backend_event_receiver: Option<mpsc::UnboundedReceiver<WaylandUpdate>>,
}

#[derive(Debug, Clone)]
pub enum Message {
    InputChanged(String),
    Backspace,
    CompleteFocusedId(Id),
    Activate(Option<usize>),
    CursorMoved(Point<f32>),
    Hide,
    LauncherEvent(launcher::Event),
    Layer(LayerEvent),
    KeyboardNav(keyboard_nav::Action),
    ActivationToken(Option<String>, String, String, GpuPreference, bool),
    AltTab,
    ShiftAltTab,
    Opened(Size, window::Id),
    AltRelease,
    SuperRelease,
    Overlap(OverlapNotifyEvent),

    BackendEvent(WaylandUpdate),
    DebouncedSearch(String), // For debounced search after delay
}

impl CosmicLauncher {
    fn set_mode(&mut self, alt_tab: bool, super_launcher: bool) {
        if alt_tab && super_launcher {
            panic!("Cannot have both alt_tab_mode and super_launcher_mode active simultaneously");
        }
        println!("DEBUG: Mode set - alt_tab: {}, super_launcher: {} (previous: alt_tab={}, super={})", 
                 alt_tab, super_launcher, self.alt_tab_mode, self.super_launcher_mode);
        self.alt_tab_mode = alt_tab;
        self.super_launcher_mode = super_launcher;
    }

    fn is_screenshot_cache_fresh(&self, handle: &ExtForeignToplevelHandleV1) -> bool {
        const CACHE_DURATION_MS: u128 = 2000; // Cache screenshots for 2 seconds
        
        if let Some(cache_time) = self.screenshot_cache_time.get(handle) {
            let age = cache_time.elapsed().as_millis();
            age < CACHE_DURATION_MS
        } else {
            false
        }
    }

    fn populate_from_cached_toplevels(&mut self) {
        // Immediately populate launcher_items from cached toplevels for Alt+Tab
        println!("DEBUG: Populating {} toplevels from cache", self.toplevels.len());
        
        self.launcher_items = self.toplevels.iter().enumerate().map(|(idx, toplevel)| {
            SearchResult {
                id: idx as u32, // Use index as simple ID
                name: if !toplevel.title.is_empty() { toplevel.title.clone() } else if !toplevel.app_id.is_empty() { toplevel.app_id.clone() } else { "Unknown".to_string() },
                description: toplevel.app_id.clone(),
                icon: None, // Will be determined in UI based on app_id
                category_icon: None,
                window: None, // We'll match screenshots by name/title instead
            }
        }).collect();
        
        println!("DEBUG: Populated {} launcher items from toplevels", self.launcher_items.len());
    }

    fn request(&self, r: launcher::Request) {
        debug!("request: {:?}", r);
        if let Some(tx) = &self.tx {
            info!("Sending request to pop-launcher: {:?}", r);
            if let Err(e) = tx.blocking_send(r) {
                error!("Failed to send request to pop-launcher: {e}");
            } else {
                info!("Request sent successfully to pop-launcher");
            }
        } else {
            error!("tx not found - pop-launcher service not connected!");
        }
    }

    fn show(&mut self) -> Task<Message> {
        self.surface_state = SurfaceState::Visible;
        self.needs_clear = true;

        let mut tasks = vec![
            get_layer_surface(SctkLayerSurfaceSettings {
                id: self.window_id,
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                anchor: Anchor::TOP,
                namespace: "launcher".into(),
                size: Some((Some(1400), Some(1600))), // Adjusted width for smaller screenshots
                size_limits: Limits::NONE.min_width(1400.0).min_height(1600.0).max_width(1400.0).max_height(1600.0),
                exclusive_zone: -1,
                ..Default::default()
            }),
            overlap_notify(self.window_id, true),
        ];

        // Focus search input when showing in super launcher mode - delay it slightly
        if self.super_launcher_mode {
            tasks.push(Task::perform(
                async { tokio::time::sleep(tokio::time::Duration::from_millis(50)).await; },
                |_| cosmic::Action::App(Message::CompleteFocusedId(INPUT_ID.clone())),
            ));
        }

        Task::batch(tasks)
    }


    fn hide(&mut self) -> Task<Message> {
        println!("DEBUG: hide() called - resetting state");
        self.input_value.clear();
        self.focused = 0;
        self.active = None;
        self.set_mode(false, false); // Reset all modes
        self.search_debounce_timer = None; // Clear search debounce timer
        self.queue.clear();

        self.request(launcher::Request::Close);

        let mut tasks = Vec::new();

        if self.surface_state == SurfaceState::Visible {
            println!("DEBUG: Destroying layer surface");
            tasks.push(destroy_layer_surface(self.window_id));
            if self.menu.take().is_some() {
                tasks.push(commands::popup::destroy_popup(*MENU_ID));
            }
        }

        self.surface_state = SurfaceState::Hidden;
        println!("DEBUG: hide() complete - surface_state={:?}", self.surface_state);

        Task::batch(tasks)
    }

    fn focus_next(&mut self) {
        if self.launcher_items.is_empty() {
            return;
        }
        self.focused = (self.focused + 1) % self.launcher_items.len();
    }

    fn focus_previous(&mut self) {
        if self.launcher_items.is_empty() {
            return;
        }
        self.focused = (self.focused + self.launcher_items.len() - 1) % self.launcher_items.len();
    }

    fn handle_overlap(&mut self) {
        if matches!(self.surface_state, SurfaceState::Hidden) {
            return;
        }
        let mid_height = self.height / 2.;
        self.margin = 0.;

        for o in self.overlap.values() {
            if self.margin + mid_height < o.y
                || self.margin > o.y + o.height
                || mid_height < o.y + o.height / 2.0
            {
                continue;
            }
            self.margin = o.y + o.height;
        }
    }

    fn find_screenshot_for_item(&self, item: &SearchResult) -> Option<&WaylandImage> {
        info!("Looking for screenshot for item: '{}' (window: {:?})", item.name, item.window.is_some());
        
        // If this launcher item represents a window, try to find matching screenshot
        if item.window.is_some() {
            // Try to match by window title/name with toplevels
            for (handle, capture_image) in self.toplevel_captures.iter() {
                // Find corresponding toplevel info
                if let Some(toplevel_info) = self.toplevels.iter().find(|t| t.foreign_toplevel == *handle) {
                    // Match by title (item.description often contains the window title for windows)
                    if item.description.contains(&toplevel_info.title) 
                        || toplevel_info.title.contains(&item.description)
                        || item.name.contains(&toplevel_info.title)
                        || toplevel_info.title.contains(&item.name) {
                        info!("Match found! Using screenshot for: {}", item.name);
                        return Some(capture_image);
                    }
                }
            }
        }
        info!("No screenshot found for item: {}", item.name);
        None
    }

    fn handle_toplevel_update(&mut self, toplevel_update: ToplevelUpdate) {
        match toplevel_update {
            ToplevelUpdate::Add(info) => {
                info!("New toplevel - title: '{}'", info.title);
                self.toplevels.push(info);
            }
            ToplevelUpdate::Update(info) => {
                info!("Update toplevel - title: '{}'", info.title);
                if let Some(t) = self
                    .toplevels
                    .iter_mut()
                    .find(|t| t.foreign_toplevel == info.foreign_toplevel)
                {
                    *t = info;
                }
            }
            ToplevelUpdate::Remove(handle) => {
                info!("Close toplevel - handle: {:?}", handle);
                self.toplevels.retain(|t| t.foreign_toplevel != handle);
            }
        }
    }
}

async fn launch(
    token: Option<String>,
    app_id: String,
    exec: String,
    gpu: GpuPreference,
    terminal: bool,
) {
    let mut envs = Vec::new();
    if let Some(token) = token {
        envs.push(("XDG_ACTIVATION_TOKEN".to_string(), token.clone()));
        envs.push(("DESKTOP_STARTUP_ID".to_string(), token));
    }

    if let Some(gpu_envs) = try_get_gpu_envs(gpu).await {
        envs.extend(gpu_envs);
    }

    cosmic::desktop::spawn_desktop_exec(exec, envs, Some(&app_id), terminal).await;
}

async fn try_get_gpu_envs(gpu: GpuPreference) -> Option<HashMap<String, String>> {
    let connection = zbus::Connection::system().await.ok()?;
    let proxy = switcheroo_control::SwitcherooControlProxy::new(&connection)
        .await
        .ok()?;
    let gpus = proxy.get_gpus().await.ok()?;
    match gpu {
        GpuPreference::Default => gpus.into_iter().find(|gpu| gpu.default),
        GpuPreference::NonDefault => gpus.into_iter().find(|gpu| !gpu.default),
        GpuPreference::SpecificIdx(idx) => gpus.into_iter().nth(idx as usize),
    }
    .map(|gpu| gpu.environment)
}

impl cosmic::Application for CosmicLauncher {
    type Message = Message;
    type Executor = cosmic::executor::single::Executor;
    type Flags = Args;
    const APP_ID: &'static str = "com.system76.CosmicLauncher";

         fn init(mut core: Core, _flags: Args) -> (Self, Task<Message>) {
        core.set_keyboard_nav(false);

        // Create backend subscription 
        let _conn = wayland_client::Connection::connect_to_env()
            .expect("Failed to connect to Wayland display");

        (
            CosmicLauncher {
                core,
                input_value: String::new(),
                surface_state: SurfaceState::Hidden,
                launcher_items: Vec::new(),
                tx: None,
                menu: None,
                cursor_position: None,
                focused: 0,
                last_hide: Instant::now(),
                alt_tab_mode: false,
                super_launcher_mode: false,
                window_id: window::Id::unique(),
                queue: VecDeque::new(),
                result_ids: (0..10)
                    .map(|id| Id::new(id.to_string()))
                    .collect::<Vec<_>>(),
                margin: 0.,
                overlap: HashMap::new(),
                height: 100.,
                needs_clear: false,

                toplevel_captures: HashMap::new(),
                screenshot_cache_time: HashMap::new(),
                toplevels: Vec::new(),
                active: None,
                backend_event_receiver: None,
                search_debounce_timer: None,
            },
            Task::none(),
        )
    }

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    #[allow(clippy::too_many_lines)]
    fn update(&mut self, message: Message) -> Task<Self::Message> {
        match message {
            Message::InputChanged(value) => {
                // Always update input value immediately for responsive UI
                self.input_value.clone_from(&value);
                
                // Use minimal debounce for responsive search
                // For short queries (1-2 chars), search immediately
                // For longer queries, use minimal debounce to avoid excessive requests
                let debounce_ms = if value.len() <= 2 { 50 } else { 100 };
                
                self.search_debounce_timer = Some(Instant::now());
                return Task::perform(
                    async move {
                        tokio::time::sleep(tokio::time::Duration::from_millis(debounce_ms)).await;
                        value
                    },
                    |search_term| cosmic::Action::App(Message::DebouncedSearch(search_term)),
                );
            }
            Message::Backspace => {
                // Always update input value immediately for responsive UI
                self.input_value.pop();
                
                // Use minimal debounce for responsive search
                let input_len = self.input_value.len();
                let debounce_ms = if input_len <= 2 { 50 } else { 100 };
                
                self.search_debounce_timer = Some(Instant::now());
                let value = self.input_value.clone();
                return Task::perform(
                    async move {
                        tokio::time::sleep(tokio::time::Duration::from_millis(debounce_ms)).await;
                        value
                    },
                    |search_term| cosmic::Action::App(Message::DebouncedSearch(search_term)),
                );
            }
            Message::CompleteFocusedId(id) => {
                // Handle both search activation and focus requests
                if id == INPUT_ID.clone() {
                    // This is a focus request for the input field
                    return cosmic::widget::text_input::focus(INPUT_ID.clone());
                }
                
                let i = self
                    .result_ids
                    .iter()
                    .position(|res_id| res_id == &id);
                
                if let Some(i) = i {
                    self.focused = i;
                    return self.update(Message::Activate(Some(i)));
                }
            }
            Message::Activate(idx) => {
                if let Some(idx) = idx {
                    if let Some(item) = self.launcher_items.get(idx) {
                        self.request(launcher::Request::Activate(item.id));
                        return self.hide();
                    }
                }
            }
            Message::CursorMoved(point) => {
                self.cursor_position = Some(point);
            }
            Message::LauncherEvent(event) => match event {
                launcher::Event::Started(tx) => {
                    self.tx = Some(tx);
                }
                launcher::Event::ServiceIsClosed => {
                    // Handle service closure by clearing the transmitter
                    self.tx = None;
                }
                launcher::Event::Response(res) => match res {
                    pop_launcher::Response::Context { id, options } => {
                        self.menu = Some((id, options));
                        if let Some(cursor_position) = self.cursor_position {
                            let rect = Rectangle {
                                x: cursor_position.x as i32,
                                y: cursor_position.y as i32,
                                width: 1,
                                height: 1,
                            };
                            return commands::popup::get_popup(SctkPopupSettings {
                                parent: self.window_id,
                                id: *MENU_ID,
                                positioner: SctkPositioner {
                                    size: None,
                                    size_limits: Limits::NONE.min_width(1.0).min_height(1.0).max_width(300.0).max_height(800.0),
                                    anchor_rect: rect,
                                    anchor:
                                        sctk::reexports::protocols::xdg::shell::client::xdg_positioner::Anchor::Right,
                                    gravity: sctk::reexports::protocols::xdg::shell::client::xdg_positioner::Gravity::Right,
                                    reactive: true,
                                    ..Default::default()
                                },
                                grab: true,
                                parent_size: None,
                                close_with_children: false,
                                input_zone: None,
                            });
                        }
                    }
                    pop_launcher::Response::DesktopEntry {
                        path,
                        gpu_preference,
                        action_name,
                    } => {
                        if let Some(entry) = cosmic::desktop::load_desktop_file(&[], path) {
                            let exec = if let Some(action_name) = action_name {
                                entry
                                    .desktop_actions
                                    .into_iter()
                                    .find(|action| action.name == action_name)
                                    .map(|action| action.exec)
                            } else {
                                entry.exec
                            };

                            let Some(exec) = exec else {
                                return Task::none();
                            };
                            return request_token(
                                Some(String::from(Self::APP_ID)),
                                Some(self.window_id),
                            )
                            .map(move |token| {
                                cosmic::Action::App(Message::ActivationToken(
                                    token,
                                    entry.id.to_string(),
                                    exec.clone(),
                                    gpu_preference,
                                    entry.terminal,
                                ))
                            });
                        }
                    }
                    pop_launcher::Response::Update(mut list) => {
                        info!("Received launcher response with {} items", list.len());
                        
                        if self.input_value.is_empty() {
                            list.reverse();
                        }
                        list.sort_by(|a, b| {
                            let a = i32::from(a.window.is_none());
                            let b = i32::from(b.window.is_none());
                            a.cmp(&b)
                        });

                        self.launcher_items.splice(.., list);
                        if self.result_ids.len() < self.launcher_items.len() {
                            self.result_ids.extend(
                                (self.result_ids.len()..self.launcher_items.len())
                                    .map(|id| Id::new((id).to_string()))
                                    .collect::<Vec<_>>(),
                            );
                        }

                        // Update screenshots for alt-tab mode and set active window
                        if !self.launcher_items.is_empty() && (self.alt_tab_mode || self.super_launcher_mode) {
                            // Set initial active window for alt-tab mode or super launcher mode
                            if let Some(current_active) = self.active {
                                // Adjust the active index if it's beyond the list size
                                if current_active >= self.launcher_items.len() {
                                    self.active = Some(0);
                                    println!("DEBUG: Adjusted selection to window 0 (only {} items)", self.launcher_items.len());
                                }
                            } else {
                                // Default to first window if no active selection
                                self.active = Some(0);
                                println!("DEBUG: Setting initial selection to window 0");
                            }
                        }
                        let mut cmds = Vec::new();

                        while let Some(element) = self.queue.pop_front() {
                            let updated = self.update(element);
                            cmds.push(updated);
                        }

                        if self.surface_state == SurfaceState::WaitingToBeShown {
                            cmds.push(self.show());
                        }

                        // Auto-focus search input when in super launcher mode - do this AFTER showing
                        if self.super_launcher_mode {
                            cmds.push(cosmic::widget::text_input::focus(
                                INPUT_ID.clone(),
                            ));
                        }

                        return Task::batch(cmds);
                    }
                    pop_launcher::Response::Fill(s) => {
                        self.input_value = s;
                        self.request(launcher::Request::Search(self.input_value.clone()));
                    }
                    pop_launcher::Response::Close => {
                        // Handle launcher close request
                        return self.hide();
                    }
                }
            }
            Message::Layer(e) => match e {
                LayerEvent::Focused | LayerEvent::Done => {
                    println!("DEBUG: Layer event: {:?}", e);
                }
                LayerEvent::Unfocused => {
                    // In Alt+Tab mode, don't hide on unfocus - wait for Alt release
                    if self.alt_tab_mode {
                        println!("DEBUG: Layer unfocused in Alt+Tab mode - staying visible");
                    } else {
                        println!("DEBUG: Layer unfocused - hiding launcher");
                        self.last_hide = Instant::now();
                        return self.hide();
                    }
                }
            },
            Message::Overlap(overlap_notify_event) => match overlap_notify_event {
                OverlapNotifyEvent::OverlapLayerAdd {
                    identifier,
                    namespace,
                    logical_rect,
                    exclusive,
                    ..
                } => {
                    if self.needs_clear {
                        self.needs_clear = false;
                        self.overlap.clear();
                    }
                    if exclusive > 0 || namespace == "Dock" || namespace == "Panel" {
                        self.overlap.insert(identifier, logical_rect);
                    }
                    self.handle_overlap();
                }
                OverlapNotifyEvent::OverlapLayerRemove { identifier } => {
                    self.overlap.remove(&identifier);
                    self.handle_overlap();
                }
                _ => {}
            },
            Message::Hide => {
                if self.menu.take().is_some() {
                    return commands::popup::destroy_popup(*MENU_ID);
                }
                return self.hide();
            }
            Message::KeyboardNav(e) => {
                match e {
                    keyboard_nav::Action::FocusNext => {
                        self.focus_next();
                        // TODO ideally we could use an operation to scroll exactly to a specific widget.
                        return iced_runtime::task::widget(operation::scrollable::snap_to(
                            SCROLLABLE.clone(),
                            RelativeOffset {
                                x: 0.,
                                y: (self.focused as f32
                                    / (self.launcher_items.len() as f32 - 1.).max(1.))
                                .max(0.0),
                            },
                        ));
                    }
                    keyboard_nav::Action::FocusPrevious => {
                        self.focus_previous();
                        return iced_runtime::task::widget(operation::scrollable::snap_to(
                            SCROLLABLE.clone(),
                            RelativeOffset {
                                x: 0.,
                                y: (self.focused as f32
                                    / (self.launcher_items.len() as f32 - 1.).max(1.))
                                .max(0.0),
                            },
                        ));
                    }
                    keyboard_nav::Action::Escape => {
                        self.input_value.clear();
                        self.request(launcher::Request::Search(String::new()));
                    }
                    _ => {}
                };
            }
            Message::ActivationToken(token, app_id, exec, dgpu, terminal) => {
                return Task::perform(launch(token, app_id, exec, dgpu, terminal), |()| {
                    cosmic::action::app(Message::Hide)
                });
            }

            Message::AltTab => {
                // Cycle to next window in Alt+Tab mode
                if !self.launcher_items.is_empty() {
                    let current = self.active.unwrap_or(0);
                    let next = (current + 1) % self.launcher_items.len();
                    self.active = Some(next);
                    println!("DEBUG: AltTab - cycling to {} (of {})", next, self.launcher_items.len());
                } else {
                    self.active = Some(0);
                }
            }
            Message::ShiftAltTab => {
                // Cycle to previous window in Alt+Tab mode
                if !self.launcher_items.is_empty() {
                    let current = self.active.unwrap_or(0);
                    let prev = if current == 0 {
                        self.launcher_items.len() - 1
                    } else {
                        current - 1
                    };
                    self.active = Some(prev);
                    println!("DEBUG: ShiftAltTab - cycling to {} (of {})", prev, self.launcher_items.len());
                } else {
                    self.active = Some(0);
                }
            }
            Message::AltRelease => {
                // On Alt release, activate the currently selected window and hide
                if self.alt_tab_mode {
                    let selected_index = self.active.unwrap_or(0);
                    println!("DEBUG: Alt released - activating window at index {} then hiding", selected_index);
                    if let Some(item) = self.launcher_items.get(selected_index) {
                        self.request(launcher::Request::Activate(item.id));
                    }
                    return self.hide();
                }
            }
            Message::SuperRelease => {
                // On Super release in super launcher mode, hide the launcher
                if self.super_launcher_mode {
                    return self.hide();
                }
            }
            Message::Opened(size, _id) => {
                self.height = size.height;
                self.handle_overlap();
            }
            Message::BackendEvent(event) => match event {
                WaylandUpdate::Toplevel(toplevel_update) => {
                    self.handle_toplevel_update(toplevel_update);
                }
                WaylandUpdate::Image(handle, wayland_image) => {
                    info!("Storing screenshot for toplevel: {:?}", handle);
                    self.toplevel_captures.insert(handle.clone(), wayland_image);
                    self.screenshot_cache_time.insert(handle, Instant::now());
                }
                WaylandUpdate::Init => {}
                WaylandUpdate::Finished => {}
            }
            Message::DebouncedSearch(search_term) => {
                // Only perform search if this is the most recent debounce timer
                if let Some(timer) = self.search_debounce_timer {
                    // Reduced threshold from 250ms to 40ms for more responsiveness
                    if timer.elapsed() >= Duration::from_millis(40) && search_term == self.input_value {
                        self.request(launcher::Request::Search(search_term));
                        self.search_debounce_timer = None;
                    }
                }
            }
        }
        
        Task::none()
    }

    fn dbus_activation(
        &mut self,
        msg: cosmic::dbus_activation::Message,
    ) -> iced::Task<cosmic::Action<Self::Message>> {
        match msg.msg {
            Details::Activate => {
                if self.surface_state != SurfaceState::Hidden {
                    return self.hide();
                }
                // hack: allow to close the launcher from the panel button
                if self.last_hide.elapsed().as_millis() > 100 {
                    self.request(launcher::Request::Search(String::new()));
                    self.set_mode(false, true); // Super launcher mode only
                    return self.show();
                }
            }
            Details::ActivateAction { action, .. } => {
                println!("DEBUG: ActivateAction {}", action);

                let Ok(cmd) = LauncherTasks::from_str(&action) else {
                    return Task::none();
                };

                self.set_mode(true, false); // Alt+Tab mode only
                
                // Use cached toplevels immediately for instant display
                self.populate_from_cached_toplevels();
                
                // For Alt+Tab, we don't need search request - we have cached data
                // Fresh screenshots will come from wayland subscription
                
                let show_task = self.show();
                let update_task = match cmd {
                    LauncherTasks::AltTab => self.update(Message::AltTab),
                    LauncherTasks::ShiftAltTab => self.update(Message::ShiftAltTab),
                };
                return Task::batch(vec![show_task, update_task]);
            }
            Details::Open { .. } => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        unreachable!("No main window")
    }

    #[allow(clippy::too_many_lines)]
    fn view_window(&self, id: SurfaceId) -> Element<'_, Self::Message> {
        if id == self.window_id {
            // Don't render if surface should be hidden
            if self.surface_state == SurfaceState::Hidden {
                println!("DEBUG: view_window called but surface is Hidden - returning empty");
                return container(text(""))
                    .width(Length::Fixed(1.0))
                    .height(Length::Fixed(1.0))
                    .into();
            }
            
            // Safety check to prevent overflow in surface sizing
            if !self.height.is_finite() || self.height > 10000.0 || self.height < 1.0 {
                return container(text("Loading..."))
                    .width(Length::Fixed(400.0))
                    .height(Length::Fixed(100.0))
                    .into();
            }
            // Show appropriate view based on mode
            if self.alt_tab_mode {
                // Alt+Tab mode: Window switching with thumbnails
                self.view_alt_tab()
            } else {
                // Default launcher mode: App search and browsing
                self.view_search()
            }
        } else {
            container(text(""))
                .width(Length::Fixed(1.0))
                .height(Length::Fixed(1.0))
                .into()
        }
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::batch(vec![
            wayland_subscription().map(Message::BackendEvent),
            launcher::subscription(0).map(Message::LauncherEvent),
            listen_raw(|e, _status, id| match e {
                cosmic::iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                    wayland::Event::Layer(e, ..),
                )) => Some(Message::Layer(e)),
                cosmic::iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                    wayland::Event::OverlapNotify(event),
                )) => Some(Message::Overlap(event)),
                cosmic::iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
                    key,
                    ..
                }) => {
                    println!("DEBUG: Key released: {:?}", key);
                    match key {
                        Key::Named(Named::Alt) => {
                            // Alt released - send message to let app decide what to do
                            println!("DEBUG: Alt key released");
                            return Some(Message::AltRelease);
                        }
                        Key::Named(Named::Super) => {
                            // Super released - send message to let app decide what to do
                            println!("DEBUG: Super key released");
                            return Some(Message::SuperRelease);
                        }
                        _ => {}
                    }
                    None
                },
                cosmic::iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                    // Debug: Log ALL key presses to understand what's happening
                    println!("DEBUG: Key pressed: {:?}, modifiers: alt={}, shift={}, ctrl={}", key, modifiers.alt(), modifiers.shift(), modifiers.control());
                    
                    // Killswitch: Ctrl+Alt+J to exit
                    if let Key::Character(c) = &key {
                        if c == "j" && modifiers.control() && modifiers.alt() {
                            println!("DEBUG: Killswitch activated - exiting");
                            std::process::exit(0);
                        }
                    }

                    // Handle Alt+Tab and Shift+Alt+Tab explicitly - but only when UI is visible
                    if let Key::Named(Named::Tab) = key {
                        println!("DEBUG: Raw Tab event: alt={}, shift={}", modifiers.alt(), modifiers.shift());
                        // Only handle Tab navigation when launcher UI might be visible
                        // We can't access self.surface_state here, so we'll handle this in the message update
                        if modifiers.alt() && modifiers.shift() {
                            println!("DEBUG: Raw Shift+Alt+Tab");
                            return Some(Message::ShiftAltTab);
                        } else if modifiers.alt() {
                            println!("DEBUG: Raw Alt+Tab");
                            return Some(Message::AltTab);
                        }
                        println!("DEBUG: Raw Tab - focusing next");
                        return Some(Message::KeyboardNav(keyboard_nav::Action::FocusNext));
                    }
                    // Handle number activation
                    // if let Key::Character(c) = key.clone() {
                    //     let nums = (1..=9)
                    //         .map(|n| (n.to_string(), ((n + 10) % 10) - 1))
                    //         .chain((0..=0).map(|n| (n.to_string(), ((n + 10) % 10) - 1)))
                    //         .collect::<Vec<_>>();
                    //     if let Some(&(ref _s, idx)) = nums.iter().find(|&&(ref s, _)| s == &c) {
                    //         return Some(Message::Activate(Some(idx)));
                    //     }
                    // }
                    // Essential key handling
                    if let Key::Named(named_key) = key.clone() {
                        match named_key {
                            Named::ArrowUp => return Some(Message::KeyboardNav(keyboard_nav::Action::FocusPrevious)),
                            Named::ArrowDown => return Some(Message::KeyboardNav(keyboard_nav::Action::FocusNext)),
                            Named::Escape => return Some(Message::Hide),
                            Named::Enter => return Some(Message::Activate(None)),
                            _ => {}
                        }
                    }
                    None
                },
                cosmic::iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                    Some(Message::CursorMoved(position))
                }
                cosmic::iced::Event::Window(WindowEvent::Opened { position: _, size }) => {
                    Some(Message::Opened(size, id))
                }
                cosmic::iced::Event::Window(WindowEvent::Resized(s)) => {
                    Some(Message::Opened(s, id))
                }
                _ => None,
            }),
        ])
    }
}

impl CosmicLauncher {
    fn create_grid_layout<'a>(&self, items: Vec<Element<'a, Message>>, columns: usize) -> Element<'a, Message> {
        if items.is_empty() {
            return column![].into();
        }

        let rows = (items.len() + columns - 1) / columns;
        let mut grid_rows: Vec<Element<'a, Message>> = Vec::new();
        let mut item_iter = items.into_iter();

        for _row_idx in 0..rows {
            let mut row_items: Vec<Element<'a, Message>> = Vec::new();
            
            for _col_idx in 0..columns {
                if let Some(item) = item_iter.next() {
                    row_items.push(item);
                }
            }
            
            if !row_items.is_empty() {
                let mut row_element = row![];
                for item in row_items {
                    row_element = row_element.push(item);
                }
                grid_rows.push(row_element.spacing(10).into());
            }
        }

        let mut grid_column = column![];
        for grid_row in grid_rows {
            grid_column = grid_column.push(grid_row);
        }
        
        grid_column.spacing(8).into()
    }

    fn create_search_item_element<'a>(&self, item: &'a SearchResult, idx: usize, is_focused: bool) -> Element<'a, Message> {
        // For search results, dispatch based on whether we have a screenshot, not item.window
        let icon_element = if let Some(wayland_image) = self.find_screenshot_for_item(item) {
            // If we have a screenshot, show both window preview AND app icon
            let handle = Handle::from_rgba(
                wayland_image.width,
                wayland_image.height,
                wayland_image.img.clone()
            );
            
            // Create app icon element
            let app_icon = match &item.icon {
                Some(IconSource::Name(icon_name)) => {
                    icon::from_name(icon_name.clone()).size(16)
                }
                Some(IconSource::Mime(_mime_type)) => {
                    icon::from_name("text-x-generic").size(16)
                }
                _ => {
                    icon::from_name("application-x-executable").size(16)
                }
            };
            
            // Show screenshot with small app icon overlay - rectangular aspect ratio
            container(
                row![
                    Image::new(handle)
                        .width(Length::Fixed(70.0))
                        .height(Length::Fixed(40.0))
                        .content_fit(cosmic::iced::ContentFit::Fill),
                    container(app_icon)
                        .width(Length::Fixed(20.0))
                        .height(Length::Fixed(20.0))
                        .center_x(Length::Fill)
                        .center_y(Length::Fill)
                ]
                .spacing(4)
                .align_y(Alignment::Center)
            )
            .width(Length::Fixed(110.0))
            .height(Length::Fixed(50.0))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
        } else {
            // If no screenshot available, show the app icon based on IconSource
            match &item.icon {
                Some(IconSource::Name(icon_name)) => {
                    container(
                        icon::from_name(icon_name.clone())
                            .size(20)
                    )
                    .width(Length::Fixed(40.0))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                }
                Some(IconSource::Mime(_mime_type)) => {
                    // For mime types, use a generic document icon
                    container(
                        icon::from_name("text-x-generic")
                            .size(20)
                    )
                    .width(Length::Fixed(40.0))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                }
                _ => {
                    // For items without specific icons, try common fallback icon names
                    container(
                        icon::from_name("application-x-executable")
                            .size(20)
                    )
                    .width(Length::Fixed(40.0))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                }
            }
        };

        // Create clickable search result item
        mouse_area(
            container(
                row![
                    icon_element,
                    // Show app name and description
                    {
                        let display_name = if let Some(paren_pos) = item.name.find('(') {
                            // Extract everything before the first '(' - e.g., "Flatpak (System)" -> "Flatpak"
                            item.name[..paren_pos].trim()
                        } else if let Some(dash_pos) = item.name.find(" - ") {
                            // Extract everything before " - " - e.g., "Firefox - Web Browser" -> "Firefox"
                            item.name[..dash_pos].trim()
                        } else {
                            // Use the full name if no parsing patterns found
                            item.name.trim()
                        };
                        
                        // Extract description part if available
                        let description = if let Some(dash_pos) = item.name.find(" - ") {
                            // Get everything after " - "
                            Some(item.name[dash_pos + 3..].trim())
                        } else if let Some(_paren_pos) = item.name.find('(') {
                            // Look for description in the description field instead
                            if !item.description.trim().is_empty() && item.description != item.name {
                                Some(item.description.trim())
                            } else {
                                None
                            }
                        } else if !item.description.trim().is_empty() && item.description != item.name {
                            // Use description field if it's different from name
                            Some(item.description.trim())
                        } else {
                            None
                        };
                        
                        column![
                            // App name
                            if is_focused {
                                text(display_name).size(14).class(cosmic::theme::Text::Accent).wrapping(Wrapping::Word)
                            } else {
                                text(display_name).size(14).wrapping(Wrapping::Word)
                            },
                            // Description (if available)
                            if let Some(desc) = description {
                                text(desc).size(12).class(cosmic::theme::Text::Default).wrapping(Wrapping::Word)
                            } else {
                                text("").size(0) // Empty placeholder
                            }
                        ]
                        .spacing(2)
                        .width(Length::Fixed(380.0)) // Adjusted width for smaller rectangular screenshot area
                    }
                ]
                .spacing(12)
                .align_y(Alignment::Center)
            )
            .padding(12) // Consistent padding with window items
            .width(Length::Fixed(520.0))
            .class(if is_focused {
                cosmic::theme::Container::Primary
            } else {
                cosmic::theme::Container::Card
            })
        )
        .on_press(Message::Activate(Some(idx)))
        .into()
    }

    fn create_window_item_element<'a>(&self, item: &'a SearchResult, idx: usize, is_selected: bool) -> Element<'a, Message> {
        // Try to find screenshot for this window
        let screenshot = self.find_screenshot_for_item(item);
        
        // Create preview image or fallback icon - fixed size and centered
        let preview_element = if let Some(wayland_image) = screenshot {
            // Use actual window screenshot as preview with app icon
            let handle = Handle::from_rgba(
                wayland_image.width,
                wayland_image.height,
                wayland_image.img.clone()
            );
            
            // Create app icon element
            let app_icon = match &item.icon {
                Some(IconSource::Name(icon_name)) => {
                    icon::from_name(icon_name.clone()).size(20)
                }
                Some(IconSource::Mime(_mime_type)) => {
                    icon::from_name("text-x-generic").size(20)
                }
                _ => {
                    icon::from_name("application-x-executable").size(20)
                }
            };
            
            container(
                column![
                    Image::new(handle)
                        .width(Length::Fixed(220.0))
                        .height(Length::Fixed(125.0))
                        .content_fit(cosmic::iced::ContentFit::Fill),
                    container(app_icon)
                        .width(Length::Fixed(24.0))
                        .height(Length::Fixed(24.0))
                        .center_x(Length::Fill)
                        .center_y(Length::Fill)
                ]
                .spacing(4)
                .align_x(Alignment::Center)
            )
            .width(Length::Fixed(220.0))
            .height(Length::Fixed(155.0)) // Height to accommodate app icon below
            .center_x(Length::Fill)
            .center_y(Length::Fill)
        } else {
            // Fallback: try to show app icon if available, otherwise use window emoji
            match &item.icon {
                Some(IconSource::Name(icon_name)) => {
                    container(
                        icon::from_name(icon_name.clone())
                            .size(40)
                    )
                    .width(Length::Fixed(220.0))
                    .height(Length::Fixed(125.0))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                }
                _ => {
                    // Final fallback to emoji icon if no app icon available
                    container(
                        text(if is_selected { "▶️" } else { "🪟" })
                            .size(32)
                    )
                    .width(Length::Fixed(220.0))
                    .height(Length::Fixed(125.0))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                }
            }
        };

        // Create consistent window item with same styling across modes, but make it clickable
        let content = row![
            // Preview image or icon - fixed size and centered
            preview_element,
            // Only show description text (second line) with consistent size and color for selection
            container(
                if is_selected {
                    text(&item.description).size(14).class(cosmic::theme::Text::Accent)
                } else {
                    text(&item.description).size(14)
                }
            )
            .width(Length::Fill)
            .center_y(Length::Fill)
        ]
        .spacing(15)
        .align_y(Alignment::Center);

        // Wrap in mouse_area for click functionality and enhanced styling for selection
        mouse_area(
            container(content)
                .padding(12) // Consistent padding - no size changes
                .width(Length::Fixed(600.0))
                .height(Length::Fixed(180.0)) // Reduced to accommodate smaller screenshots
                .class(if is_selected {
                    cosmic::theme::Container::Primary // Use primary highlight for selection
                } else {
                    cosmic::theme::Container::Card
                })
        )
        .on_press(Message::Activate(Some(idx)))
        .into()
    }

    fn view_search(&self) -> Element<'_, Message> {
        let mut content = column![]
            .spacing(15)
            .align_x(Alignment::Center);

        // Search field at the top with background
        content = content.push(
            container(
                column![
                    text("Launcher").size(24),
                    text_input::search_input("Type to search", &self.input_value)
                        .on_input(Message::InputChanged)
                        .width(600) // Increased width
                        .id(INPUT_ID.clone())
                ]
                .spacing(8)
                .align_x(Alignment::Center)
            )
            .padding(20)
            .class(cosmic::theme::Container::Card) // Add background card styling
        );

        // Show results below search - use search item elements when there's input, window elements when empty
        if self.launcher_items.is_empty() {
            content = content.push(text("No windows open").size(16));
        } else {
            let mut item_elements: Vec<Element<Message>> = Vec::new();
            
            for (idx, item) in self.launcher_items.iter().enumerate() {
                let is_selected = self.active == Some(idx);
                println!("DEBUG: Launcher rendering item {} - '{}', selected: {}", idx, item.name, is_selected);
                
                // Use search item elements when searching (input not empty) to show clean app names
                // Use window item elements when browsing (input empty) to show window titles
                let item_element = if self.input_value.trim().is_empty() {
                    // Browsing mode - show window titles using window item element
                    self.create_window_item_element(item, idx, is_selected)
                } else {
                    // Search mode - show clean app names using search item element
                    let is_focused = self.focused == idx;
                    self.create_search_item_element(item, idx, is_focused)
                };
                item_elements.push(item_element);
            }
            
            // Create grid layout with 2 columns in a wide container
            let grid = self.create_grid_layout(item_elements, 2);
            content = content.push(
                container(grid)
                    .width(Length::Fixed(
                        if self.input_value.trim().is_empty() { 1300.0 } else { 1200.0 }
                    )) // Adjust width based on content type
                    .padding(20)
                    .class(cosmic::theme::Container::Card)
            );
        }

        container(content)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding([80, 20, 20, 20]) // top, right, bottom, left - moved down from top
            .into()
    }

    fn view_alt_tab(&self) -> Element<'_, Message> {
        let mut content = column![]
            .spacing(15)
            .align_x(Alignment::Center);

        // Title and instructions
        content = content.push(
            container(
                column![
                    text("Alt + Tab - Task Switcher").size(24),
                    text("Use Tab to cycle through windows, release Alt to switch")
                        .size(14)
                        .class(cosmic::theme::Text::Default)
                ]
                .spacing(8)
                .align_x(Alignment::Center)
            )
            .padding(20)
            .class(cosmic::theme::Container::Card)
        );

        // Show window items
        if self.launcher_items.is_empty() {
            content = content.push(text("No windows open").size(16));
        } else {
            let mut item_elements: Vec<Element<Message>> = Vec::new();
            
            for (idx, item) in self.launcher_items.iter().enumerate() {
                let is_selected = self.active == Some(idx);
                let window_element = self.create_window_item_element(item, idx, is_selected);
                item_elements.push(window_element);
            }
            
            // Create grid layout with 2 columns
            let grid = self.create_grid_layout(item_elements, 2);
            content = content.push(
                container(grid)
                    .width(Length::Fixed(1300.0))
                    .padding(20)
                    .class(cosmic::theme::Container::Card)
            );
        }

        container(content)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding([80, 20, 20, 20])
            .into()
    }
}

