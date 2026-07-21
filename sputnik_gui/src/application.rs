use iced::event::{self, Event};
use iced::keyboard;
use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::widget::{column, container, space, stack, text};
use iced::{Element, Length, Subscription, Task};
use ropey::Rope;

use tracing::{debug, info};

use crate::APP_ICON;
use crate::message::{self, Message};
use crate::widgets::{Action, Editor};

pub struct Application {
    window_id: iced::window::Id,
    editor: Editor<Message>,
}

impl Application {
    pub fn new() -> (Self, Task<Message>) {
        let icon = iced::window::icon::from_file_data(APP_ICON, None).ok();
        let settings = iced::window::Settings {
            exit_on_close_request: false,
            icon,
            ..Default::default()
        };
        let (main_window_id, open_main_window) = iced::window::open(settings);

        let tasks = vec![
            open_main_window
                .map(|_| Message::Window(message::WindowMessage::InitializedMainWindow)),
        ];

        let content = Rope::from_str(
            "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed \ndo eiusmod tempor incididunt ut labore et dolore magna aliqua",
        );

        (
            Self {
                window_id: main_window_id,
                editor: Editor::<Message>::new(content),
            },
            Task::batch(tasks),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Window(msg) => match msg {
                message::WindowMessage::InitializedMainWindow => {
                    debug!("Main window initialized")
                }
                message::WindowMessage::Close(id) => {
                    let mut close_task = iced::window::close(id);
                    if id == self.window_id {
                        close_task = close_task.chain(self.exit());
                    }
                    return close_task;
                }
            },
            Message::KeyboardInput { key, text } => match key {
                Key::Named(Named::ArrowLeft) => {
                    self.editor.action(Action::MoveCursorLeft);
                }
                Key::Named(Named::ArrowRight) => {
                    self.editor.action(Action::MoveCursorRight);
                }
                Key::Named(Named::ArrowUp) => {
                    self.editor.action(Action::MoveCursorUp);
                }
                Key::Named(Named::ArrowDown) => {
                    self.editor.action(Action::MoveCursorDown);
                }
                Key::Named(Named::Backspace) => {
                    self.editor.action(Action::DeleteBackward);
                }
                Key::Named(Named::Delete) => {
                    self.editor.action(Action::DeleteForward);
                }
                Key::Named(Named::Tab) => {
                    self.editor.action(Action::InsertTab);
                }
                Key::Named(Named::Space) => {
                    self.editor.action(Action::Insert(' '));
                }
                Key::Named(Named::Enter) => {
                    self.editor.action(Action::Insert('\n'));
                }

                _ => {
                    if let Some(txt) = text {
                        for c in txt.chars() {
                            self.editor.action(Action::Insert(c));
                        }
                    }
                }
            },

            Message::None => {}
        }

        Task::none()
    }

    pub fn view(&self, _window_id: iced::window::Id) -> Element<'_, Message> {
        let hud: Element<'_, Message> = text(format!(
            "cursor: {}/{}",
            self.editor.cursor(),
            self.editor.total_chars(),
        ))
        .size(14.0)
        .color(iced::color!(0x666666))
        .into();

        container(
            stack([
                self.editor.view().into(),
                column![space::vertical(), hud].into(),
            ])
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .padding(32.0)
        .into()
    }

    pub fn title(&self, _window_id: iced::window::Id) -> String {
        String::from("Sputnik")
    }

    fn exit(&mut self) -> Task<Message> {
        info!("Closing application gracefully");

        iced::exit()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let tasks: Vec<Subscription<Message>> = vec![
            iced::window::close_requests()
                .map(|id| Message::Window(message::WindowMessage::Close(id))),
            event::listen().map(|event| match event {
                Event::Keyboard(keyboard::Event::KeyPressed { key, text, .. }) => {
                    Message::KeyboardInput { key, text }
                }
                _ => Message::None,
            }),
        ];

        Subscription::batch(tasks)
    }
}
