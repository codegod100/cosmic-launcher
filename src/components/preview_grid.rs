use cosmic::iced_core::{
    event::{self, Event},
    layout, mouse, overlay, renderer,
    widget::{tree::Tag, Operation, Tree},
    Alignment, Clipboard, Element, Layout, Length, Padding, Pixels, Rectangle, Shell, Size, Vector,
    Widget,
};
use cosmic::{
    widget::{button, container, image, text, Row},
    Element as CosmicElement, Renderer, Theme,
};
use pop_launcher::SearchResult;
use std::collections::HashMap;

pub struct WindowPreview {
    pub window_id: Option<String>,
    pub title: String,
    pub description: String,
    pub screenshot: Option<cosmic::widget::image::Handle>,
    pub selected: bool,
}

impl WindowPreview {
    pub fn from_search_result(result: &SearchResult, selected: bool) -> Self {
        let window_id = result.window.clone();
        let (title, description) = if result.window.is_some() {
            (result.description.clone(), result.name.clone())
        } else {
            (result.name.clone(), result.description.clone())
        };

        Self {
            window_id,
            title,
            description,
            screenshot: None,
            selected,
        }
    }

    pub fn with_screenshot(mut self, screenshot: cosmic::widget::image::Handle) -> Self {
        self.screenshot = Some(screenshot);
        self
    }
}

#[derive(Debug, Clone)]
pub enum PreviewMessage {
    WindowSelected(usize),
    WindowActivated(usize),
}

pub struct PreviewGrid<'a> {
    previews: Vec<WindowPreview>,
    selected_index: usize,
    columns: usize,
    thumbnail_size: (f32, f32),
    spacing: f32,
    padding: Padding,
    width: Length,
    height: Length,
    on_select: Option<Box<dyn Fn(usize) -> PreviewMessage + 'a>>,
    on_activate: Option<Box<dyn Fn(usize) -> PreviewMessage + 'a>>,
}

impl<'a> PreviewGrid<'a> {
    pub fn new(previews: Vec<WindowPreview>) -> Self {
        Self {
            previews,
            selected_index: 0,
            columns: 3,
            thumbnail_size: (256.0, 144.0),
            spacing: 16.0,
            padding: Padding::from(16),
            width: Length::Fill,
            height: Length::Fill,
            on_select: None,
            on_activate: None,
        }
    }

    pub fn selected_index(mut self, index: usize) -> Self {
        self.selected_index = index.min(self.previews.len().saturating_sub(1));
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = columns.max(1);
        self
    }

