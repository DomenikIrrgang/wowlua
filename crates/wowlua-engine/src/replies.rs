//! The server's answers to what the player types (decision T362): a
//! roll, the time played, away and busy, the ignore list, a who, an
//! emote. Each answers as the server's message would, in Blizzard's words
//! (`RANDOM_ROLL_RESULT`, `ERR_IGNORE_ADDED_S`, …, the global strings when
//! loaded), a round trip later. Unverified on WoW Forever (probe owed):
//! the messages' timing and events, the time played, a who's results.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::{engine, engine_mut, units, widgets};

/// A global string, else its English text (without Blizzard's UI).
pub(crate) fn global(vm: &mut Vm, key: &str, fallback: &str) -> String {
    match vm.get_global(key) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => fallback.to_string(),
    }
}

/// A system line (`CHAT_MSG_SYSTEM`) a round trip later.
pub(crate) fn system_later(vm: &mut Vm, text: String) {
    let t = vm.str(&text);
    let f = vm.native_with("system reply", system_reply, vec![t]);
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, f);
}

fn system_reply(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(Value::Str(s)) = c.upvals.first().copied() {
        let text = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
        crate::chat::game_message(vm, "CHAT_MSG_SYSTEM", &text, None);
    }
    Ok(vec![])
}

fn text_arg(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).trim().to_string()),
        Value::Num(n) => Some(format!("{n}")),
        _ => None,
    }
    .filter(|s| !s.is_empty())
}

/// The player's name as the server writes it: first and last name on
/// WoW Forever, else the name alone.
fn player_name(vm: &Vm) -> String {
    let p = &engine(vm).units.list[units::PLAYER.0];
    match &p.last_name {
        Some(last) => format!("{} {last}", p.name),
        None => p.name.clone(),
    }
}

// ----------------------------------------------------------------- roll

/// `RandomRoll(min, max)` (`/roll`): `RANDOM_ROLL_RESULT` from the server,
/// 1 to 100 by default; its own generator, not Lua's `math.random`
/// (unverified: the bounds' limits, a reversed range).
fn random_roll(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = |vm: &Vm, v: Value| {
        text_arg(vm, v)
            .and_then(|s| s.parse::<f64>().ok())
            .map(f64::trunc)
    };
    let (mut low, mut high) = match (n(vm, c.arg(1)), n(vm, c.arg(2))) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (1.0, a),
        _ => (1.0, 100.0),
    };
    if low > high {
        std::mem::swap(&mut low, &mut high);
    }
    let chat = &mut engine_mut(vm).world.chat;
    if chat.rolls == 0 {
        // A seed of its own, so the first roll is not always 1.
        chat.rolls = 0x2545_f491;
    }
    chat.rolls = chat.rolls.wrapping_mul(1_103_515_245).wrapping_add(12345);
    let r = f64::from((chat.rolls >> 1) & 0x7fff_ffff) / f64::from(0x7fff_ffffu32);
    let roll = (r * (high - low + 1.0)).floor() + low;
    let format = global(vm, "RANDOM_ROLL_RESULT", "%s rolls %d (%d-%d)");
    let name = player_name(vm);
    let text = format
        .replacen("%s", &name, 1)
        .replacen("%d", &format!("{roll}"), 1)
        .replacen("%d", &format!("{low}"), 1)
        .replacen("%d", &format!("{high}"), 1);
    system_later(vm, text);
    Ok(vec![])
}

// ---------------------------------------------------------- time played

/// `RequestTimePlayed()` (`/played`): `TIME_PLAYED_MSG(total, thisLevel)`
/// a round trip later; the time since the session began stands in for
/// both (the character's own was never recorded).
fn request_time_played(vm: &mut Vm, _c: NativeCall) -> Ret {
    let f = vm.native("time played", time_played);
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, f);
    Ok(vec![])
}

fn time_played(vm: &mut Vm, _c: NativeCall) -> Ret {
    let seconds = (vm.clock.elapsed_ms / 1000.0).floor();
    widgets::fire_event(
        vm,
        "TIME_PLAYED_MSG",
        &[Value::Num(seconds), Value::Num(seconds)],
    );
    Ok(vec![])
}

// --------------------------------------------------------- away and busy

