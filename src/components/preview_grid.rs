use cosmic::cctk::toplevel_info::ToplevelInfo as TopLevel;
use cosmic::{
    iced::widget::{button, image},
    Element as CosmicElement,
};
use cosmic::{
    iced::{Alignment, Length},
    widget::{column, container, row, text},
};

pub struct WindowPreview {
    pub toplevel: TopLevel,
    pub selected: bool,
    pub screenshot: Option<image::Handle>,
}

impl WindowPreview {
    pub fn new(toplevel: TopLevel, selected: bool) -> Self {
        Self {
            toplevel,
            selected,
            screenshot: None,
        }
    }

    pub fn with_screenshot(mut self, screenshot: image::Handle) -> Self {
        self.screenshot = Some(screenshot);
        self
    }
}

#[derive(Debug, Clone)]
pub enum PreviewMessage {
    WindowSelected(usize),
    WindowActivated(usize),
}

pub fn create_preview_grid(
    previews: Vec<WindowPreview>,
    _selected_index: usize,
) -> CosmicElement<'static, PreviewMessage> {
    let columns = 2;
    let thumbnail_size = (400.0, 200.0);
    let spacing = 8.0;

    let rows = (previews.len() + columns - 1) / columns;
    let mut grid_rows: Vec<cosmic::Element<PreviewMessage>> = Vec::new();

    for row_idx in 0..rows {
        let mut row_widgets: Vec<cosmic::Element<PreviewMessage>> = Vec::new();

        for col_idx in 0..columns {
            let index = row_idx * columns + col_idx;
            if index < previews.len() {
                let preview = &previews[index];
                let is_selected = preview.selected;

                let thumbnail_content = if let Some(handle) = preview.screenshot.clone() {
                    container(image::viewer(handle))
                } else {
                    container(text("Loading..."))
                }
                .width(thumbnail_size.0)
                .height(thumbnail_size.1)
                .center_x(Length::Fill)
                .center_y(Length::Fill);

                let title_text = text(preview.toplevel.title.clone()).size(12);

                let preview_button = button(
                    {
                        let mut col = column();
                        col = col.push(thumbnail_content);
                        col = col.push(title_text);
                        col.spacing(4).align_x(Alignment::Center)
                    }
                        .align_x(Alignment::Center),
                )
                .on_press(PreviewMessage::WindowActivated(index));                row_widgets.push(
                    container(preview_button)
                        .padding(8)
                        .width(Length::Fixed(thumbnail_size.0 + 16.0))
                        .into(),
                );
            }
        }

        let mut row_element = row();
        for widget in row_widgets {
            row_element = row_element.push(widget);
        }
        row_element = row_element.spacing(spacing);

        grid_rows.push(row_element.into());
    }

    {
        let mut col = column();
        for row in grid_rows {
            col = col.push(row);
        }
        col
    }
        .spacing(spacing)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
