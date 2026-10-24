//! Animation groups and animations: objects a region makes
//! (`CreateAnimationGroup`, `<Animations>` in XML), played on the frame
//! clock. Nothing is drawn; what code can see is: `Play` runs `OnPlay`, a
//! group finishes once its animations' time passed (the orders one after
//! another, the animations of an order together: start delay, duration,
//! end delay), running `OnFinished`, or `OnLoop` when it repeats; `Stop`
//! runs `OnStop`, `Finish` finishes now.
//!
//! Unverified: the timing within a frame (groups advance after the
//! frame's `OnUpdate`s), that `Finish` runs `OnFinished` at once, and the
//! scripts' arguments.

use std::collections::HashMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::widgets::{self, MethodFn, WidgetId};
use crate::{engine, engine_mut};

/// A playing (or paused) group.
#[derive(Clone, Copy, Default)]
pub struct Playing {
    pub elapsed: f64,
    pub paused: bool,
}

/// The groups playing now, by widget.
#[derive(Default)]
pub struct Animations {
    pub playing: HashMap<usize, Playing>,
}

fn stored_number(vm: &mut Vm, id: WidgetId, stem: &str) -> f64 {
    let props = engine(vm).world.widgets[id.0].props;
    match widgets::stored_args(vm, props, stem).and_then(|v| v.first().copied()) {
        Some(Value::Num(n)) => n,
        _ => 0.0,
    }
}

/// The animations of a group, in creation order.
fn animations(vm: &Vm, group: WidgetId) -> Vec<WidgetId> {
    let e = engine(vm);
    e.world.widgets[group.0]
        .children
        .iter()
        .copied()
        .filter(|c| widgets::is_a(e, &e.world.widgets[c.0].class, "Animation"))
        .collect()
}

/// A group's length: its orders one after another, each as long as its
/// longest animation.
fn group_duration(vm: &mut Vm, group: WidgetId) -> f64 {
    let mut by_order: std::collections::BTreeMap<i64, f64> = Default::default();
    for a in animations(vm, group) {
        let order = stored_number(vm, a, "Order") as i64;
        let length = stored_number(vm, a, "StartDelay")
            + stored_number(vm, a, "Duration")
            + stored_number(vm, a, "EndDelay");
        let slot = by_order.entry(order).or_insert(0.0);
        *slot = slot.max(length);
    }
    // Not `sum`: no animations last 0, not -0.
    by_order.values().fold(0.0, |a, b| a + b)
}

fn play(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let resumed = {
        let p = &mut engine_mut(vm).animations.playing;
        match p.get_mut(&id.0) {
            Some(state) if state.paused => {
                state.paused = false;
                true
            }
            _ => {
                p.insert(id.0, Playing::default());
                false
            }
        }
    };
    if !resumed {
        widgets::run_script(vm, id, "OnPlay", &[]);
    }
    Ok(vec![])
}

fn stop(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    stop_group(vm, id);
    Ok(vec![])
}

/// A group stopped: `OnStop(false)` if it was playing.
pub fn stop_group(vm: &mut Vm, id: WidgetId) {
    if engine_mut(vm).animations.playing.remove(&id.0).is_some() {
        widgets::run_script(vm, id, "OnStop", &[Value::Bool(false)]);
    }
}

fn pause(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if let Some(state) = engine_mut(vm).animations.playing.get_mut(&id.0) {
        state.paused = true;
        widgets::run_script(vm, id, "OnPause", &[]);
    }
    Ok(vec![])
}

fn finish(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if engine_mut(vm).animations.playing.remove(&id.0).is_some() {
        widgets::run_script(vm, id, "OnFinished", &[Value::Bool(true)]);
    }
    Ok(vec![])
}

fn restart(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    engine_mut(vm).animations.playing.remove(&id.0);
    play(vm, id, c)
}

fn is_playing(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let playing = engine(vm)
        .animations
        .playing
        .get(&id.0)
        .is_some_and(|p| !p.paused);
    Ok(vec![Value::Bool(playing)])
}

fn is_paused(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let paused = engine(vm)
        .animations
        .playing
        .get(&id.0)
        .is_some_and(|p| p.paused);
    Ok(vec![Value::Bool(paused)])
}

fn is_done(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let done = !engine(vm).animations.playing.contains_key(&id.0);
    Ok(vec![Value::Bool(done)])
}

fn group_get_duration(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(group_duration(vm, id))])
}

fn get_duration(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(stored_number(vm, id, "Duration"))])
}

