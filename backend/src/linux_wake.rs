//! Supply the GLib deadline source missing from Tao 0.35's GTK WaitUntil path.
use crate::native::UserEvent;
use gtk::glib::{self, SourceId};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};
use tao::event_loop::{ControlFlow, EventLoopProxy};

#[derive(Default)]
pub struct DeadlineWake {
    pending: Option<(Instant, SourceId, Rc<Cell<bool>>)>,
}

impl DeadlineWake {
    pub fn update(&mut self, flow: ControlFlow, proxy: &EventLoopProxy<UserEvent>) {
        let ControlFlow::WaitUntil(deadline) = flow else {
            self.clear();
            return;
        };
        // An earlier wake is sufficient; avoid replacing the source for every
        // GTK callback. The explicit event also rearms if the deadline moved.
        if self
            .pending
            .as_ref()
            .is_some_and(|(at, _, fired)| !fired.get() && *at <= deadline)
        {
            return;
        }
        self.clear();
        let delay = deadline.saturating_duration_since(Instant::now());
        if delay.is_zero() {
            return; // Tao will immediately process the already-due deadline.
        }
        // GLib takes whole milliseconds. Round up so the source cannot fire
        // before Tao's deadline and put the otherwise idle loop back to sleep.
        let delay = Duration::from_millis(delay.as_nanos().div_ceil(1_000_000) as u64);
        let fired = Rc::new(Cell::new(false));
        let complete = fired.clone();
        let proxy = proxy.clone();
        let source = glib::timeout_add_local_once(delay, move || {
            complete.set(true);
            let _ = proxy.send_event(UserEvent::DeadlineWake);
        });
        self.pending = Some((deadline, source, fired));
    }

    fn clear(&mut self) {
        if let Some((_, source, fired)) = self.pending.take() {
            // A one-shot source has already removed itself after firing.
            if !fired.get() {
                source.remove();
            }
        }
    }
}

impl Drop for DeadlineWake {
    fn drop(&mut self) {
        self.clear();
    }
}