    pub fn thumbnail_size(mut self, width: f32, height: f32) -> Self {
        self.thumbnail_size = (width, height);
        self
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn padding<P: Into<Padding>>(mut self, padding: P) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn on_select<F>(mut self, f: F) -> Self
    where
        F: Fn(usize) -> PreviewMessage + 'a,
    {
        self.on_select = Some(Box::new(f));
        self
    }

    pub fn on_activate<F>(mut self, f: F) -> Self
    where
        F: Fn(usize) -> PreviewMessage + 'a,
    {
        self.on_activate = Some(Box::new(f));
        self
    }

    fn create_preview_widget(&self, preview: &WindowPreview, index: usize) -> CosmicElement<PreviewMessage> {
        let is_selected = index == self.selected_index;
        
        let thumbnail_content = if let Some(ref handle) = preview.screenshot {
            image(handle.clone())
                .width(self.thumbnail_size.0)
                .height(self.thumbnail_size.1)
                .into()
        } else {
            // Placeholder when no screenshot available
            container(
                text("No Preview")
                    .size(14)
                    .color(cosmic::theme::palette::Extended::bright_white())
            )
            .width(self.thumbnail_size.0)
            .height(self.thumbnail_size.1)
            .center_x()
            .center_y()
            .style(cosmic::theme::Container::custom(|_theme| {
                container::Style {
                    background: Some(cosmic::iced::Background::Color(
                        cosmic::iced::Color::from_rgb(0.2, 0.2, 0.2)
                    )),
                    border: cosmic::iced::Border {
                        color: cosmic::iced::Color::from_rgb(0.4, 0.4, 0.4),
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..Default::default()
                }
            }))
            .into()
        };

        let title_text = text(preview.title.clone())
            .size(12)
            .color(if is_selected {
                cosmic::theme::palette::Extended::bright_white()
            } else {
                cosmic::theme::palette::Extended::light_gray()
            });

        let preview_content = cosmic::widget::column![
            thumbnail_content,
            title_text
        ]
        .spacing(8)
        .align_items(Alignment::Center);

        let container_style = if is_selected {
            cosmic::theme::Container::custom(|_theme| {
                container::Style {
                    background: Some(cosmic::iced::Background::Color(
                        cosmic::iced::Color::from_rgba(0.3, 0.5, 0.8, 0.3)
                    )),
                    border: cosmic::iced::Border {
                        color: cosmic::iced::Color::from_rgb(0.4, 0.6, 1.0),
                        width: 2.0,
                        radius: 8.0.into(),
                    },
                    ..Default::default()
                }
            })
        } else {
            cosmic::theme::Container::custom(|_theme| {
                container::Style {
                    background: Some(cosmic::iced::Background::Color(
                        cosmic::iced::Color::from_rgba(0.1, 0.1, 0.1, 0.5)
                    )),
                    border: cosmic::iced::Border {
                        color: cosmic::iced::Color::from_rgb(0.3, 0.3, 0.3),
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..Default::default()
                }
            })
        };

        let preview_button = button(preview_content)
            .padding(12)
            .width(Length::Fixed(self.thumbnail_size.0 + 24.0))
            .on_press(PreviewMessage::WindowSelected(index))
            .style(cosmic::theme::Button::Custom {
                active: Box::new(|_theme| {
                    button::Style {
                        background: Some(cosmic::iced::Background::Color(
                            cosmic::iced::Color::TRANSPARENT
                        )),
                        border: cosmic::iced::Border::default(),
                        ..Default::default()
                    }
                }),
                hovered: Box::new(|_theme| {
                    button::Style {
                        background: Some(cosmic::iced::Background::Color(
                            cosmic::iced::Color::from_rgba(1.0, 1.0, 1.0, 0.1)
                        )),
                        border: cosmic::iced::Border::default(),
                        ..Default::default()
                    }
                }),
                pressed: Box::new(|_theme| {
                    button::Style {
                        background: Some(cosmic::iced::Background::Color(
                            cosmic::iced::Color::from_rgba(1.0, 1.0, 1.0, 0.2)
                        )),
                        border: cosmic::iced::Border::default(),
                        ..Default::default()
                    }
                }),
                disabled: Box::new(|_theme| {
                    button::Style::default()
                }),
            });

        container(preview_button)
            .style(container_style)
            .into()
    }

    pub fn build_grid(&self) -> CosmicElement<PreviewMessage> {
        let rows = (self.previews.len() + self.columns - 1) / self.columns;
        
        let mut grid_rows = Vec::new();
        
        for row in 0..rows {
            let mut row_widgets = Vec::new();
            
            for col in 0..self.columns {
                let index = row * self.columns + col;
                if index < self.previews.len() {
                    row_widgets.push(self.create_preview_widget(&self.previews[index], index));
                } else {
                    // Empty space for incomplete rows
                    row_widgets.push(
                        container(cosmic::widget::Space::new(
                            self.thumbnail_size.0 + 24.0,
                            self.thumbnail_size.1 + 48.0
                        ))
                        .into()
                    );
                }
            }
            
            let row_element = Row::with_children(row_widgets)
                .spacing(self.spacing)
                .align_items(Alignment::Center);
            
            grid_rows.push(row_element.into());
        }
        
        cosmic::widget::column(grid_rows)
            .spacing(self.spacing)
            .padding(self.padding)
            .align_items(Alignment::Center)
            .width(self.width)
            .height(self.height)
            .into()
    }
}

pub fn preview_grid<'a>(previews: Vec<WindowPreview>) -> PreviewGrid<'a> {
    PreviewGrid::new(previews)
}