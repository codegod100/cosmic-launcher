use crate::{app::iced::event::listen_raw, components, fl, subscriptions::launcher};
use crate::wayland_subscription::{WaylandUpdate, ToplevelUpdate, WaylandImage, wayland_subscription};
use cosmic::cctk::toplevel_info::ToplevelInfo;
use cosmic::cctk::wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use clap::Parser;
use cosmic::app::{Core, CosmicFlags, Settings, Task};
use cosmic::cctk::sctk;
use cosmic::dbus_activation::Details;
use cosmic::iced::alignment::Alignment;
use cosmic::iced::event::Status;
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
    overlap_notify,
};
use cosmic::iced::widget::{column, container, image::{Handle, Image}};
use cosmic::iced::{self, Length, Size, Subscription};
use cosmic::iced_core::keyboard::key::Named;
use cosmic::iced_core::widget::operation;
use cosmic::iced_core::{Padding, Point, Rectangle, window};
use cosmic::iced_runtime::core::event::wayland::LayerEvent;
use cosmic::iced_runtime::core::event::{PlatformSpecific, wayland};
use cosmic::iced_runtime::core::layout::Limits;
use cosmic::iced_runtime::core::window::{Event as WindowEvent, Id as SurfaceId};
use cosmic::iced_runtime::platform_specific::wayland::{
    layer_surface::IcedMargin,
};
use cosmic::iced_widget::row;
use cosmic::iced_widget::scrollable::RelativeOffset;
use cosmic::iced_winit::commands::overlap_notify::overlap_notify;
use cosmic::theme::Button;
use cosmic::widget::icon;
use cosmic::widget::{
    button, mouse_area, text,
    text_input,
};
use cosmic::iced::widget::text::Wrapping;
use cosmic::{Element, keyboard_nav};
use cosmic::{iced_runtime, surface};
use iced::keyboard::Key;
use pop_launcher::{ContextOption, GpuPreference, IconSource, SearchResult};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::sync::LazyLock;
use std::{
    collections::{HashMap, VecDeque},
    rc::Rc,
    str::FromStr,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tracing::{debug, error, info};
use unicode_truncate::UnicodeTruncateStr;
use unicode_width::UnicodeWidthStr;

static AUTOSIZE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("autosize"));
static MAIN_ID: LazyLock<Id> = LazyLock::new(|| Id::new("main"));
static INPUT_ID: LazyLock<Id> = LazyLock::new(|| Id::new("input_id"));
static SCROLLABLE: LazyLock<Id> = LazyLock::new(|| Id::new("scrollable"));

pub(crate) static MENU_ID: LazyLock<SurfaceId> = LazyLock::new(SurfaceId::unique);
const SCROLL_MIN: usize = 8;

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

pub fn menu_button<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> cosmic::widget::Button<'a, Message> {
    button::custom(content)
        .class(Button::AppletMenu)
        .padding(menu_control_padding())
        .width(Length::Fill)
}

pub fn menu_control_padding() -> Padding {
    let theme = cosmic::theme::active();
    let cosmic = theme.cosmic();
    [cosmic.space_xxs(), cosmic.space_m()].into()
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
    Context(usize),
    MenuButton(u32, u32),
    CloseContextMenu,
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
    Surface(surface::Action),
    PreviewAction(components::preview_grid::PreviewMessage),

    BackendEvent(WaylandUpdate),
    DebouncedSearch(String), // For debounced search after delay
}

impl CosmicLauncher {
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
                size: Some((Some(1200), Some(1600))), // Increased width from 600 to 1200
                size_limits: Limits::NONE.min_width(1200.0).min_height(1600.0).max_width(1200.0).max_height(1600.0),
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
        self.alt_tab_mode = false; // Reset Alt+Tab mode
        self.super_launcher_mode = false; // Reset Super launcher mode
        self.search_debounce_timer = None; // Clear search debounce timer
        self.queue.clear();

        self.request(launcher::Request::Close);

        let mut tasks = Vec::new();

