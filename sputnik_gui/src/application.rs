use std::sync::Arc;

use iced::event::{self, Event};
use iced::keyboard;
use iced::widget::center;
use iced::{Element, Subscription, Task};

use tracing::{debug, info};

use crate::APP_ICON;
use crate::message::{self, Message};
use crate::widgets::Editor;

pub struct Application<'a> {
    window_id: iced::window::Id,
    editor: Editor<'a, Message>,
}

impl<'a> Application<'a> {
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

        let content = Arc::new(String::from(
            "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua",
        ));

        (
            Self {
                window_id: main_window_id,
                editor: Editor::new(content),
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
            Message::KeyboardInput(_) => {}

            Message::None => {}
        }

        Task::none()
    }

    pub fn view(&self, _window_id: iced::window::Id) -> Element<'_, Message> {
        center(self.editor.to_element()).into()
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
                Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
                    Message::KeyboardInput(key)
                }
                _ => Message::None,
            }),
        ];

        Subscription::batch(tasks)
    }
}
