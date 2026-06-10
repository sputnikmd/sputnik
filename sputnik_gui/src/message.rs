#[derive(Debug, Clone)]
pub enum Message {
    Window(WindowMessage),
    None,
}

#[derive(Debug, Clone)]
pub enum WindowMessage {
    InitializedMainWindow,
    Close(iced::window::Id),
}