/// Away (`AFK`) or busy (`DND`): a chat message of that type sets it with
/// its text (Blizzard's default without one), and clears it when set;
/// `PLAYER_FLAGS_CHANGED` follows (unverified: whether one clears the
/// other, and the moment).
pub(crate) fn toggle_status(vm: &mut Vm, kind: &str, message: &str) {
    let afk = kind == "AFK";
    let chat = &mut engine_mut(vm).world.chat;
    let slot = if afk { &mut chat.afk } else { &mut chat.dnd };
    let line = if slot.take().is_some() {
        if afk {
            global(vm, "CLEARED_AFK", "You are no longer AFK.")
        } else {
            global(vm, "CLEARED_DND", "You are no longer marked DND.")
        }
    } else {
        let text = if !message.is_empty() {
            message.to_string()
        } else if afk {
            global(vm, "DEFAULT_AFK_MESSAGE", "Away from Keyboard")
        } else {
            global(vm, "DEFAULT_DND_MESSAGE", "Do not Disturb")
        };
        let chat = &mut engine_mut(vm).world.chat;
        if afk {
            chat.afk = Some(text.clone());
            chat.dnd = None;
        } else {
            chat.dnd = Some(text.clone());
            chat.afk = None;
        }
        if afk {
            global(vm, "MARKED_AFK_MESSAGE", "You are now AFK: %s").replacen("%s", &text, 1)
        } else {
            global(vm, "MARKED_DND", "You are now DND: %s.").replacen("%s", &text, 1)
        }
    };
    system_later(vm, line);
    let unit = vm.str("player");
    let f = vm.native_with("flags changed", flags_changed, vec![unit]);
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, f);
}

fn flags_changed(vm: &mut Vm, c: NativeCall) -> Ret {
    widgets::fire_event(vm, "PLAYER_FLAGS_CHANGED", &c.upvals);
    Ok(vec![])
}

/// Whether a unit token names the player.
fn is_player(vm: &mut Vm, v: Value) -> bool {
    match vm.hidden(v) {
        Value::Str(s) => {
            let t = String::from_utf8_lossy(vm.str_bytes(s)).to_ascii_lowercase();
            t == "player"
        }
        _ => false,
    }
}

/// `UnitIsAFK(unit)`: the player while away; others never.
fn unit_is_afk(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = is_player(vm, c.arg(1));
    Ok(vec![Value::Bool(me && engine(vm).world.chat.afk.is_some())])
}

/// `UnitIsDND(unit)`: the player while busy; others never.
fn unit_is_dnd(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = is_player(vm, c.arg(1));
    Ok(vec![Value::Bool(me && engine(vm).world.chat.dnd.is_some())])
}

// ------------------------------------------------------------- ignores

/// The ignore list's entry for a name, any case.
fn ignored_at(vm: &Vm, name: &str) -> Option<usize> {
    engine(vm)
        .world
        .chat
        .ignores
        .iter()
        .position(|n| n.eq_ignore_ascii_case(name))
}

/// Whether a name is the player's own.
fn is_own_name(vm: &Vm, name: &str) -> bool {
    let p = &engine(vm).units.list[units::PLAYER.0];
    name.eq_ignore_ascii_case(&player_name(vm)) || name.eq_ignore_ascii_case(&p.name)
}

/// Another player the world has by that name: the name as the server
/// writes it.
fn known_player(vm: &Vm, name: &str) -> Option<String> {
    let first = name.split(['-', ' ']).next().unwrap_or(name);
    engine(vm)
        .units
        .list
        .iter()
        .skip(1)
        .find(|u| {
            u.player
                && (u.name.eq_ignore_ascii_case(name)
                    || (u.name.eq_ignore_ascii_case(first)
                        && u.last_name.as_deref().is_some_and(|l| {
                            name.eq_ignore_ascii_case(&format!("{} {l}", u.name))
                        })))
        })
        .map(|u| match &u.last_name {
            Some(l) => format!("{} {l}", u.name),
            None => u.name.clone(),
        })
}

fn add_ignore_named(vm: &mut Vm, name: &str) {
    let line = if is_own_name(vm, name) {
        global(vm, "ERR_IGNORE_SELF", "You can't ignore yourself.")
    } else if let Some(i) = ignored_at(vm, name) {
        let n = engine(vm).world.chat.ignores[i].clone();
        global(vm, "ERR_IGNORE_ALREADY_S", "%s is already being ignored.").replacen("%s", &n, 1)
    } else if let Some(n) = known_player(vm, name) {
        engine_mut(vm).world.chat.ignores.push(n.clone());
        ignores_changed(vm);
        global(vm, "ERR_IGNORE_ADDED_S", "%s is now being ignored.").replacen("%s", &n, 1)
    } else {
        global(vm, "ERR_IGNORE_NOT_FOUND", "Player not found.")
    };
    system_later(vm, line);
}

