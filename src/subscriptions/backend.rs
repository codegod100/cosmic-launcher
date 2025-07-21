


use tokio::sync::mpsc;
use cosmic::iced::Subscription;
use crate::backend::Event;
use async_stream::stream;

pub fn subscription(
    mut receiver: Option<mpsc::UnboundedReceiver<Event>>,
) -> Subscription<Event> {
    Subscription::run_with_id(
        "backend-subscription",
        stream! {
            if let Some(ref mut rx) = receiver {
                while let Some(event) = rx.recv().await {
                    yield event;
                }
            }
        },
    )
}


