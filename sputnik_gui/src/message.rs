use std::io;
use std::sync::Arc;

use iced::keyboard::Key;

#[derive(Debug, Clone)]
pub enum Message {
    Window(WindowMessage),
    KeyboardInput {
        key: Key,
        text: Option<smol_str::SmolStr>,
    },
    FileOpened(Result<Arc<String>, io::ErrorKind>),
    None,
}

#[derive(Debug, Clone)]
pub enum WindowMessage {
    InitializedMainWindow,
    Close(iced::window::Id),
}
