use iced::keyboard::Key;

#[derive(Debug, Clone)]
pub enum Message {
    Window(WindowMessage),
    KeyboardInput(Key),
    None,
}

#[derive(Debug, Clone)]
pub enum WindowMessage {
    InitializedMainWindow,
    Close(iced::window::Id),
}
