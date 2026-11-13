//! The mouse held down while playing (wowlua-engine's `play`).

use crate::widgets::{self, WidgetId};

/// The mouse held down while playing.
pub struct Press {
    pub right: bool,
    /// The frame it was pressed on; none: the world.
    pub frame: Option<WidgetId>,
    /// Where it was pressed, and where it was last (screen pixels).
    pub from: (f64, f64),
    pub last: (f64, f64),
    /// It moved beyond a click's slack: no click when let go.
    pub dragged: bool,
    /// The frame's drag under way.
    pub dragging: Option<widgets::Dragging>,
}
