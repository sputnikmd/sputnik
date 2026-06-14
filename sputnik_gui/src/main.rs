#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod application;
mod message;
mod widgets;

use crate::application::Application;

pub const APP_TITLE: &str = "Sputnik";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_AUTHORS: &str = env!("CARGO_PKG_AUTHORS");
pub const APP_ICON: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/icon.ico"));

pub fn main() -> iced::Result {
    iced::daemon(Application::new, Application::update, Application::view)
        .title(Application::title)
        .subscription(Application::subscription)
        .run()
}
