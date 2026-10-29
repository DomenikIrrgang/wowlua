//! The flavor's CVars with their defaults and flags (`wowlua_engine::cvars`).

#[test]
fn camelot_has_its_recorded_cvars_with_flags() {
    let cvars = wowlua_engine::cvars("camelot");
    assert!(cvars.len() > 1000);
    let key_down = cvars
        .iter()
        .find(|c| c.name == "ActionButtonUseKeyDown")
        .expect("recorded");
    assert_eq!(key_down.default, "1");
    assert!(key_down.secure && !key_down.read_only);
    let cast_bars = cvars
        .iter()
        .find(|c| c.name == "nameplateShowCastBars")
        .expect("recorded");
    assert!(cast_bars.read_only);
}

#[test]
fn other_flavors_have_the_deprecation_fallbacks() {
    let cvars = wowlua_engine::cvars("vanilla");
    assert_eq!(cvars.len(), 1);
    assert_eq!(cvars[0].name, "loadDeprecationFallbacks");
    assert_eq!(cvars[0].default, "1");
}
