use cosmic::{
    widget::{container, image, text, row, column},
    Element as CosmicElement, iced::Length,
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

pub fn create_preview_grid(previews: Vec<WindowPreview>, selected_index: usize) -> CosmicElement<'static, PreviewMessage> {
    let columns = 3;
    let thumbnail_size = (256.0, 144.0);
    let spacing = 16.0;
    
    let rows = (previews.len() + columns - 1) / columns;
    let mut grid_rows = Vec::new();
    
    for row in 0..rows {
        let mut row_widgets = Vec::new();
        
        for col in 0..columns {
            let index = row * columns + col;
            if index < previews.len() {
                let preview = &previews[index];
                let is_selected = index == selected_index;
                
                let thumbnail_content = if let Some(ref handle) = preview.screenshot {
                    image(handle.clone())
                        .width(thumbnail_size.0)
                        .height(thumbnail_size.1)
                        .into()
                } else {
                    container(
                        text("No Preview")
                            .size(14)
                    )
                    .width(thumbnail_size.0)
                    .height(thumbnail_size.1)
                    .center_x(Length::Shrink)
                    .center_y(Length::Shrink)
                    .into()
                };

                let title_text = text(preview.title.clone())
                    .size(12);

                let preview_content = cosmic::widget::column::with_children(vec![
                    thumbnail_content,
                    title_text.into(),
                ]);

                let preview_container = container(preview_content)
                    .padding(12)
                    .width(Length::Fixed(thumbnail_size.0 + 24.0));

                let widget = if is_selected {
                    container(preview_container)
                        .padding(2)
                        .into()
                } else {
                    preview_container.into()
                };
                
                row_widgets.push(widget);
            }
        }
        
        let row_element = cosmic::widget::row::with_children(row_widgets);
        
        grid_rows.push(row_element.into());
    }
    
    let grid_column = cosmic::widget::column::with_children(grid_rows)
        .width(Length::Fill)
        .height(Length::Fill);
    
    grid_column.into()
}