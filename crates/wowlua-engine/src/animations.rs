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
        apply_final_alpha(vm, id);
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
            apply_final_alpha(vm, id);
            widgets::run_script(vm, id, "OnFinished", &[Value::Bool(false)]);
        }
    }
}

fn stored_ref_number(vm: &Vm, id: WidgetId, stem: &str) -> Option<f64> {
    let props = engine(vm).world.widgets[id.0].props;
    match widgets::stored_ref(vm, props, stem).and_then(|v| v.first().copied()) {
        Some(Value::Num(n)) => Some(n),
        _ => None,
    }
}

/// The region an animation acts on: its target set (`SetTarget`), its
/// child key's or target key's field of the group's owner, a global by
/// name, else the group's owner.
pub fn target_of(vm: &Vm, anim: WidgetId) -> Option<WidgetId> {
    let e = engine(vm);
    let group = e.world.widgets[anim.0].parent?;
    let owner = e.world.widgets[group.0].parent?;
    let props = e.world.widgets[anim.0].props;
    let first = |stem: &str| widgets::stored_ref(vm, props, stem).and_then(|v| v.first().copied());
    if let Some(t) = first("Target").and_then(|v| widgets::widget_of(vm, v)) {
        return Some(t);
    }
    for stem in ["ChildKey", "TargetKey"] {
        if let Some(Value::Str(k)) = first(stem) {
            let table = e.world.widgets[owner.0].table;
            let key = String::from_utf8_lossy(vm.heap.str_bytes(k)).into_owned();
            // A path (`Glow.Flipbook`) from the owner.
            let mut cur = Value::Table(table);
            for part in key.split('.') {
                let Value::Table(t) = cur else { return None };
                let k = vm.heap.interned(part.as_bytes())?;
                cur = vm.heap.table(t).get(Value::Str(k));
            }
            return widgets::widget_of(vm, cur);
        }
    }
    if let Some(Value::Str(n)) = first("TargetName") {
        let k = vm.heap.interned(vm.heap.str_bytes(n))?;
        return widgets::widget_of(vm, vm.heap.table(vm.globals).get(Value::Str(k)));
    }
    Some(owner)
}

/// What playing animations do to a region now (spec debugger §18.9): an
/// alpha in place of its own, a flip book's frame (rows, columns, frame).
#[derive(Clone, Copy, Debug, Default)]
pub struct Effect {
    pub alpha: Option<f64>,
    pub flip: Option<(u32, u32, u32)>,
}

/// Each region's effect from the groups playing: each animation from its
/// start (its order's start, its delay) to its end, linear; an alpha held
/// at its end until the group ends, a later order's over an earlier's.
/// Unverified: the smoothings' curves (linear here).
pub fn effects(vm: &Vm) -> HashMap<usize, Effect> {
    let mut out: HashMap<usize, Effect> = HashMap::new();
    let e = engine(vm);
    let mut groups: Vec<(usize, Playing)> =
        e.animations.playing.iter().map(|(k, v)| (*k, *v)).collect();
    groups.sort_by_key(|g| g.0);
    for (g, state) in groups {
        let mut anims: Vec<(i64, WidgetId)> = animations(vm, WidgetId(g))
            .into_iter()
            .map(|a| (stored_ref_number(vm, a, "Order").unwrap_or(0.0) as i64, a))
            .collect();
        anims.sort_by_key(|a| a.0);
        // Each order's start: the earlier orders' lengths.
        let mut order_start: std::collections::BTreeMap<i64, f64> = Default::default();
        let mut lengths: std::collections::BTreeMap<i64, f64> = Default::default();
        for (o, a) in &anims {
            let len = stored_ref_number(vm, *a, "StartDelay").unwrap_or(0.0)
                + stored_ref_number(vm, *a, "Duration").unwrap_or(0.0)
                + stored_ref_number(vm, *a, "EndDelay").unwrap_or(0.0);
            let slot = lengths.entry(*o).or_insert(0.0);
            *slot = slot.max(len);
        }
        let mut t0 = 0.0;
        for (o, len) in &lengths {
            order_start.insert(*o, t0);
            t0 += len;
        }
        for (o, a) in anims {
            let start = order_start.get(&o).copied().unwrap_or(0.0)
                + stored_ref_number(vm, a, "StartDelay").unwrap_or(0.0);
            if state.elapsed < start {
                continue;
            }
            let duration = stored_ref_number(vm, a, "Duration").unwrap_or(0.0);
            let p = if duration > 0.0 {
                ((state.elapsed - start) / duration).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let Some(target) = target_of(vm, a) else {
                continue;
            };
            let class = &e.world.widgets[a.0].class;
            let effect = out.entry(target.0).or_default();
            if widgets::is_a(e, class, "Alpha") {
                let from = stored_ref_number(vm, a, "FromAlpha").unwrap_or(0.0);
                let to = stored_ref_number(vm, a, "ToAlpha").unwrap_or(0.0);
                effect.alpha = Some(from + (to - from) * p);
            } else if widgets::is_a(e, class, "FlipBook") {
                let rows = stored_ref_number(vm, a, "FlipBookRows")
                    .unwrap_or(1.0)
                    .max(1.0) as u32;
                let cols = stored_ref_number(vm, a, "FlipBookColumns")
                    .unwrap_or(1.0)
                    .max(1.0) as u32;
                let frames = stored_ref_number(vm, a, "FlipBookFrames")
                    .filter(|f| *f > 0.0)
                    .unwrap_or(f64::from(rows * cols)) as u32;
                let frame = ((p * f64::from(frames)) as u32).min(frames.saturating_sub(1));
                effect.flip = Some((rows, cols, frame));
            }
        }
    }
    out
}

/// A finished group set to its final alpha (`setToFinalAlpha`): its
/// alpha animations' end values stay on their targets.
fn apply_final_alpha(vm: &mut Vm, group: WidgetId) {
    let props = engine(vm).world.widgets[group.0].props;
    let on = widgets::stored_ref(vm, props, "ToFinalAlpha")
        .and_then(|v| v.first().copied())
        .is_some_and(|v| v.truthy());
    if !on {
        return;
    }
    let mut anims: Vec<(i64, WidgetId)> = animations(vm, group)
        .into_iter()
        .map(|a| (stored_ref_number(vm, a, "Order").unwrap_or(0.0) as i64, a))
        .collect();
    anims.sort_by_key(|a| a.0);
    for (_, a) in anims {
        let class = engine(vm).world.widgets[a.0].class.clone();
        if !widgets::is_a(engine(vm), &class, "Alpha") {
            continue;
        }
        let to = stored_ref_number(vm, a, "ToAlpha").unwrap_or(0.0);
        if let Some(t) = target_of(vm, a) {
            engine_mut(vm).world.widgets[t.0].alpha = to;
            crate::widgets::mark(crate::engine_mut(vm), t);
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
