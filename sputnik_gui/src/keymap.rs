//! Keyboard control, in one place.
//!
//! This function *is* the control scheme. The editor widget never sees a
//! key press and the document has no idea one exists, so replacing this
//! module — with a modal, vim-style map, or one read from a config file —
//! changes how the editor is driven without touching either of them.

use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use smol_str::SmolStr;

use sputnik_editor::{Action, Edit, Motion};

/// The action a key press should perform, or `None` if it means nothing
/// to the editor.
pub fn action(key: &Key, modifiers: Modifiers, text: Option<&SmolStr>) -> Option<Action> {
    // Holding shift is the *only* difference between moving the caret and
    // extending the selection, so it is applied once here rather than
    // spelled out for every motion key below.
    let motion = |motion: Motion| {
        Some(if modifiers.shift() {
            Action::Select(motion)
        } else {
            Action::Move(motion)
        })
    };

    match key.as_ref() {
        Key::Character("a") if modifiers.command() => Some(Action::SelectAll),

        Key::Named(Named::ArrowLeft) if modifiers.command() => motion(Motion::WordLeft),
        Key::Named(Named::ArrowRight) if modifiers.command() => motion(Motion::WordRight),
        Key::Named(Named::ArrowLeft) => motion(Motion::Left),
        Key::Named(Named::ArrowRight) => motion(Motion::Right),
        Key::Named(Named::ArrowUp) => motion(Motion::Up),
        Key::Named(Named::ArrowDown) => motion(Motion::Down),
        Key::Named(Named::PageUp) => motion(Motion::PageUp),
        Key::Named(Named::PageDown) => motion(Motion::PageDown),
        Key::Named(Named::Home) if modifiers.command() => motion(Motion::DocumentStart),
        Key::Named(Named::End) if modifiers.command() => motion(Motion::DocumentEnd),
        Key::Named(Named::Home) => motion(Motion::RowStart),
        Key::Named(Named::End) => motion(Motion::RowEnd),

        Key::Named(Named::Backspace) => Some(Action::Edit(Edit::Backspace)),
        Key::Named(Named::Delete) => Some(Action::Edit(Edit::Delete)),
        Key::Named(Named::Enter) => Some(Action::Edit(Edit::Enter)),
        Key::Named(Named::Tab) => Some(Action::Edit(Edit::Tab)),
        Key::Named(Named::Space) => Some(Action::Edit(Edit::Insert(' '))),

        // Anything else that produced text is typing. Guarded on the
        // command modifier so that a shortcut which happens to carry text
        // — Ctrl+S — does not also insert an "s". Delivered whole rather
        // than character by character, so an IME commit or a dead-key
        // sequence replaces the selection once instead of once per char.
        _ if !modifiers.command() => {
            let text = text?;
            (!text.is_empty() && !text.chars().any(char::is_control))
                .then(|| Action::Edit(Edit::Paste(text.to_string())))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: Key, modifiers: Modifiers) -> Option<Action> {
        action(&key, modifiers, None)
    }

    #[test]
    fn shift_turns_every_motion_into_a_selection() {
        let shift = Modifiers::SHIFT;
        for (key, motion) in [
            (Named::ArrowLeft, Motion::Left),
            (Named::ArrowRight, Motion::Right),
            (Named::ArrowUp, Motion::Up),
            (Named::ArrowDown, Motion::Down),
            (Named::PageUp, Motion::PageUp),
            (Named::PageDown, Motion::PageDown),
            (Named::Home, Motion::RowStart),
            (Named::End, Motion::RowEnd),
        ] {
            assert_eq!(
                press(Key::Named(key), Modifiers::empty()),
                Some(Action::Move(motion))
            );
            assert_eq!(press(Key::Named(key), shift), Some(Action::Select(motion)));
        }
    }

    #[test]
    fn a_shortcut_carrying_text_does_not_type_it() {
        assert_eq!(
            action(
                &Key::Character("s".into()),
                Modifiers::CTRL,
                Some(&SmolStr::new("s")),
            ),
            None,
            "Ctrl+S must not insert an \"s\""
        );
        assert_eq!(
            action(
                &Key::Character("s".into()),
                Modifiers::empty(),
                Some(&SmolStr::new("s")),
            ),
            Some(Action::Edit(Edit::Paste("s".into()))),
        );
    }

    #[test]
    fn control_characters_are_not_typed() {
        assert_eq!(
            action(
                &Key::Named(Named::Escape),
                Modifiers::empty(),
                Some(&SmolStr::new("\u{1b}")),
            ),
            None,
        );
    }
}
