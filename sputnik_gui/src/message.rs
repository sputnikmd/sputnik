use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use iced::keyboard::Key;

#[derive(Debug, Clone)]
pub enum Message {
    Window(WindowMessage),
    KeyboardInput {
        key: Key,
        text: Option<smol_str::SmolStr>,
    },
    OpenFile(PathBuf),
    FileOpened(Result<Arc<String>, (PathBuf, io::ErrorKind)>),
    None,
}

#[derive(Debug, Clone)]
pub enum WindowMessage {
    InitializedMainWindow,
    Close(iced::window::Id),
}