fn get_animations(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let list = animations(vm, id);
    Ok(list
        .into_iter()
        .map(|a| Value::Table(engine(vm).world.widgets[a.0].table))
        .collect())
}

/// `CreateAnimationGroup([name, inherits])` on a region.
fn create_animation_group(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let name = vm
        .opt_str(&c, 2)?
        .map(|s| String::from_utf8_lossy(vm.str_bytes(s)).into_owned());
    let group = widgets::create(vm, "AnimationGroup", name, Some(id));
    if let Some(t) = vm.opt_str(&c, 3)? {
        let t = String::from_utf8_lossy(vm.str_bytes(t)).into_owned();
        widgets::apply_template(vm, group, &t)?;
    }
    Ok(vec![group])
}

/// `CreateAnimation([type, name, inherits])` on a group: an `Animation` of
/// that type (any case).
fn create_animation(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let kind = vm
        .opt_str(&c, 2)?
        .map(|s| String::from_utf8_lossy(vm.str_bytes(s)).into_owned())
        .unwrap_or_else(|| "Animation".into());
    let class = widgets::canonical_class(engine(vm), &kind)
        .filter(|k| widgets::is_a(engine(vm), k, "Animation"))
        .ok_or_else(|| vm.error(&format!("CreateAnimation: Unknown animation type '{kind}'")))?;
    let name = vm
        .opt_str(&c, 3)?
        .map(|s| String::from_utf8_lossy(vm.str_bytes(s)).into_owned());
    let anim = widgets::create(vm, &class, name, Some(id));
    if let Some(t) = vm.opt_str(&c, 4)? {
        let t = String::from_utf8_lossy(vm.str_bytes(t)).into_owned();
        widgets::apply_template(vm, anim, &t)?;
    }
    Ok(vec![anim])
}

fn get_animation_groups(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let e = engine(vm);
    Ok(e.world.widgets[id.0]
        .children
        .iter()
        .filter(|c| e.world.widgets[c.0].class == "AnimationGroup")
        .map(|c| Value::Table(e.world.widgets[c.0].table))
        .collect())
}

/// Moves the playing groups by `seconds`: finished ones run `OnFinished`
/// (or, repeating, `OnLoop`).
pub fn tick(vm: &mut Vm, seconds: f64) {
    let ids: Vec<usize> = engine(vm).animations.playing.keys().copied().collect();
    for i in ids {
        let id = WidgetId(i);
        let Some(state) = engine(vm).animations.playing.get(&i).copied() else {
            continue;
        };
        if state.paused {
            continue;
        }
        let elapsed = state.elapsed + seconds;
        let duration = group_duration(vm, id);
        if elapsed < duration {
            if let Some(s) = engine_mut(vm).animations.playing.get_mut(&i) {
                s.elapsed = elapsed;
            }
            continue;
        }
        let props = engine(vm).world.widgets[i].props;
        let looping = matches!(
            widgets::stored_args(vm, props, "Looping").and_then(|v| v.first().copied()),
            Some(Value::Str(s)) if !vm.str_bytes(s).eq_ignore_ascii_case(b"NONE")
        );
        if looping {
            if let Some(s) = engine_mut(vm).animations.playing.get_mut(&i) {
                s.elapsed = 0.0;
            }
            let state = vm.str("FORWARD");
            widgets::run_script(vm, id, "OnLoop", &[state]);
        } else {
            engine_mut(vm).animations.playing.remove(&i);
            widgets::run_script(vm, id, "OnFinished", &[Value::Bool(false)]);
        }
    }
}

/// The methods with behaviour here (by class and name).
pub fn methods() -> Vec<(&'static str, MethodFn)> {
    vec![
        ("AnimationGroup:Play", play),
        ("AnimationGroup:Stop", stop),
        ("AnimationGroup:Pause", pause),
        ("AnimationGroup:Finish", finish),
        ("AnimationGroup:Restart", restart),
        ("AnimationGroup:IsPlaying", is_playing),
        ("AnimationGroup:IsPaused", is_paused),
        ("AnimationGroup:IsDone", is_done),
        ("AnimationGroup:GetDuration", group_get_duration),
        ("AnimationGroup:GetAnimations", get_animations),
        ("AnimationGroup:CreateAnimation", create_animation),
        ("Animation:GetDuration", get_duration),
        ("CreateAnimationGroup", create_animation_group),
        ("GetAnimationGroups", get_animation_groups),
    ]
}
