//! The names over units in the 3D world (decision T534), as the game
//! draws them behind the UI: a unit at a place the camera sees, within
//! `nameplateMaxDistance`, whose kind the game's name settings show
//! (`UnitName…` CVars) and whose nameplate does not show, has its name
//! over it in its reaction's color, an NPC's title (`<Cloth Armor
//! Merchant>`) under it.

use wowlua_vm::Vm;

use crate::engine;
use crate::scene::{Item, Rect, Run, Text, TextLine};
use crate::units::{Reaction, UnitId};

/// A name's height over its unit's feet (yards; unverified: the model's).
const NAME_HEIGHT: f64 = 2.4;
/// The name's font size on a 1080-pixel-high screen, its title's (pixels;
/// unverified).
const NAME_PX: f64 = 15.0;
const TITLE_PX: f64 = 13.0;

/// Whether a setting is on, as the game's: a CVar not "0"; one the flavor
/// lacks, its camelot default.
fn on(vm: &mut Vm, name: &str, default: bool) -> bool {
    crate::framescript::cvar_get(vm, name).map_or(default, |v| v != "0" && !v.is_empty())
}

/// Whether an NPC offers anything (a vendor, a trainer, gossip, a
/// service): the game's "interactive" NPC.
fn interactive(vm: &Vm, id: UnitId) -> bool {
    let w = &engine(vm).world;
    crate::merchant::has(vm, id)
        || w.gossip.defined.contains_key(&id.0)
        || w.trainers.defined.contains_key(&id.0)
        || w.taxis.defined.contains_key(&id.0)
        || crate::auction::has(vm, id)
        || crate::bank::has(vm, id)
        || crate::mail::has(vm, id)
}

/// Whether the game's name settings show a unit's name (decision T534):
/// another player's by friend or foe; an NPC's all with `UnitNameNPC`,
/// else a critter's by `UnitNameNonCombatCreatureName`, an interactive
/// one's by `UnitNameInteractiveNPC`, a hostile or neutral one's by
/// `UnitNameHostleNPC`, another friendly one's by
/// `UnitNameFriendlySpecialNPCName` (unverified: which NPCs are
/// "special").
fn shown(vm: &mut Vm, id: UnitId) -> bool {
    let (player, reaction, critter) = {
        let u = &engine(vm).units.list[id.0];
        let critter = u
            .creature_type
            .as_ref()
            .is_some_and(|(t, _)| t.eq_ignore_ascii_case("Critter"));
        (u.player, u.reaction, critter)
    };
    if player {
        return match reaction {
            Reaction::Hostile => on(vm, "UnitNameEnemyPlayerName", true),
            _ => on(vm, "UnitNameFriendlyPlayerName", true),
        };
    }
    if on(vm, "UnitNameNPC", false) {
        return true;
    }
    if critter {
        return on(vm, "UnitNameNonCombatCreatureName", false);
    }
    if interactive(vm, id) {
        return on(vm, "UnitNameInteractiveNPC", true);
    }
    match reaction {
        Reaction::Friendly => on(vm, "UnitNameFriendlySpecialNPCName", true),
        _ => on(vm, "UnitNameHostleNPC", true),
    }
}

/// A unit's name color: an NPC's its reaction's (red, yellow, green),
/// another player's blue unless hostile (unverified: the shades).
fn color(player: bool, reaction: Reaction) -> [f32; 4] {
    match (player, reaction) {
        (_, Reaction::Hostile) => [1.0, 0.1, 0.1, 1.0],
        (false, Reaction::Neutral) => [1.0, 1.0, 0.0, 1.0],
        (false, Reaction::Friendly) => [0.1, 1.0, 0.1, 1.0],
        (true, _) => [0.25, 0.6, 1.0, 1.0],
    }
}

/// One line of text centered at `x`, its top at `y` (screen pixels).
fn line(vm: &Vm, text: &str, px: f64, x: f64, y: f64, color: [f32; 4]) -> (TextLine, f32, f32) {
    let font = crate::text::Font {
        file: "frizqt__.ttf".into(),
        size: px,
        outline: true,
        shadow_x: 0.0,
        client: engine(vm).client_data.clone(),
    };
    let g = crate::text::glyph_line_of(&font, px.round() as i64, text);
    let width = g.width_px as f32;
    let line = TextLine {
        x: (x as f32 - width / 2.0).round(),
        y: y.round() as f32,
        width,
        runs: vec![Run {
            text: text.to_owned(),
            color,
            xs: g.xs.iter().map(|&v| v as f32).collect(),
        }],
    };
    (line, width, g.line_height_px as f32)
}

/// The names to draw over the units, farthest first (decision T534); none
/// while the 3D world is off.
pub fn items(vm: &mut Vm, width: f64, height: f64) -> Vec<Item> {
    if !engine(vm).world_on {
        return Vec::new();
    }
    let world_frame = engine(vm)
        .world
        .widgets
        .iter()
        .position(|w| w.name.as_deref() == Some("WorldFrame"))
        .unwrap_or(0);
    let max = crate::framescript::cvar_get(vm, "nameplateMaxDistance")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(45.0);
    let scale = height / 1080.0;
    let mut found: Vec<(f64, Item)> = Vec::new();
    for i in 1..engine(vm).units.list.len() {
        let id = UnitId(i);
        let (alive, distance, name, title, player, reaction) = {
            let u = &engine(vm).units.list[i];
            (
                u.alive_in_world && u.world.is_some(),
                u.distance,
                u.name.clone(),
                u.title.clone(),
                u.player,
                u.reaction,
            )
        };
        if !alive
            || name.is_empty()
            || distance.is_some_and(|d| d > max)
            || engine(vm).units.nameplates.contains(&Some(id))
            || !shown(vm, id)
        {
            continue;
        }
        let Some(at) = crate::character::place_of(vm, id) else {
            continue;
        };
        let Some((sx, sy)) =
            crate::camera::project(vm, (at.0, at.1, at.2 + NAME_HEIGHT), width, height)
        else {
            continue;
        };
        let (cx, bottom) = (width / 2.0 + sx, height / 2.0 - sy);
        let c = color(player, reaction);
        let (name_px, title_px) = ((NAME_PX * scale).max(9.0), (TITLE_PX * scale).max(8.0));
        let mut lines = Vec::new();
        let mut y = bottom;
        let mut w: f32 = 0.0;
        // (the title under the name: laid out from the bottom up)
        if let Some(title) = title.filter(|t| !t.is_empty()) {
            let text = format!("<{title}>");
            let (l, lw, _) = line(vm, &text, title_px, cx, y - title_px * 1.25, c);
            y = f64::from(l.y);
            w = w.max(lw);
            lines.push(l);
        }
        let (l, lw, lh) = line(vm, &name, name_px, cx, y - name_px * 1.25, c);
        w = w.max(lw);
        let top = l.y;
        lines.insert(0, l);
        let depth = distance.unwrap_or(0.0);
        let rect = Rect {
            x: (cx as f32 - w / 2.0).round(),
            y: top,
            w,
            h: (bottom as f32 - top).max(lh),
        };
        found.push((
            depth,
            Item::Text(Text {
                widget: world_frame,
                images: Vec::new(),
                rect,
                font: "Fonts\\FRIZQT__.TTF".into(),
                px: name_px as f32,
                line_height: lh,
                outline: 1,
                monochrome: false,
                shadow: None,
                lines,
                clip: None,
            }),
        ));
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    found.into_iter().map(|(_, item)| item).collect()
}
