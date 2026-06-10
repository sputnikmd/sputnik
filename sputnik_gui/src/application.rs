use iced::widget::{center, text};
use iced::{Element, Subscription, Task};

use tracing::{debug, info};

use crate::APP_ICON;
use crate::message::{self, Message};

pub struct Application {
    window_id: iced::window::Id,
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

        (
            Self {
                window_id: main_window_id,
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
                    // Close an entire application if we trying to close main window
                    if id == self.window_id {
                        close_task = close_task.chain(self.exit());
                    }
                    return close_task;
                }
            },
            Message::None => {}
        }

        Task::none()
    }

    pub fn view(&self, _window_id: iced::window::Id) -> Element<'_, Message> {
        center(text("Nothing here...")).into()
    }

    pub fn title(&self, _window_id: iced::window::Id) -> String {
        String::from("Sputnik")
    }

    fn exit(&mut self) -> Task<Message> {
        info!("Closing application gracefully");

        iced::exit()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let tasks = vec![
            iced::window::close_requests()
                .map(|id| Message::Window(message::WindowMessage::Close(id))),
        ];
        Subscription::batch(tasks)
    }
}
