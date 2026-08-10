use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use iced::keyboard::{Key, Modifiers};

use sputnik_editor::Interaction;

#[derive(Debug, Clone)]
pub enum Message {
    Window(WindowMessage),
    KeyboardInput {
        key: Key,
        modifiers: Modifiers,
        text: Option<smol_str::SmolStr>,
    },
    /// Something the editor widget noticed. What it *means* is decided in
    /// `update`, not by the widget.
    Editor(Interaction),
    RequestOpenFile,
    OpenFile(PathBuf),
    FileOpened(Result<Arc<String>, (PathBuf, io::ErrorKind)>),
    None,
}

#[derive(Debug, Clone)]
pub enum WindowMessage {
    InitializedMainWindow,
    Close(iced::window::Id),
}