fn del_ignore_named(vm: &mut Vm, name: &str) {
    let line = match ignored_at(vm, name) {
        Some(i) => {
            let n = engine_mut(vm).world.chat.ignores.remove(i);
            ignores_changed(vm);
            global(vm, "ERR_IGNORE_REMOVED_S", "%s is no longer being ignored.")
                .replacen("%s", &n, 1)
        }
        None => global(vm, "ERR_IGNORE_NOT_FOUND", "Player not found."),
    };
    system_later(vm, line);
}

/// `IGNORELIST_UPDATE` a round trip later.
fn ignores_changed(vm: &mut Vm) {
    let f = vm.native("ignores changed", |vm, _| {
        widgets::fire_event(vm, "IGNORELIST_UPDATE", &[]);
        Ok(vec![])
    });
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, f);
}

/// `C_FriendList.AddIgnore(name)`: ignored, if the world has that
/// player; the server's answer either way.
fn add_ignore(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(name) = text_arg(vm, c.arg(1)) {
        add_ignore_named(vm, &name);
    }
    Ok(vec![Value::Bool(true)])
}

/// `C_FriendList.DelIgnore(name)`.
fn del_ignore(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(name) = text_arg(vm, c.arg(1)) {
        del_ignore_named(vm, &name);
    }
    Ok(vec![Value::Bool(true)])
}

/// `C_FriendList.AddOrDelIgnore(name)` (`/ignore`): off the list if on it,
/// else on.
fn add_or_del_ignore(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(name) = text_arg(vm, c.arg(1)) {
        if ignored_at(vm, &name).is_some() {
            del_ignore_named(vm, &name);
        } else {
            add_ignore_named(vm, &name);
        }
    }
    Ok(vec![])
}

/// `C_FriendList.DelIgnoreByIndex(index)`.
fn del_ignore_by_index(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Value::Num(i) = vm.hidden(c.arg(1)) {
        let name = engine(vm)
            .world
            .chat
            .ignores
            .get((i as usize).wrapping_sub(1))
            .cloned();
        if let Some(name) = name {
            del_ignore_named(vm, &name);
        }
    }
    Ok(vec![])
}

/// `C_FriendList.GetNumIgnores()`.
fn get_num_ignores(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(engine(vm).world.chat.ignores.len() as f64)])
}

/// `C_FriendList.GetIgnoreName(index)`: nil past the list.
fn get_ignore_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = match vm.hidden(c.arg(1)) {
        Value::Num(i) if i >= 1.0 => engine(vm).world.chat.ignores.get(i as usize - 1).cloned(),
        _ => None,
    };
    Ok(vec![name.map_or(Value::Nil, |n| vm.str(&n))])
}

/// `C_FriendList.IsIgnored(name)`, `IsOnIgnoredList(name)`.
fn is_ignored(vm: &mut Vm, c: NativeCall) -> Ret {
    let on = text_arg(vm, c.arg(1)).is_some_and(|n| ignored_at(vm, &n).is_some());
    Ok(vec![Value::Bool(on)])
}

// ----------------------------------------------------------------- who

/// A player's name as the server writes it in a who: first and last name,
/// else the name.
fn who_name(u: &units::Unit) -> String {
    match &u.last_name {
        Some(l) => format!("{} {l}", u.name),
        None => u.name.clone(),
    }
}

