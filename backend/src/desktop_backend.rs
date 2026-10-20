//! Select a desktop transport without changing the native scene or X11 helper.
use crate::{
    native::{Native, UserEvent},
    service::Shared,
};
use serde_json::Value;
use std::{sync::Arc, time::Instant};
use tao::event_loop::{EventLoopProxy, EventLoopWindowTarget};

pub enum Backend {
    Native(Native),
    #[cfg(target_os = "linux")]
    Wayland(crate::wayland::Engine),
}
impl Backend {
    pub fn new(shared: Arc<Shared>, proxy: EventLoopProxy<UserEvent>) -> Self {
        #[cfg(target_os = "linux")]
        if crate::platform::is_wayland() {
            return Self::Wayland(crate::wayland::Engine::new(shared, proxy));
        }
        Self::Native(Native::new(shared, proxy))
    }
    pub fn control(
        &mut self,
        target: &EventLoopWindowTarget<UserEvent>,
        method: &str,
        p: Value,
    ) -> Result<Value, String> {
        match self {
            Self::Native(v) => v.control(target, method, p),
            #[cfg(target_os = "linux")]
            Self::Wayland(v) => v.control(method, p),
        }
    }
    pub fn refresh(&mut self, s: Value, library: bool) {
        match self {
            Self::Native(v) => v.refresh(s, library),
            #[cfg(target_os = "linux")]
            Self::Wayland(v) => v.refresh(s, library),
        }
    }
    pub fn helper_event(&mut self, generation: u64, event: Value) {
        match self {
            Self::Native(v) => v.helper_event(generation, event),
            #[cfg(target_os = "linux")]
            Self::Wayland(_) => {}
        }
    }
    pub fn helper_gone(&mut self, generation: u64) {
        match self {
            Self::Native(v) => v.helper_gone(generation),
            #[cfg(target_os = "linux")]
            Self::Wayland(_) => {}
        }
    }
    #[cfg(target_os = "linux")]
    pub fn wayland_event(&mut self, generation: u64, event: Value) {
        if let Self::Wayland(v) = self {
            v.event(generation, event);
        }
    }
    pub fn next_wake(&self) -> Instant {
        match self {
            Self::Native(v) => v.next_wake(),
            #[cfg(target_os = "linux")]
            Self::Wayland(v) => v.next_wake(),
        }
    }
    pub fn tick(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        match self {
            Self::Native(v) => v.tick(target),
            #[cfg(target_os = "linux")]
            Self::Wayland(v) => v.tick(),
        }
    }
}
