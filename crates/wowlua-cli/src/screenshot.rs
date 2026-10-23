//! Screenshots of the emulated client's UI (spec debugger §18.11): the
//! renderer drawing a scene off-screen.

/// The screenshot function sessions use (none until the renderer is
/// built in).
pub fn screenshotter() -> Option<wowlua_debug::controller::Screenshotter> {
    None
}