/// The online players a who's filter finds (decision T363): the player
/// and the world's other players. Plain words match a name, zone, guild,
/// class or race (any case, a part); `n-`, `z-`, `g-`, `c-`, `r-` one of
/// them; `20-30` (or `60`) the levels.
fn who_matches(vm: &mut Vm, filter: &str) -> Vec<usize> {
    let player_zone = match vm.get_global("GetZoneText") {
        f @ Value::Func(_) => match vm.call(f, &[]).ok().and_then(|r| r.first().copied()) {
            Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    let guild = crate::guild::guild_name(vm).unwrap_or_default();
    let e = engine(vm);
    let mut out = Vec::new();
    for (i, u) in e.units.list.iter().enumerate() {
        if !(u.player || i == units::PLAYER.0) || !u.online {
            continue;
        }
        let zone = if i == units::PLAYER.0 {
            player_zone.clone()
        } else {
            u.zone.clone().unwrap_or_else(|| player_zone.clone())
        };
        let their_guild = if i == units::PLAYER.0 {
            guild.clone()
        } else {
            String::new()
        };
        let has = |field: &str, word: &str| field.to_lowercase().contains(&word.to_lowercase());
        let ok = filter.split_whitespace().all(|word| {
            if let Some((a, b)) = word.split_once('-')
                && let (Ok(lo), Ok(hi)) = (a.parse::<f64>(), b.parse::<f64>())
            {
                return u.level >= lo && u.level <= hi;
            }
            if let Ok(l) = word.parse::<f64>() {
                return u.level == l;
            }
            match word.split_once('-') {
                Some(("n", w)) => has(&who_name(u), w),
                Some(("z", w)) => has(&zone, w),
                Some(("g", w)) => has(&their_guild, w),
                Some(("c", w)) => has(&u.class_name, w),
                Some(("r", w)) => has(&u.race, w),
                _ => {
                    has(&who_name(u), word)
                        || has(&zone, word)
                        || has(&their_guild, word)
                        || has(&u.class_name, word)
                        || has(&u.race, word)
                }
            }
        });
        if ok {
            out.push(i);
        }
    }
    out
}

/// `C_FriendList.SendWho(filter)` (`/who`): the online players the filter
/// finds, a round trip later (`WHO_LIST_UPDATE`); in chat their lines
/// (`WHO_LIST_FORMAT`, three at most) and the count (`WHO_NUM_RESULTS`).
/// Unverified: whether a who from the who frame prints too, the lines'
/// limit, the results' order.
fn send_who(vm: &mut Vm, c: NativeCall) -> Ret {
    let filter = text_arg(vm, c.arg(1)).unwrap_or_default();
    let found = who_matches(vm, &filter);
    engine_mut(vm).world.chat.who = found;
    let f = vm.native("who list", |vm, _| {
        widgets::fire_event(vm, "WHO_LIST_UPDATE", &[]);
        let found = engine(vm).world.chat.who.clone();
        if found.len() <= 3 {
            let format = global(
                vm,
                "WHO_LIST_FORMAT",
                "|Hplayer:%s|h[%s]|h: Level %d %s %s - %s",
            );
            for i in &found {
                let info = who_fields(vm, *i);
                let line = format
                    .replacen("%s", &info.0, 1)
                    .replacen("%s", &info.0, 1)
                    .replacen("%d", &format!("{}", info.1), 1)
                    .replacen("%s", &info.2, 1)
                    .replacen("%s", &info.3, 1)
                    .replacen("%s", &info.4, 1);
                crate::chat::game_message(vm, "CHAT_MSG_SYSTEM", &line, None);
            }
        }
        let total = global(vm, "WHO_NUM_RESULTS", "%d |4player:players; total").replacen(
            "%d",
            &found.len().to_string(),
            1,
        );
        crate::chat::game_message(vm, "CHAT_MSG_SYSTEM", &total, None);
        Ok(vec![])
    });
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, f);
    Ok(vec![])
}

/// A who result's name, level, race, class and zone.
fn who_fields(vm: &mut Vm, i: usize) -> (String, f64, String, String, String) {
    let zone = if i == units::PLAYER.0 {
        match vm.get_global("GetZoneText") {
            f @ Value::Func(_) => match vm.call(f, &[]).ok().and_then(|r| r.first().copied()) {
                Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                _ => String::new(),
            },
            _ => String::new(),
        }
    } else {
        engine(vm).units.list[i].zone.clone().unwrap_or_default()
    };
    let u = &engine(vm).units.list[i];
    (
        who_name(u),
        u.level,
        u.race.clone(),
        u.class_name.clone(),
        zone,
    )
}

/// `C_FriendList.GetNumWhoResults()`: the last who's count, twice.
fn get_num_who_results(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = engine(vm).world.chat.who.len() as f64;
    Ok(vec![Value::Num(n), Value::Num(n)])
}

/// `C_FriendList.GetWhoInfo(index)`: a result's `WhoInfo`.
fn get_who_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(n) = vm.hidden(c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let Some(&i) = engine(vm).world.chat.who.get((n as usize).wrapping_sub(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let (name, level, race, class, zone) = who_fields(vm, i);
    let guild = if i == units::PLAYER.0 {
        crate::guild::guild_name(vm).unwrap_or_default()
    } else {
        String::new()
    };
    let (file, sex) = {
        let u = &engine(vm).units.list[i];
        (u.class.clone(), u.sex)
    };
    let t = vm.new_table();
    let set = |vm: &mut Vm, k: &str, v: Value| {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, v);
    };
    let v = vm.str(&name);
    set(vm, "fullName", v);
    let v = vm.str(&guild);
    set(vm, "fullGuildName", v);
    set(vm, "level", Value::Num(level));
    let v = vm.str(&race);
    set(vm, "raceStr", v);
    let v = vm.str(&class);
    set(vm, "classStr", v);
    let v = vm.str(&zone);
    set(vm, "area", v);
    let v = vm.str(&file);
    set(vm, "filename", v);
    set(vm, "gender", Value::Num(sex));
    Ok(vec![Value::Table(t)])
}

// --------------------------------------------------------------- emotes

/// `C_ChatInfo.PerformEmote(emote, target)`: false, the emote not
/// restricted (Blizzard's chat box reads the answer so: a true one shows
/// the unknown command's help; unverified: the emote's chat line,
/// `CHAT_MSG_TEXT_EMOTE`, from the client's emote texts).
fn perform_emote(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

// --------------------------------------------------------------- guild

/// `C_GuildInfo.Invite(name)`, `GuildInvite(name)` (`/ginvite`): the world
/// has no one else to invite, so the server's "not found"; a player it
/// has is invited (unverified: the invited player's answer).
fn guild_invite(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(name) = text_arg(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    if !crate::guild::in_guild(vm) {
        let t = global(
            vm,
            "ERR_GUILD_PLAYER_NOT_IN_GUILD",
            "You are not in a guild.",
        );
        system_later(vm, t);
    } else if is_own_name(vm, &name) {
        let t = global(
            vm,
            "ERR_GUILD_INVITE_SELF",
            "You can't invite yourself to a guild.",
        );
        system_later(vm, t);
    } else if let Some(n) = known_player(vm, &name) {
        if crate::guild::has_member(vm, &n) {
            let t = global(vm, "ERR_ALREADY_IN_GUILD_S", "%s is already in a guild.")
                .replacen("%s", &n, 1);
            system_later(vm, t);
            return Ok(vec![]);
        }
        let t = global(
            vm,
            "ERR_GUILD_INVITE_S",
            "You have invited %s to join your guild.",
        )
        .replacen("%s", &n, 1);
        system_later(vm, t);
        // A player who accepts invitations joins a moment later.
        if let Some(i) = unit_of_name(vm, &n)
            && engine(vm).units.list[i].accepts_invites
        {
            let f = vm.native_with("guild accepted", guild_accepted, vec![Value::Num(i as f64)]);
            let delay = crate::clock::server_delay(vm) + 1.5;
            crate::clock::after_engine(vm, delay, f);
        }
    } else {
        let t = global(vm, "ERR_GUILD_PLAYER_NOT_FOUND_S", "\"%s\" not found.")
            .replacen("%s", &name, 1);
        system_later(vm, t);
    }
    Ok(vec![])
}

/// The index of the world's player unit with that name (as the server
/// writes it).
fn unit_of_name(vm: &Vm, name: &str) -> Option<usize> {
    engine(vm)
        .units
        .list
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, u)| u.player && who_name(u).eq_ignore_ascii_case(name))
        .map(|(i, _)| i)
}

fn guild_accepted(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(Value::Num(i)) = c.upvals.first().copied() {
        let id = units::UnitId(i as usize);
        let name = who_name(&engine(vm).units.list[id.0]);
        if crate::guild::in_guild(vm) && !crate::guild::has_member(vm, &name) {
            crate::guild::join_from_unit(vm, id);
        }
    }
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("RandomRoll", random_roll),
        ("RequestTimePlayed", request_time_played),
        ("UnitIsAFK", unit_is_afk),
        ("UnitIsDND", unit_is_dnd),
        ("C_FriendList.AddIgnore", add_ignore),
        ("C_FriendList.DelIgnore", del_ignore),
        ("C_FriendList.AddOrDelIgnore", add_or_del_ignore),
        ("C_FriendList.DelIgnoreByIndex", del_ignore_by_index),
        ("C_FriendList.GetNumIgnores", get_num_ignores),
        ("C_FriendList.GetIgnoreName", get_ignore_name),
        ("C_FriendList.IsIgnored", is_ignored),
        ("C_FriendList.IsOnIgnoredList", is_ignored),
        ("C_FriendList.SendWho", send_who),
        ("C_FriendList.GetNumWhoResults", get_num_who_results),
        ("C_FriendList.GetWhoInfo", get_who_info),
        ("C_ChatInfo.PerformEmote", perform_emote),
        ("C_GuildInfo.Invite", guild_invite),
        ("GuildInvite", guild_invite),
    ]
}
