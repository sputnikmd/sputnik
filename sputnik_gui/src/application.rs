use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use iced::event::{self, Event};
use iced::keyboard;
use iced::keyboard::Key;
use iced::widget::{column, container, text};
use iced::{Element, Length, Subscription, Task};

use tracing::{debug, error, info};

use crate::APP_ICON;
use crate::keymap;
use crate::message::{self, Message};
use sputnik_editor::{Action, Document, Editor, Interaction, Motion, Text};

pub struct Application {
    window_id: iced::window::Id,
    editor: Editor,
}

async fn load_file(path: PathBuf) -> Result<Arc<String>, (PathBuf, io::ErrorKind)> {
    tokio::fs::read_to_string(&path)
        .await
        .map(Arc::new)
        .map_err(|err| (path, err.kind()))
}

async fn pick_file() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .pick_file()
        .await
        .map(|handle| handle.path().to_path_buf())
}

impl Application {
    pub fn new(file: Option<PathBuf>) -> (Self, Task<Message>) {
        let icon = iced::window::icon::from_file_data(APP_ICON, None).ok();
        let settings = iced::window::Settings {
            exit_on_close_request: false,
            icon,
            ..Default::default()
        };
        let (main_window_id, open_main_window) = iced::window::open(settings);

        let mut tasks = vec![
            open_main_window
                .map(|_| Message::Window(message::WindowMessage::InitializedMainWindow)),
        ];
        if let Some(path) = file {
            tasks.push(Task::done(Message::OpenFile(path)));
        }

        (
            Self {
                window_id: main_window_id,
                editor: Editor::default(),
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
            Message::KeyboardInput {
                key,
                modifiers,
                text,
            } => {
                if let Some(action) = keymap::action(&key, modifiers, text.as_ref()) {
                    self.editor.perform(action);
                }
            }

            // The widget reports *where* the mouse landed; deciding that a
            // press places the caret and a drag extends the selection is
            // this layer's call, and is all it would take to make the
            // mouse behave differently.
            Message::Editor(interaction) => match interaction {
                Interaction::Press(at) => self.editor.perform(Action::Move(Motion::To(at))),
                Interaction::Drag(at) => self.editor.perform(Action::Select(Motion::To(at))),
                Interaction::Release => {}
            },

            Message::RequestOpenFile => {
                return Task::perform(pick_file(), |path| match path {
                    Some(path) => Message::OpenFile(path),
                    None => Message::None,
                });
            }

            Message::OpenFile(path) => {
                return Task::perform(load_file(path), Message::FileOpened);
            }

            Message::FileOpened(Ok(content)) => {
                // Not `Editor::from_str`: the widget's measurements
                // describe the view, which has not changed, and dropping
                // them would break wrap-aware motion until the next
                // layout pass.
                self.editor.open(Document::from_str(&content));
            }
            Message::FileOpened(Err((path, err))) => {
                error!("Failed to open {}: {err:?}", path.display());
            }

            Message::None => {}
        }

        Task::none()
    }

    pub fn view(&self, _window_id: iced::window::Id) -> Element<'_, Message> {
        let selection = self.editor.selection();
        let hud: Element<'_, Message> = text(format!(
            "cursor: {}/{}{}",
            self.editor.cursor(),
            self.editor.text_storage().len(),
            if selection.is_empty() {
                String::new()
            } else {
                format!("  ({} selected)", selection.range().len())
            },
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
        let editor = self
            .editor
            .view()
            .on_interaction(Message::Editor)
            .show_line_numbers(true)
            .size(24.0);

        container(
            column![editor, hud]
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
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    text,
                    ..
                }) => match key.as_ref() {
                    Key::Character("o") if modifiers.command() => Message::RequestOpenFile,
                    _ => Message::KeyboardInput {
                        key,
                        modifiers,
                        text,
                    },
                },
                _ => Message::None,
            }),
        ];

        Subscription::batch(tasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INITIAL_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../README.md");

    #[tokio::test]
    async fn initial_file_loads_real_content() {
        let content = load_file(PathBuf::from(INITIAL_FILE))
            .await
            .expect("README.md should load");
        assert!(!content.is_empty());

        let expected = std::fs::read_to_string(INITIAL_FILE).expect("README.md should exist");
        assert_eq!(*content, expected);
    }

    #[tokio::test]
    async fn missing_file_returns_an_error() {
        let result = load_file(PathBuf::from("/nonexistent/path/for/sputnik-test.txt")).await;
        assert!(result.is_err());
    }

    /// Drives the real `update` path — the same one a keystroke takes —
    /// so the wiring from key to keymap to document is covered, not just
    /// the pieces on either side of it.
    fn press(app: &mut Application, key: Key, modifiers: iced::keyboard::Modifiers) {
        let text = match &key {
            Key::Character(c) => Some(smol_str::SmolStr::new(c.as_str())),
            _ => None,
        };
        let _ = app.update(Message::KeyboardInput {
            key,
            modifiers,
            text,
        });
    }

    fn app() -> Application {
        Application::new(None).0
    }

    #[test]
    fn typing_then_selecting_then_deleting_works_end_to_end() {
        use iced::keyboard::Modifiers;
        use iced::keyboard::key::Named;

        let mut app = app();
        for c in ["h", "i", "!"] {
            press(&mut app, Key::Character(c.into()), Modifiers::empty());
        }
        assert_eq!(app.editor.text(), "hi!");

        // Shift+Left twice selects the last two characters ...
        press(&mut app, Key::Named(Named::ArrowLeft), Modifiers::SHIFT);
        press(&mut app, Key::Named(Named::ArrowLeft), Modifiers::SHIFT);
        assert_eq!(app.editor.document().selected_text(), "i!");

        // ... and Backspace removes exactly them.
        press(&mut app, Key::Named(Named::Backspace), Modifiers::empty());
        assert_eq!(app.editor.text(), "h");
        assert!(app.editor.selection().is_empty());
    }

    /// Opening a file must not throw away what the widget already measured
    /// about the view. The viewport describes the *widget* — wrap width,
    /// font size, how many rows fit — and the widget did not change; only
    /// the text did. Resetting it left the next keystroke resolving
    /// wrap-aware motions against defaults, most visibly making PageDown
    /// travel a single row.
    #[test]
    fn opening_a_file_keeps_the_measured_viewport() {
        use iced::keyboard::Modifiers;
        use iced::keyboard::key::Named;

        let mut app = app();
        // A layout pass measures the real view.
        {
            let element: iced::Element<'_, Message, iced::Theme, iced::Renderer> =
                app.view(app.window_id);
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                iced_test::core::Size::new(500.0, 400.0),
                element,
            );
            let _ = ui.snapshot(&iced::Theme::Light);
        }
        let measured = app.editor.viewport();
        assert!(
            measured.visible_rows > 1,
            "a 400px-tall viewport should fit more than one row"
        );

        let text: String = (1..=50).map(|i| format!("line {i}\n")).collect();
        let _ = app.update(Message::FileOpened(Ok(Arc::new(text))));

        assert_eq!(
            app.editor.viewport().scroll,
            (0, 0.0),
            "a newly opened file starts at the top"
        );
        assert_eq!(
            app.editor.viewport().visible_rows,
            measured.visible_rows,
            "but the measurements survive"
        );

        // The discriminating check: a page must be a page, straight away.
        press(&mut app, Key::Named(Named::PageDown), Modifiers::empty());
        assert!(
            app.editor.text_storage().line_of(app.editor.cursor()) > 1,
            "PageDown right after opening a file moved only one row, which \
             means it was resolved against a default one-row viewport"
        );
    }

    #[test]
    fn select_all_then_typing_replaces_the_whole_document() {
        use iced::keyboard::Modifiers;

        let mut app = app();
        let _ = app.update(Message::FileOpened(Ok(Arc::new("throw away".into()))));
        assert_eq!(app.editor.text(), "throw away");

        press(&mut app, Key::Character("a".into()), Modifiers::CTRL);
        press(&mut app, Key::Character("x".into()), Modifiers::empty());

        assert_eq!(app.editor.text(), "x");
    }

    /// A press places the caret and a drag extends the selection — the
    /// meaning this layer, not the widget, assigns to a mouse.
    #[test]
    fn a_press_then_a_drag_selects() {
        use sputnik_editor::Interaction;

        let mut app = app();
        let _ = app.update(Message::FileOpened(Ok(Arc::new("hello world".into()))));

        let _ = app.update(Message::Editor(Interaction::Press(6)));
        assert!(
            app.editor.selection().is_empty(),
            "a press only moves the caret"
        );

        let _ = app.update(Message::Editor(Interaction::Drag(11)));
        assert_eq!(app.editor.document().selected_text(), "world");

        let _ = app.update(Message::Editor(Interaction::Release));
        assert_eq!(app.editor.document().selected_text(), "world");
    }

    /// Rows are shaped independently of where they land, so one scrolled
    /// partway off an edge still renders its full height and must be
    /// clipped to the editor's own bounds.
    ///
    /// Only the real composition can show this. A bare `editor.view()`
    /// starts at bounds origin y≈0, where overflow has nowhere to go;
    /// inside `container(column![editor, hud]).padding(32.0)` the editor
    /// begins at a positive y and ends above the HUD, so anything
    /// unclipped bleeds into the padding above or the HUD below.
    #[test]
    fn scrolled_content_clips_to_editor_bounds_not_surroundings() {
        let readme = std::fs::read_to_string(INITIAL_FILE).expect("README.md should exist");
        let editor: Editor = Editor::from_str(&readme);

        let hud: iced::Element<'_, (), iced::Theme, iced::Renderer> =
            text("cursor: 0/0").size(14.0).into();
        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = container(
            iced::widget::column![editor.view(), hud]
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .padding(32.0)
        .into();

        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(500.0, 300.0),
            element,
        );
        ui.point_at(iced_test::core::Point::new(50.0, 50.0));
        let _ = ui.snapshot(&iced::Theme::Light);

        for (label, delta) in [("5", -5.0), ("20", -20.0), ("28", -28.0)] {
            ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Pixels { x: 0.0, y: delta },
            })]);
            let snapshot = ui
                .snapshot(&iced::Theme::Light)
                .expect("snapshot should render");
            let matches = snapshot
                .matches_image(
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/snapshots/real_composition_scroll_"
                    )
                    .to_owned()
                        + label
                        + ".png",
                )
                .expect("snapshot should save");
            assert!(matches, "scrolled rendering drifted at {label}px");
        }
    }
}
