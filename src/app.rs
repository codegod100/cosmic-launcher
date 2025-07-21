use crate::{app::iced::event::listen_raw, components, fl, screenshot::ScreenshotManager, subscriptions::launcher};
use crate::wayland_subscription::{WaylandUpdate, ToplevelUpdate, WaylandImage, wayland_subscription};
use cosmic::cctk::toplevel_info::ToplevelInfo;
use cosmic::cctk::wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use clap::Parser;
use cosmic::app::{Core, CosmicFlags, Settings, Task};
use cosmic::cctk::sctk;
use cosmic::dbus_activation::Details;
use cosmic::iced::alignment::{Horizontal, Vertical};
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
use cosmic::iced::widget::{Column, column, container, image::{Handle, Image}};
use cosmic::iced::{self, Length, Size, Subscription};
use cosmic::iced_core::keyboard::key::Named;
use cosmic::iced_core::widget::operation;
use cosmic::iced_core::{Border, Padding, Point, Rectangle, Shadow, window};
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
use cosmic::theme::{self, Button, Container};
use cosmic::widget::icon::{IconFallback, from_name};
use cosmic::widget::id_container;
use cosmic::widget::{
    autosize, button, divider, horizontal_space, icon, mouse_area, scrollable, text,
    text_input::{self, StyleSheet as TextInputStyleSheet},
    vertical_space,
};
use cosmic::{Element, keyboard_nav};
use cosmic::{iced_runtime, surface};
use iced::keyboard::Key;
use iced::{Alignment, Color};
use pop_launcher::{ContextOption, GpuPreference, IconSource, SearchResult};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::sync::LazyLock;
use std::{
    collections::{HashMap, VecDeque},
    rc::Rc,
    str::FromStr,
    time::Instant,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LauncherState {
    Search,    // Normal search/launcher mode
    AltTab,    // Alt+Tab task switcher mode
}

pub struct CosmicLauncher {
    core: Core,
    input_value: String,
    surface_state: SurfaceState,
    launcher_state: LauncherState,
    launcher_items: Vec<SearchResult>,
    tx: Option<mpsc::Sender<launcher::Request>>,
    menu: Option<(u32, Vec<ContextOption>)>,
    cursor_position: Option<Point<f32>>,
    focused: usize,
    last_hide: Instant,
    alt_tab: bool,
    window_id: window::Id,
    queue: VecDeque<Message>,
    result_ids: Vec<Id>,
    overlap: HashMap<String, Rectangle>,
    margin: f32,
    height: f32,
    needs_clear: bool,

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
    TabPress,
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
    Overlap(OverlapNotifyEvent),
    Surface(surface::Action),
    PreviewAction(components::preview_grid::PreviewMessage),

    BackendEvent(WaylandUpdate),
}

impl CosmicLauncher {
    fn request(&self, r: launcher::Request) {
        debug!("request: {:?}", r);
        println!("DEBUG: Sending request to pop-launcher: {:?}", r);
        if let Some(tx) = &self.tx {
            info!("Sending request to pop-launcher: {:?}", r);
            println!("DEBUG: TX found, sending request");
            if let Err(e) = tx.blocking_send(r) {
                error!("Failed to send request to pop-launcher: {e}");
                println!("ERROR: Failed to send request: {}", e);
            } else {
                info!("Request sent successfully to pop-launcher");
                println!("DEBUG: Request sent successfully");
            }
        } else {
            error!("tx not found - pop-launcher service not connected!");
            println!("ERROR: TX not found - pop-launcher service not connected!");
        }
    }

    fn show(&mut self) -> Task<Message> {
        self.surface_state = SurfaceState::Visible;
        self.needs_clear = true;

        Task::batch(vec![
            get_layer_surface(SctkLayerSurfaceSettings {
                id: self.window_id,
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                anchor: Anchor::TOP,
                namespace: "launcher".into(),
                size: Some((Some(600), Some(1600))),
                size_limits: Limits::NONE.min_width(600.0).min_height(1600.0).max_width(600.0).max_height(1600.0),
                exclusive_zone: -1,
                ..Default::default()
            }),
            overlap_notify(self.window_id, true),
        ])
    }

    fn hide(&mut self) -> Task<Message> {
        self.input_value.clear();
        self.focused = 0;
        self.alt_tab = false;
        self.queue.clear();

        self.request(launcher::Request::Close);

        let mut tasks = Vec::new();

        if self.surface_state == SurfaceState::Visible {
            tasks.push(destroy_layer_surface(self.window_id));
            if self.menu.take().is_some() {
                tasks.push(commands::popup::destroy_popup(*MENU_ID));
            }
        }

        self.surface_state = SurfaceState::Hidden;

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
        println!("DEBUG: Looking for screenshot for item: '{}' (window: {:?})", item.name, item.window.is_some());
        println!("DEBUG: Have {} toplevel_captures", self.toplevel_captures.len());
        println!("DEBUG: Have {} toplevels", self.toplevels.len());
        
        // If this launcher item represents a window, try to find matching screenshot
        if item.window.is_some() {
            // Try to match by window title/name with toplevels
            for (i, (handle, capture_image)) in self.toplevel_captures.iter().enumerate() {
                println!("DEBUG: Checking capture {} with handle: {:?}", i, handle);
                // Find corresponding toplevel info
                if let Some(toplevel_info) = self.toplevels.iter().find(|t| t.foreign_toplevel == *handle) {
                    println!("DEBUG: Found toplevel info with title: '{}'", toplevel_info.title);
                    // Match by title (item.description often contains the window title for windows)
                    if item.description.contains(&toplevel_info.title) 
                        || toplevel_info.title.contains(&item.description)
                        || item.name.contains(&toplevel_info.title)
                        || toplevel_info.title.contains(&item.name) {
                        println!("DEBUG: Match found! Using screenshot for: {}", item.name);
                        return Some(capture_image);
                    } else {
                        println!("DEBUG: No match between '{}' and '{}'", item.description, toplevel_info.title);
                    }
                } else {
                    println!("DEBUG: No toplevel info found for handle: {:?}", handle);
                }
            }
        } else {
            println!("DEBUG: Item '{}' is not a window", item.name);
        }
        println!("DEBUG: No screenshot found for item: {}", item.name);
        None
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
                launcher_state: LauncherState::Search,
                launcher_items: Vec::new(),
                tx: None,
                menu: None,
                cursor_position: None,
                focused: 0,
                last_hide: Instant::now(),
                alt_tab: false,
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
                self.input_value.clone_from(&value);
                self.request(launcher::Request::Search(value));
            }
            Message::Backspace => {
                self.input_value.pop();
                self.request(launcher::Request::Search(self.input_value.clone()));
            }
            Message::TabPress if self.launcher_state == LauncherState::Search => {
                let focused = self.focused;
                self.focused = 0;
                return cosmic::task::message(cosmic::Action::App(
                    Self::Message::CompleteFocusedId(self.result_ids[focused].clone()),
                ));
            }
            Message::TabPress if self.launcher_state == LauncherState::AltTab => {
                // Cycle to next window in Alt-Tab mode
                if !self.launcher_items.is_empty() {
                    let current = self.active.unwrap_or(0);
                    let next = (current + 1) % self.launcher_items.len();
                    self.active = Some(next);
                    info!("Alt+Tab: cycling to window {}: {}", next, 
                          self.launcher_items.get(next).map(|t| &t.name).unwrap_or(&"Unknown".to_string()));
                }
            }
            Message::CompleteFocusedId(id) => {
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
                if let Some(item) = self.launcher_items.get(i.unwrap_or(self.focused)) {
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
                    println!("DEBUG: Pop-launcher service started and connected");
                    self.tx.replace(tx);
                    info!("Sending initial search request to pop-launcher");
                    println!("DEBUG: Sending initial search request");
                    self.request(launcher::Request::Search(self.input_value.clone()));
                }
                launcher::Event::ServiceIsClosed => {
                    info!("Pop-launcher service closed");
                    println!("DEBUG: Pop-launcher service closed");
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
                        println!("DEBUG: Received launcher response with {} items", list.len());
                        info!("Received launcher response with {} items", list.len());
                        
                        // Log the first few items for debugging
                        for (i, item) in list.iter().take(3).enumerate() {
                            info!("Item {}: name='{}', description='{}', window={:?}", 
                                  i, item.name, item.description, item.window.is_some());
                            println!("DEBUG: Item {}: name='{}', description='{}', window={:?}", 
                                  i, item.name, item.description, item.window.is_some());
                        }
                        
                        if self.alt_tab && list.is_empty() {
                            info!("Alt-tab mode but list is empty, hiding launcher");
                            println!("DEBUG: Alt-tab mode but list is empty, hiding launcher");
                            return self.hide();
                        }
                        if self.alt_tab || self.input_value.is_empty() {
                            info!("Reversing list (alt_tab={}, empty_input={})", 
                                  self.alt_tab, self.input_value.is_empty());
                            println!("DEBUG: Reversing list (alt_tab={}, empty_input={})", 
                                  self.alt_tab, self.input_value.is_empty());
                            list.reverse();
                        }
                        list.sort_by(|a, b| {
                            let a = i32::from(a.window.is_none());
                            let b = i32::from(b.window.is_none());
                            a.cmp(&b)
                        });

                        info!("Setting launcher_items to {} items", list.len());

                        self.launcher_items.splice(.., list);
                        if self.result_ids.len() < self.launcher_items.len() {
                            self.result_ids.extend(
                                (self.result_ids.len()..self.launcher_items.len())
                                    .map(|id| Id::new((id).to_string()))
                                    .collect::<Vec<_>>(),
                            );
                        }

                        // Update screenshots for alt-tab mode and set active window
                        if self.alt_tab && self.launcher_state == LauncherState::AltTab {
                            info!("In alt-tab mode, current active: {:?}", self.active);
                            // Set initial active window for alt-tab mode
                            if self.active.is_none() && !self.launcher_items.is_empty() {
                                info!("Setting initial active window for alt-tab");
                                // For ShiftAltTab, start from the last item
                                if self.launcher_items.len() > 1 {
                                    self.active = Some(self.launcher_items.len() - 1);
                                } else {
                                    self.active = Some(0);
                                }
                            }
                            info!("Alt+Tab mode: {} windows available", self.launcher_items.len());
                            if let Some(active_idx) = self.active {
                                if let Some(item) = self.launcher_items.get(active_idx) {
                                    info!("Currently selected window: {}", item.name);
                                }
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
                        return Task::batch(cmds);
                    }
                    pop_launcher::Response::Fill(s) => {
                        self.input_value = s;
                        self.request(launcher::Request::Search(self.input_value.clone()));
                    }
                },
            },
            Message::Layer(e) => match e {
                LayerEvent::Focused | LayerEvent::Done => {}
                LayerEvent::Unfocused => {
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

            Message::BackendEvent(event) => {
                println!("DEBUG: Received backend event: {:?}", 
                         std::mem::discriminant(&event));
                match event {
                WaylandUpdate::Toplevel(toplevel_update) => {
                    match toplevel_update {
                        ToplevelUpdate::Add(info) => {
                            println!("DEBUG: New toplevel - title: '{}'", info.title);
                            self.toplevels.push(info);
                        }
                        ToplevelUpdate::Update(info) => {
                            println!("DEBUG: Update toplevel - title: '{}'", info.title);
                            if let Some(t) = self
                                .toplevels
                                .iter_mut()
                                .find(|t| t.foreign_toplevel == info.foreign_toplevel)
                            {
                                *t = info;
                            }
                        }
                        ToplevelUpdate::Remove(handle) => {
                            println!("DEBUG: Close toplevel - handle: {:?}", handle);
                            self.toplevels.retain(|t| t.foreign_toplevel != handle);
                        }
                    }
                }
                WaylandUpdate::Image(handle, wayland_image) => {
                    // Store screenshot for the toplevel
                    println!("DEBUG: Storing screenshot for toplevel: {:?}", handle);
                    info!("Storing screenshot for toplevel: {:?}", handle);
                    self.toplevel_captures.insert(handle, wayland_image);
                }
                WaylandUpdate::Init => {
                    println!("DEBUG: Wayland init event");
                    // TODO: handle wayland init if needed
                }
                WaylandUpdate::Finished => {
                    println!("DEBUG: Wayland finished event");
                    // TODO: handle wayland finished if needed
                }
            }
            },
            Message::AltTab => {
                // Show the launcher in Alt-Tab mode
                println!("DEBUG: Alt+Tab pressed - switching to task switcher mode");
                info!("Alt+Tab pressed - switching to task switcher mode");
                info!("Current launcher_items count: {}", self.launcher_items.len());
                info!("Current surface_state: {:?}", self.surface_state);
                info!("Current launcher_state: {:?}", self.launcher_state);
                println!("DEBUG: Current launcher_items count: {}", self.launcher_items.len());
                println!("DEBUG: Current surface_state: {:?}", self.surface_state);
                
                // Set to alt-tab mode and enable alt_tab flag
                self.launcher_state = LauncherState::AltTab;
                self.alt_tab = true;
                
                // Clear input and request window list through search
                self.input_value.clear();
                info!("Requesting window list from pop-launcher with empty search");
                println!("DEBUG: Requesting window list from pop-launcher with empty search");
                self.request(launcher::Request::Search(String::new()));
                
                // Show the surface if hidden
                if self.surface_state == SurfaceState::Hidden {
                    info!("Surface was hidden, setting to WaitingToBeShown");
                    println!("DEBUG: Surface was hidden, setting to WaitingToBeShown");
                    self.surface_state = SurfaceState::WaitingToBeShown;
                } else {
                    info!("Surface state is already: {:?}", self.surface_state);
                    println!("DEBUG: Surface state is already: {:?}", self.surface_state);
                }
                
                // Select first available toplevel will be handled in the Response::Update
                self.active = Some(0);
                info!("Set active window to index 0");
                println!("DEBUG: Set active window to index 0");
            }
            Message::ShiftAltTab => {
                // Show the launcher in Alt-Tab mode and go backwards
                info!("Shift+Alt+Tab pressed - switching to task switcher mode (reverse)");
                
                // Set to alt-tab mode and enable alt_tab flag
                self.launcher_state = LauncherState::AltTab;
                self.alt_tab = true;
                
                // Clear input and request window list through search
                self.input_value.clear();
                self.request(launcher::Request::Search(String::new()));
                
                // Show the surface if hidden
                if self.surface_state == SurfaceState::Hidden {
                    self.surface_state = SurfaceState::WaitingToBeShown;
                }
                
                // Select last available toplevel (will be handled in Response::Update)
                self.active = None; // Will be set to last item when list is populated
            }
            Message::AltRelease => {
                // If we're in Alt-Tab mode, activate the selected window and hide launcher
                if self.launcher_state == LauncherState::AltTab {
                    info!("Alt released - activating selected window");
                    
                    if let Some(active_idx) = self.active {
                        if let Some(item) = self.launcher_items.get(active_idx) {
                            info!("Activating window: {}", item.name);
                            // Use the existing activation system
                            self.request(launcher::Request::Activate(item.id));
                        }
                    }
                    
                    // Reset state and hide launcher
                    self.launcher_state = LauncherState::Search;
                    self.alt_tab = false;
                    return self.hide();
                }
            }
            Message::TabPress => {
                // Handle Tab press during Alt+Tab mode to cycle through windows
                if self.launcher_state == LauncherState::AltTab && !self.launcher_items.is_empty() {
                    let current = self.active.unwrap_or(0);
                    let next = (current + 1) % self.launcher_items.len();
                    self.active = Some(next);
                    info!("Tab pressed - cycling to window {}", next);
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
                    self.request(launcher::Request::Search(String::new()));

                    self.surface_state = SurfaceState::WaitingToBeShown;
                    return Task::none();
                }
            }
            Details::ActivateAction { action, .. } => {
                debug!("ActivateAction {}", action);

                let Ok(cmd) = LauncherTasks::from_str(&action) else {
                    return Task::none();
                };

                if self.surface_state == SurfaceState::Hidden {
                    self.surface_state = SurfaceState::WaitingToBeShown;
                }

                match cmd {
                    LauncherTasks::AltTab => {
                        return self.update(Message::AltTab);
                    }
                    LauncherTasks::ShiftAltTab => {
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
        println!("DEBUG: view_window called with id: {:?}, my window_id: {:?}", id, self.window_id);
        if id == self.window_id {
            // Safety check to prevent overflow in surface sizing
            if !self.height.is_finite() || self.height > 10000.0 || self.height < 1.0 {
                println!("DEBUG: Height issue, showing loading");
                return container(text("Loading..."))
                    .width(Length::Fixed(400.0))
                    .height(Length::Fixed(100.0))
                    .into();
            }

            // Show different UI based on launcher state
            println!("DEBUG: Current launcher_state: {:?}", self.launcher_state);
            match self.launcher_state {
                LauncherState::AltTab => {
                    println!("DEBUG: Rendering AltTab view");
                    self.view_alt_tab()
                },
                LauncherState::Search => {
                    println!("DEBUG: Rendering Search view");
                    self.view_search()
                },
            }
        } else {
            container(text(""))
                .width(Length::Fixed(1.0))
                .height(Length::Fixed(1.0))
                .into()
        }
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        println!("DEBUG: Setting up subscriptions for cosmic-launcher");
        info!("Setting up subscriptions for cosmic-launcher");
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
                    key: Key::Named(Named::Alt | Named::Super),
                    ..
                }) => Some(Message::AltRelease),
                cosmic::iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    ..
                }) => match key {
                    Key::Character(c) => {
                        println!("DEBUG: Key pressed: '{}', alt: {}, shift: {}", c, modifiers.alt(), modifiers.shift());
                        if c == "a" && modifiers.alt() && !modifiers.shift() {
                            println!("DEBUG: Alt+A detected, triggering AltTab");
                            Some(Message::AltTab)
                        } else if c == "A" && modifiers.alt() && modifiers.shift() {
                            println!("DEBUG: Alt+Shift+A detected, triggering ShiftAltTab");
                            Some(Message::ShiftAltTab) 
                        } else {
                            let nums = (1..=9)
                                .map(|n| (n.to_string(), ((n + 10) % 10) - 1))
                                .chain((0..=0).map(|n| (n.to_string(), ((n + 10) % 10) - 1)))
                                .collect::<Vec<_>>();
                            nums.iter().find_map(|n| (n.0 == c).then(|| Message::Activate(Some(n.1))))
                        }
                    }
                    Key::Named(func_key @ (Named::F1 | Named::F2 | Named::F3 | Named::F4 | Named::F5 | Named::F6 | Named::F7 | Named::F8 | Named::F9 | Named::F10)) => {
                        // Handle function keys (F1=0, F2=1, etc.)
                        let key_index = match func_key {
                            Named::F1 => Some(0),
                            Named::F2 => Some(1),
                            Named::F3 => Some(2),
                            Named::F4 => Some(3),
                            Named::F5 => Some(4),
                            Named::F6 => Some(5),
                            Named::F7 => Some(6),
                            Named::F8 => Some(7),
                            Named::F9 => Some(8),
                            Named::F10 => Some(9),
                            _ => None,
                        };
                        key_index.map(|n| Message::Activate(Some(n)))
                    }
                    Key::Named(Named::ArrowUp) => {
                        Some(Message::KeyboardNav(keyboard_nav::Action::FocusPrevious))
                    }
                    Key::Named(Named::ArrowDown) => {
                        Some(Message::KeyboardNav(keyboard_nav::Action::FocusNext))
                    }
                    Key::Named(Named::Escape) => Some(Message::Hide),
                    Key::Named(Named::Tab) => {
                        println!("DEBUG: Tab key pressed");
                        Some(Message::TabPress)
                    },
                    Key::Named(Named::Backspace)
                        if matches!(status, Status::Ignored) && modifiers.is_empty() =>
                    {
                        Some(Message::Backspace)
                    }
                    _ => None,
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
    fn view_search(&self) -> Element<Message> {
        // Original search UI code - simplified for now
        container(
            column![
                text("Search Mode").size(20),
                text_input::search_input(fl!("type-to-search"), &self.input_value)
                    .on_input(Message::InputChanged)
                    .width(400)
                    .id(INPUT_ID.clone())
            ]
            .spacing(10)
            .align_x(Alignment::Center)
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    }

    fn view_alt_tab(&self) -> Element<Message> {
        println!("DEBUG: Rendering Alt-Tab view with {} launcher_items", self.launcher_items.len());
        info!("Rendering Alt-Tab view with {} launcher_items", self.launcher_items.len());
        
        let mut content = column![]
            .spacing(10)
            .align_x(Alignment::Center);

        // Title
        content = content.push(
            text("Alt + Tab - Task Switcher")
                .size(24)
                .class(cosmic::theme::Text::Default)
        );

        // Instructions
        content = content.push(
            text("Use Tab to cycle through windows, release Alt to switch")
                .size(14)
                .class(cosmic::theme::Text::Default)
        );

        // List of launcher items (windows) in alt-tab mode
        if self.launcher_items.is_empty() {
            println!("DEBUG: No launcher items found, showing 'No windows open' message");
            info!("No launcher items found, showing 'No windows open' message");
            content = content.push(text("No windows open").size(16));
        } else {
            println!("DEBUG: Rendering {} windows in alt-tab view", self.launcher_items.len());
            info!("Rendering {} windows in alt-tab view", self.launcher_items.len());
            let mut windows_column = column![].spacing(5);
            
            for (idx, item) in self.launcher_items.iter().enumerate() {
                let is_selected = self.active == Some(idx);
                println!("DEBUG: Rendering window {}: '{}' (selected: {})", idx, item.name, is_selected);
                info!("Rendering window {}: '{}' (selected: {})", idx, item.name, is_selected);
                
                // Try to get screenshot for this item
                let screenshot_widget: Element<Message> = if let Some(capture_image) = self.find_screenshot_for_item(item) {
                    println!("DEBUG: Found screenshot for window: {}", item.name);
                    info!("Found screenshot for window: {}", item.name);
                    // Use the Image widget with the WaylandImage
                    Image::new(Handle::from_rgba(
                        capture_image.width,
                        capture_image.height,
                        capture_image.img.clone(),
                    ))
                    .width(Length::Fixed(120.0))
                    .height(Length::Fixed(80.0))
                    .into()
                } else {
                    println!("DEBUG: No screenshot found for window: {}, using fallback", item.name);
                    info!("No screenshot found for window: {}, using fallback", item.name);
                    // Fallback to text placeholder
                    container(text("📄").size(48))
                        .width(Length::Fixed(120.0))
                        .height(Length::Fixed(80.0))
                        .center_x(Length::Fill)
                        .center_y(Length::Fill)
                        .into()
                };
                
                let window_item = container(
                    row![
                        screenshot_widget,
                        // Window title and info
                        column![
                            text(&item.name)
                                .size(if is_selected { 18 } else { 16 })
                                .class(if is_selected {
                                    cosmic::theme::Text::Accent
                                } else {
                                    cosmic::theme::Text::Default
                                }),
                            text(&item.description)
                                .size(12)
                                .class(cosmic::theme::Text::Default)
                        ]
                        .spacing(4)
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                )
                .padding(10)
                .width(Length::Fill)
                .class(if is_selected {
                    cosmic::theme::Container::Custom(Box::new(|theme| {
                        cosmic::iced::widget::container::Style {
                            background: Some(cosmic::iced::Color::from(theme.cosmic().accent_color()).into()),
                            text_color: Some(cosmic::iced::Color::from(theme.cosmic().on_accent_color()).into()),
                            border: Border {
                                color: cosmic::iced::Color::from(theme.cosmic().accent_color()),
                                width: 2.0,
                                radius: theme.cosmic().corner_radii.radius_s.into(),
                            },
                            ..Default::default()
                        }
                    }))
                } else {
                    cosmic::theme::Container::Card
                });
                
                windows_column = windows_column.push(window_item);
            }
            
            content = content.push(
                scrollable(windows_column)
                    .width(Length::Fixed(500.0))
                    .height(Length::Fixed(400.0))
            );
        }

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }
}

