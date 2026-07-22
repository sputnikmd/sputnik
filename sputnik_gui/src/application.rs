use std::io;
use std::sync::Arc;

use iced::event::{self, Event};
use iced::keyboard;
use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::widget::{column, container, text};
use iced::{Element, Length, Subscription, Task};
use ropey::Rope;

use tracing::{debug, error, info};

use crate::APP_ICON;
use crate::message::{self, Message};
use crate::widgets::{Action, Editor};

/// File opened on startup. Hardcoded for now — there's no file picker yet.
const INITIAL_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../README.md");

pub struct Application {
    window_id: iced::window::Id,
    editor: Editor<Message>,
}

async fn load_file(path: &'static str) -> Result<Arc<String>, io::ErrorKind> {
    tokio::fs::read_to_string(path)
        .await
        .map(Arc::new)
        .map_err(|err| err.kind())
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
            Task::perform(load_file(INITIAL_FILE), Message::FileOpened),
        ];

        (
            Self {
                window_id: main_window_id,
                editor: Editor::<Message>::new(Rope::new()),
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

            Message::FileOpened(Ok(content)) => {
                self.editor = Editor::new(Rope::from_str(&content));
            }
            Message::FileOpened(Err(err)) => {
                error!("Failed to open {INITIAL_FILE}: {err:?}");
            }

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

        // Siblings in a column, not layers in a stack: a stack would give
        // the editor and the HUD the *same* bounds, so the editor's own
        // viewport height would legitimately extend under the HUD's row
        // (that's overlap by shared bounds, not overflow — clipping the
        // editor to its own bounds wouldn't change anything). As column
        // siblings, the editor's `Fill` height excludes the HUD's `Shrink`
        // row instead.
        container(
            column![self.editor.view(), hud]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initial_file_loads_real_content() {
        let content = load_file(INITIAL_FILE)
            .await
            .expect("README.md should load");
        assert!(!content.is_empty());

        let expected = std::fs::read_to_string(INITIAL_FILE).expect("README.md should exist");
        assert_eq!(*content, expected);
    }

    #[tokio::test]
    async fn missing_file_returns_an_error() {
        let result = load_file("/nonexistent/path/for/sputnik-test.txt").await;
        assert!(result.is_err());
    }
}
