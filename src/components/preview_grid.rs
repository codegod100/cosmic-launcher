use cosmic::{
    widget::{button, container, image, text, row, column},
    Element as CosmicElement, iced::{Alignment, Length},
};
use pop_launcher::SearchResult;

pub struct WindowPreview {
    pub window_id: Option<(u32, u32)>,
    pub title: String,
    pub description: String,
    pub screenshot: Option<cosmic::widget::image::Handle>,
    pub selected: bool,
}

impl WindowPreview {
    pub fn from_search_result(result: &SearchResult, selected: bool) -> Self {
        let window_id = result.window;
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
            )
            .width(self.thumbnail_size.0)
            .height(self.thumbnail_size.1)
            .center_x(Length::Shrink)
            .center_y(Length::Shrink)
            .into()
        };

        let title_text = text(preview.title.clone())
            .size(12);

        let preview_content = column()
            .push(thumbnail_content)
            .push(title_text)
            .spacing(8)
            .align_items(Alignment::Center);

        let preview_button = button::standard(preview_content)
            .padding(12)
            .width(Length::Fixed(self.thumbnail_size.0 + 24.0))
            .on_press(PreviewMessage::WindowSelected(index));

        if is_selected {
            container(preview_button)
                .padding(2)
                .into()
        } else {
            preview_button.into()
        }
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
                }
            }
            
            let row_element = row()
                .extend(row_widgets)
                .spacing(self.spacing);
            
            grid_rows.push(row_element.into());
        }
        
        column()
            .extend(grid_rows)
            .spacing(self.spacing)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

pub fn preview_grid<'a>(previews: Vec<WindowPreview>) -> PreviewGrid<'a> {
    PreviewGrid::new(previews)
}