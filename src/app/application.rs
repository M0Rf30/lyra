// SPDX-License-Identifier: GPL-3.0

use super::{AppFlags, AppModel, Message};
use cosmic::app::context_drawer;
use cosmic::iced::Subscription;
use cosmic::prelude::*;
use cosmic::widget::nav_bar;

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = AppFlags;
    type Message = Message;
    const APP_ID: &'static str = "io.github.m0rf30.Lyra";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, flags: Self::Flags) -> (Self, Task<cosmic::Action<Self::Message>>) {
        Self::init_model(core, flags)
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        self.header_start_elements()
    }

    fn header_center(&self) -> Vec<Element<'_, Self::Message>> {
        self.header_center_elements()
    }

    fn header_end(&self) -> Vec<Element<'_, Self::Message>> {
        self.header_end_elements()
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav)
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Self::Message>> {
        self.context_drawer_page()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        self.view_page()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        self.build_subscription()
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        self.handle_message(message)
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<cosmic::Action<Self::Message>> {
        self.select_nav(id)
    }
}
