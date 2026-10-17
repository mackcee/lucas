use crate::{Overlay, OverlayError, Pulse, Rect, Result, render::Surface};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Show(Pulse),
    Hide,
    Target(Rect),
    ReassertTopmost,
    Shutdown,
}

/// Headless command recorder. No wall clock: tests own animation/scheduling time.
#[derive(Debug)]
pub struct Recording {
    target: Rect,
    events: Vec<Event>,
    closed: bool,
}

impl Recording {
    pub fn new(target: Rect) -> Result<Self> {
        Surface::for_target(target)?;
        Ok(Self {
            target,
            events: Vec::new(),
            closed: false,
        })
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn target_rect(&self) -> Rect {
        self.target
    }

    fn record(&mut self, event: Event) -> Result<()> {
        if self.closed {
            return Err(OverlayError::Closed);
        }
        self.events.push(event);
        Ok(())
    }
}

impl Overlay for Recording {
    fn show_pulse(&mut self, pulse: Pulse) -> Result<()> {
        self.record(Event::Show(pulse))
    }

    fn hide(&mut self) -> Result<()> {
        self.record(Event::Hide)
    }

    fn set_target_rect(&mut self, rect: Rect) -> Result<()> {
        if self.closed {
            return Err(OverlayError::Closed);
        }
        Surface::for_target(rect)?;
        self.record(Event::Target(rect))?;
        self.target = rect;
        Ok(())
    }

    fn reassert_topmost(&mut self) -> Result<()> {
        self.record(Event::ReassertTopmost)
    }

    fn shutdown(&mut self) -> Result<()> {
        if !self.closed {
            self.record(Event::Shutdown)?;
            self.closed = true;
        }
        Ok(())
    }
}
