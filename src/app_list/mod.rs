// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

pub mod app;
pub mod localize;
pub mod utils;
pub mod wayland_handler;
pub mod wayland_subscription;

pub use wayland_subscription::{WaylandUpdate, ToplevelUpdate, OutputUpdate, WaylandRequest, ToplevelRequest, WaylandImage};

use localize::localize;

pub fn run() -> cosmic::iced::Result {
    localize();
    app::run()
}