        if self.surface_state == SurfaceState::Visible {
            println!("DEBUG: Destroying layer surface and hiding");
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
        let conn = wayland_client::Connection::connect_to_env()
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
                
                // Set debounce timer for search in super launcher mode
                if self.super_launcher_mode {
                    self.search_debounce_timer = Some(Instant::now());
                    return Task::perform(
                        async move {
                            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                            value
                        },
                        |search_term| cosmic::Action::App(Message::DebouncedSearch(search_term)),
                    );
                }
            }
            Message::Backspace => {
                // Always update input value immediately for responsive UI
                self.input_value.pop();
                
                // Set debounce timer for search in super launcher mode
                if self.super_launcher_mode {
                    self.search_debounce_timer = Some(Instant::now());
                    let value = self.input_value.clone();
                    return Task::perform(
                        async move {
                            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                            value
                        },
                        |search_term| cosmic::Action::App(Message::DebouncedSearch(search_term)),
                    );
                }
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
                    .position(|res_id| res_id == &id)
                    .unwrap_or_default();

                if let Some(id) = self.launcher_items.get(i).map(|res| res.id) {
                    self.request(launcher::Request::Complete(id));
                }
            }
            Message::Activate(i) => {
                let index = i.unwrap_or(self.focused);
                
                if let Some(item) = self.launcher_items.get(index) {
                    self.request(launcher::Request::Activate(item.id));
                } else {
                    return self.hide();
                }
            }
            Message::Context(i) => {
                if self.menu.take().is_some() {
                    return commands::popup::destroy_popup(*MENU_ID);
                }

                if let Some(item) = self.launcher_items.get(i) {
                    self.request(launcher::Request::Context(item.id));
                }
            }
            Message::CursorMoved(pos) => {
                self.cursor_position = Some(pos);
            }
            Message::MenuButton(i, context) => {
                self.request(launcher::Request::ActivateContext(i, context));

                if self.menu.take().is_some() {
                    return commands::popup::destroy_popup(*MENU_ID);
                }
            }
            Message::Opened(size, window_id) => {
                if window_id == self.window_id {
                    // Clamp height to reasonable bounds to prevent overflow
                    self.height = size.height.clamp(1.0, 800.0);
                    self.handle_overlap();
                }
            }
            Message::LauncherEvent(e) => match e {
                launcher::Event::Started(tx) => {
                    info!("Pop-launcher service started and connected");
                    self.tx.replace(tx);
                    info!("Sending initial search request to pop-launcher");
                    self.request(launcher::Request::Search(self.input_value.clone()));
                }
                launcher::Event::ServiceIsClosed => {
                    info!("Pop-launcher service closed");
                    self.request(launcher::Request::ServiceIsClosed);
                }
                launcher::Event::Response(response) => match response {
                    pop_launcher::Response::Close => {
                        return self.hide();
                    }
                    #[allow(clippy::cast_possible_truncation)]
                    pop_launcher::Response::Context { id, options } => {
                        if options.is_empty() {
                            return Task::none();
                        }

                        self.menu = Some((id, options));
                        let Some(pos) = self.cursor_position.as_ref() else {
                            return Task::none();
                        };
                        let rect = Rectangle {
                            x: pos.x.round() as i32,
                            y: pos.y.round() as i32,
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
                },
            },
            Message::Layer(e) => match e {
                LayerEvent::Focused | LayerEvent::Done => {
                    println!("DEBUG: Layer event: {:?}", e);
                }
                LayerEvent::Unfocused => {
                    println!("DEBUG: Layer unfocused - hiding launcher");
                    self.last_hide = Instant::now();
                    return self.hide();
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
            Message::CloseContextMenu => {
                if self.menu.take().is_some() {
                    return commands::popup::destroy_popup(*MENU_ID);
                }
            }
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

            Message::BackendEvent(event) => match event {
                WaylandUpdate::Toplevel(toplevel_update) => {
                    self.handle_toplevel_update(toplevel_update);
                }
                WaylandUpdate::Image(handle, wayland_image) => {
                    info!("Storing screenshot for toplevel: {:?}", handle);
                    self.toplevel_captures.insert(handle, wayland_image);
                }
                WaylandUpdate::Init => {}
                WaylandUpdate::Finished => {}
            },
            Message::DebouncedSearch(search_term) => {
                // Only perform search if this is the most recent debounce timer
                if let Some(timer) = self.search_debounce_timer {
                    // Check if enough time has passed and input matches
                    if timer.elapsed() >= Duration::from_millis(250) && search_term == self.input_value {
                        self.request(launcher::Request::Search(search_term));
                        self.search_debounce_timer = None;
                    }
                }
            }
            Message::AltTab => {
                // Cycle to next window in the list
                if !self.launcher_items.is_empty() {
                    let current = self.active.unwrap_or(0);
                    let next = (current + 1) % self.launcher_items.len();
                    self.active = Some(next);
                    println!("DEBUG: AltTab - cycling from {} to {} (of {})", current, next, self.launcher_items.len());
                }
            }
            Message::ShiftAltTab => {
                // Cycle to previous window in the list
                if !self.launcher_items.is_empty() {
                    let current = self.active.unwrap_or(0);
                    let prev = if current == 0 {
                        self.launcher_items.len() - 1
                    } else {
                        current - 1
                    };
                    self.active = Some(prev);
                    println!("DEBUG: ShiftAltTab - cycling from {} to {} (of {})", current, prev, self.launcher_items.len());
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
                // On Super release, just hide if in super launcher mode
                if self.super_launcher_mode {
                    println!("DEBUG: Super key released in launcher mode");
                    return self.hide();
                }
            }
            Message::Surface(_) => {
                // TODO: handle surface action
            }
            Message::PreviewAction(_) => {
                // TODO: handle preview action
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
                    // Enable Super launcher mode (Alt+Tab list with search)
                    self.super_launcher_mode = true;
                    self.request(launcher::Request::Search(String::new()));

                    self.surface_state = SurfaceState::WaitingToBeShown;
                    
                    // Focus search input when launcher opens - use multiple attempts
                    return Task::batch(vec![
                        cosmic::widget::text_input::focus(INPUT_ID.clone()),
                        Task::perform(
                            async { tokio::time::sleep(tokio::time::Duration::from_millis(100)).await; },
                            |_| cosmic::Action::App(Message::CompleteFocusedId(INPUT_ID.clone())),
                        ),
                    ]);
                }
            }
            Details::ActivateAction { action, .. } => {
                debug!("ActivateAction {}", action);

                let Ok(cmd) = LauncherTasks::from_str(&action) else {
                    return Task::none();
                };

                if self.surface_state == SurfaceState::Hidden {
                    self.surface_state = SurfaceState::WaitingToBeShown;
                    // For Alt+Tab, we need to populate the launcher with windows
                    // Send an empty search to get the window list
                    self.request(launcher::Request::Search(String::new()));
                }

                match cmd {
                    LauncherTasks::AltTab => {
                        // Enable Alt+Tab mode
                        self.alt_tab_mode = true;
                        // Set initial selection for Alt+Tab mode
                        if self.active.is_none() && !self.launcher_items.is_empty() {
                            self.active = Some(0);
                        }
                        return self.update(Message::AltTab);
                    }
                    LauncherTasks::ShiftAltTab => {
                        // Enable Alt+Tab mode
                        self.alt_tab_mode = true;
                        // Set initial selection for Shift+Alt+Tab mode
                        if self.active.is_none() && !self.launcher_items.is_empty() {
                            // Start with second window for reverse cycling
                            self.active = Some(if self.launcher_items.len() > 1 { 1 } else { 0 });
                        }
                        return self.update(Message::ShiftAltTab);
                    }
                }
            }
            Details::Open { .. } => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<Self::Message> {
        unreachable!("No main window")
    }

    #[allow(clippy::too_many_lines)]
    fn view_window(&self, id: SurfaceId) -> Element<Self::Message> {
        if id == self.window_id {
            // Safety check to prevent overflow in surface sizing
            if !self.height.is_finite() || self.height > 10000.0 || self.height < 1.0 {
                return container(text("Loading..."))
                    .width(Length::Fixed(400.0))
                    .height(Length::Fixed(100.0))
                    .into();
            }
            // Show appropriate view based on mode
            if self.alt_tab_mode {
                self.view_alt_tab()
            } else if self.super_launcher_mode {
                self.view_super_launcher()
            } else {
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
            listen_raw(|e, status, id| match e {
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
                    
                    // Handle Alt+Tab and Shift+Alt+Tab explicitly
                    if let Key::Named(Named::Tab) = key {
                        println!("DEBUG: Raw Tab event: alt={}, shift={}", modifiers.alt(), modifiers.shift());
                        // Raw keyboard events for Tab
                        if modifiers.alt() && modifiers.shift() {
                            println!("DEBUG: Raw Shift+Alt+Tab");
                            return Some(Message::ShiftAltTab);
                        } else if modifiers.alt() {
                            println!("DEBUG: Raw Alt+Tab");
                            return Some(Message::AltTab);
                        } else {
                            println!("DEBUG: Raw Tab - focusing next");
                            return Some(Message::KeyboardNav(keyboard_nav::Action::FocusNext));
                        }
                    }
                    // Handle number activation
                    if let Key::Character(c) = key.clone() {
                        let nums = (1..=9)
                            .map(|n| (n.to_string(), ((n + 10) % 10) - 1))
                            .chain((0..=0).map(|n| (n.to_string(), ((n + 10) % 10) - 1)))
                            .collect::<Vec<_>>();
                        if let Some(&(ref s, idx)) = nums.iter().find(|&&(ref s, _)| s == &c) {
                            return Some(Message::Activate(Some(idx)));
                        }
                    }
                    // Function keys for activation
                    if let Key::Named(func_key) = key.clone() {
                        match func_key {
                            Named::F1 | Named::F2 | Named::F3 | Named::F4 | Named::F5
                            | Named::F6 | Named::F7 | Named::F8 | Named::F9 | Named::F10 => {
                                // Map function keys F1-F10 to indices 0-9
                                let idx = match func_key {
                                    Named::F1 => 0,
                                    Named::F2 => 1,
                                    Named::F3 => 2,
                                    Named::F4 => 3,
                                    Named::F5 => 4,
                                    Named::F6 => 5,
                                    Named::F7 => 6,
                                    Named::F8 => 7,
                                    Named::F9 => 8,
                                    Named::F10 => 9,
                                    _ => unreachable!(),
                                };
                                return Some(Message::Activate(Some(idx)));
                            }
                            Named::ArrowUp => return Some(Message::KeyboardNav(keyboard_nav::Action::FocusPrevious)),
                            Named::ArrowDown => return Some(Message::KeyboardNav(keyboard_nav::Action::FocusNext)),
                            Named::Escape => return Some(Message::Hide),
                            Named::Backspace if matches!(status, Status::Ignored) && modifiers.is_empty() => {
                                return Some(Message::Backspace);
                            }
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
            
            // Show screenshot with small app icon overlay
            container(
                row![
                    Image::new(handle)
                        .width(Length::Fixed(32.0))
                        .height(Length::Fixed(24.0))
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
            .width(Length::Fixed(60.0))
            .height(Length::Fixed(30.0))
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
                        } else if let Some(paren_pos) = item.name.find('(') {
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
                        .width(Length::Fixed(360.0)) // Adjusted width to account for wider icon area
                    }
                ]
                .spacing(12)
                .align_y(Alignment::Center)
            )
            .padding(12) // Consistent padding with window items
            .width(Length::Fixed(450.0))
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
                        .width(Length::Fixed(100.0))
                        .height(Length::Fixed(75.0))
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
            .width(Length::Fixed(100.0))
            .height(Length::Fixed(105.0)) // Increased height to accommodate app icon
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
                    .width(Length::Fixed(100.0))
                    .height(Length::Fixed(75.0))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                }
                _ => {
                    // Final fallback to emoji icon if no app icon available
                    container(
                        text(if is_selected { "▶️" } else { "🪟" })
                            .size(32)
                    )
                    .width(Length::Fixed(100.0))
                    .height(Length::Fixed(75.0))
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
                .width(Length::Fixed(520.0))
                .height(Length::Fixed(120.0)) // Increased height to accommodate app icon below screenshot
                .class(if is_selected {
                    cosmic::theme::Container::Primary // Use primary highlight for selection
                } else {
                    cosmic::theme::Container::Card
                })
        )
        .on_press(Message::Activate(Some(idx)))
        .into()
    }

    fn view_search(&self) -> Element<Message> {
        // Search UI with clickable items
        let mut content = column![]
            .spacing(15)
            .align_x(Alignment::Center);

        // Search field at the top with background
        content = content.push(
            container(
                column![
                    text("Search Mode").size(20),
                    text_input::search_input(fl!("type-to-search"), &self.input_value)
                        .on_input(Message::InputChanged)
                        .width(600) // Increased width
                        .id(INPUT_ID.clone())
                ]
                .spacing(10)
                .align_x(Alignment::Center)
            )
            .padding(20)
            .class(cosmic::theme::Container::Card) // Add background card styling
        );

        // Show search results if any - use same field as alt-tab (description) with proper icons
        if !self.launcher_items.is_empty() {
            let mut result_elements: Vec<Element<Message>> = Vec::new();
            
            for (idx, item) in self.launcher_items.iter().enumerate() {
                let is_focused = self.focused == idx;
                
                // Use the specialized search item element that shows proper icons/previews
                let result_item = self.create_search_item_element(item, idx, is_focused);
                result_elements.push(result_item);
            }
            
            // Create grid layout with 2 columns for search results in a wider container
            let grid = self.create_grid_layout(result_elements, 2);
            content = content.push(
                container(grid)
                    .width(Length::Fixed(1000.0)) // Wide container for grid
                    .padding(20)
                    .class(cosmic::theme::Container::Card)
            );
        }

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }

    fn view_alt_tab(&self) -> Element<Message> {
        let mut content = column![]
            .spacing(10)
            .align_x(Alignment::Center);

        // Title
        content = content.push(
            text("Alt + Tab - Task Switcher")
                .size(24)
        );

        // Instructions
        content = content.push(
            text("Use Tab to cycle through windows, release Alt to switch")
                .size(14)
        );

        // List of launcher items (windows) in alt-tab mode
        if self.launcher_items.is_empty() {
            content = content.push(text("No windows open").size(16));
        } else {
            let mut window_elements: Vec<Element<Message>> = Vec::new();
            
            for (idx, item) in self.launcher_items.iter().enumerate() {
                let is_selected = self.active == Some(idx);
                println!("DEBUG: Rendering item {} - '{}', selected: {}", idx, item.name, is_selected);
                
                // Use reusable window item element
                let window_item = self.create_window_item_element(item, idx, is_selected);
                window_elements.push(window_item);
            }
            
            // Create grid layout with 2 columns for window items in a wider container
            let grid = self.create_grid_layout(window_elements, 2);
            content = content.push(
                container(grid)
                    .width(Length::Fixed(1100.0)) // Wide container for window grid
                    .padding(20)
                    .class(cosmic::theme::Container::Card)
            );
        }

        // Single container wrapper with top margin to move away from screen edge
        container(content)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding([80, 20, 20, 20]) // top, right, bottom, left - moved down from top
            .into()
    }

    fn view_super_launcher(&self) -> Element<Message> {
        let mut content = column![]
            .spacing(15)
            .align_x(Alignment::Center);

        // Search field at the top with background
        content = content.push(
            container(
                column![
                    text("Launcher").size(24),
                    text_input::search_input(fl!("type-to-search"), &self.input_value)
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
                println!("DEBUG: Super launcher rendering item {} - '{}', selected: {}", idx, item.name, is_selected);
                
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
                        if self.input_value.trim().is_empty() { 1100.0 } else { 1000.0 }
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
}

