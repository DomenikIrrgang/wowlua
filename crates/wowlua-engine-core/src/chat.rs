//! Addon messages and slash commands.
//!
//! Addon messages: `C_ChatInfo` registers prefixes and sends; a sent
//! message is kept for the test (`sentAddonMessages`), and one the player
//! receives (to a group or guild they are in, a whisper to themselves)
//! comes back as `CHAT_MSG_ADDON` on the next frame, for a registered
//! prefix only. Tests deliver other players' messages
//! (`receiveAddonMessage`). As `/stdtest engine comm` recorded: prefixes
//! of 1 to 16 characters (else InvalidPrefix), a duplicate refused; SAY and
//! unknown chat types InvalidChatType, a whisper without a target
//! TargetRequired; messages up to 8192 bytes (and zero bytes) go out; a
//! whisper to the player's "First Last" arrives from "First Last" (half a
//! second later there: the server's round trip; here on the next frame).
//! Unverified: the message limit and delivery of long messages, the prefix
//! count, the echo to groups.
//!
//! Slash commands: the `slash(line)` control runs `/cmd args` as the chat
//! box does, through `SlashCmdList` and the `SLASH_<KEY><n>` globals.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::{engine, engine_mut, units, widgets};

/// A sent addon message.
#[derive(Clone)]
pub struct Sent {
    pub prefix: String,
    pub message: Vec<u8>,
    pub chat_type: String,
    pub target: Option<String>,
}

/// Prefixes and messages.
#[derive(Default)]
pub struct Chat {
    pub prefixes: Vec<String>,
    pub sent: Vec<Sent>,
    /// The player's guild, if any (`guild.rs`).
    pub guild: Option<crate::guild::Guild>,
    /// Away (`/afk`) and busy (`/dnd`): their messages (`replies.rs`).
    pub afk: Option<String>,
    pub dnd: Option<String>,
    /// The ignore list's names, in order added.
    pub ignores: Vec<String>,
    /// The server's dice (`/roll`): its own generator's state.
    pub rolls: u32,
    /// The last who's results: the units' indices (`replies.rs`).
    pub who: Vec<usize>,
}

const MAX_PREFIX: usize = 16;
const MAX_PREFIXES: usize = 128;

/// `SendAddonMessageResult`'s values.
mod result {
    pub const SUCCESS: f64 = 0.0;
    pub const INVALID_PREFIX: f64 = 1.0;
    pub const INVALID_CHAT_TYPE: f64 = 4.0;
    pub const NOT_IN_GROUP: f64 = 5.0;
    pub const TARGET_REQUIRED: f64 = 6.0;
    pub const NOT_IN_GUILD: f64 = 10.0;
    pub const LOCKDOWN: f64 = 11.0;
}

fn text(vm: &mut Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn register_prefix(vm: &mut Vm, c: NativeCall) -> Ret {
    let prefix = text(vm, c.arg(1)).unwrap_or_default();
    let chat = &mut engine_mut(vm).world.chat;
    let r = if prefix.is_empty() || prefix.len() > MAX_PREFIX {
        2.0
    } else if chat.prefixes.contains(&prefix) {
        1.0
    } else if chat.prefixes.len() >= MAX_PREFIXES {
        3.0
    } else {
        chat.prefixes.push(prefix);
        0.0
    };
    Ok(vec![Value::Num(r)])
}

fn is_prefix_registered(vm: &mut Vm, c: NativeCall) -> Ret {
    let prefix = text(vm, c.arg(1)).unwrap_or_default();
    let yes = engine(vm).world.chat.prefixes.contains(&prefix);
    Ok(vec![Value::Bool(yes)])
}

fn registered_prefixes(vm: &mut Vm, _c: NativeCall) -> Ret {
    let list = engine(vm).world.chat.prefixes.clone();
    let t = vm.new_table();
    for (i, p) in list.iter().enumerate() {
        let s = vm.str(p);
        let _ = vm.heap.table_mut(t).set(Value::Num(i as f64 + 1.0), s);
    }
    Ok(vec![Value::Table(t)])
}

fn messaging_lockdown(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(units::active(
        &engine(vm).units,
        &[units::CHAT],
    ))])
}

/// The name others address the player by: on WoW Forever the first and
/// last name (`/stdtest engine comm`: a whisper to "First Last" arrives,
/// from "First Last"; the first name alone or Name-Realm reach nobody),
/// elsewhere Name-Realm.
fn player_full_name(vm: &Vm) -> String {
    let p = &engine(vm).units.list[units::PLAYER.0];
    match &p.last_name {
        Some(last) => format!("{} {last}", p.name),
        None => format!(
            "{}-{}",
            p.name,
            p.realm.clone().unwrap_or_else(|| "Realm".into())
        ),
    }
}

/// Fires `CHAT_MSG_ADDON` for a registered prefix (the rest of the payload
/// empty).
fn deliver(vm: &mut Vm, prefix: &str, message: &[u8], chat_type: &str, sender: &str, target: &str) {
    if !engine(vm).world.chat.prefixes.iter().any(|p| p == prefix) {
        return;
    }
    let args = [
        vm.str(prefix),
        vm.bytes(message),
        vm.str(chat_type),
        vm.str(sender),
        vm.str(target),
        Value::Num(0.0),
        Value::Num(0.0),
        vm.str(""),
        Value::Num(0.0),
    ];
    widgets::fire_event(vm, "CHAT_MSG_ADDON", &args);
}

/// The native that delivers a message on the next frame.
fn delivery(vm: &mut Vm, c: NativeCall) -> Ret {
    let [prefix, message, chat_type, sender, target] =
        [0, 1, 2, 3, 4].map(|i| c.upvals.get(i).copied().unwrap_or(Value::Nil));
    let s = |vm: &mut Vm, v: Value| text(vm, v).unwrap_or_default();
    let message = match message {
        Value::Str(m) => vm.str_bytes(m).to_vec(),
        _ => Vec::new(),
    };
    let (prefix, chat_type, sender, target) = (
        s(vm, prefix),
        s(vm, chat_type),
        s(vm, sender),
        s(vm, target),
    );
    deliver(vm, &prefix, &message, &chat_type, &sender, &target);
    Ok(vec![])
}

fn send(vm: &mut Vm, c: NativeCall) -> Ret {
    let prefix = text(vm, c.arg(1)).unwrap_or_default();
    let message = match vm.hidden(c.arg(2)) {
        Value::Str(s) => vm.str_bytes(s).to_vec(),
        _ => Vec::new(),
    };
    let chat_type = text(vm, c.arg(3))
        .unwrap_or_else(|| "PARTY".into())
        .to_uppercase();
    let target = text(vm, c.arg(4));
    let u = &engine(vm).units;
    let in_group = !u.group.is_empty();
    let r = if prefix.is_empty() || prefix.len() > MAX_PREFIX {
        result::INVALID_PREFIX
    } else if units::active(u, &[units::CHAT]) {
        result::LOCKDOWN
    } else {
        match chat_type.as_str() {
            "PARTY" | "RAID" | "INSTANCE_CHAT" if !in_group => result::NOT_IN_GROUP,
            "GUILD" | "OFFICER" if !crate::guild::in_guild(vm) => result::NOT_IN_GUILD,
            "WHISPER" | "CHANNEL" if target.is_none() => result::TARGET_REQUIRED,
            "PARTY" | "RAID" | "INSTANCE_CHAT" | "GUILD" | "OFFICER" | "WHISPER" | "CHANNEL" => {
                result::SUCCESS
            }
            _ => result::INVALID_CHAT_TYPE,
        }
    };
    if r != result::SUCCESS {
        return Ok(vec![Value::Num(r)]);
    }
    engine_mut(vm).world.chat.sent.push(Sent {
        prefix: prefix.clone(),
        message: message.clone(),
        chat_type: chat_type.clone(),
        target: target.clone(),
    });
    // The player hears their own messages to groups and guilds, and a
    // whisper to themselves.
    let me = player_full_name(vm);
    // A whisper reaches the player when addressed by that name (with a last
    // name, the first name alone does not).
    let player = &engine(vm).units.list[units::PLAYER.0];
    let first_name_reaches = player.last_name.is_none();
    let own_name = player.name.clone();
    let to_self = chat_type == "WHISPER"
        && target.as_deref().is_some_and(|t| {
            t.eq_ignore_ascii_case(&me) || (first_name_reaches && t.eq_ignore_ascii_case(&own_name))
        });
    if chat_type != "WHISPER" || to_self {
        let upvals = vec![
            vm.str(&prefix),
            vm.bytes(&message),
            vm.str(&chat_type),
            vm.str(&me),
            vm.str(target.as_deref().unwrap_or("")),
        ];
        let f = vm.native_with("delivery", delivery, upvals);
        crate::clock::after_engine(vm, 0.0, f);
    }
    Ok(vec![Value::Num(result::SUCCESS)])
}

/// `receiveAddonMessage(prefix, message, chatType, sender)`: another
/// player's message arrives.
/// A message of the game's own (`CHAT_MSG_COMBAT_XP_GAIN`, `_SYSTEM`,
/// `_LOOT`, …), its payload as recorded (`/stdtest engine scenario
/// combat`): the text, five empty strings, 0, 0, "", 0, the line's number,
/// no GUID, 0, four falses, the Discord fields; `sender` the player's full
/// name for loot (with the player's GUID).
pub fn game_message(vm: &mut Vm, event: &str, text: &str, sender: Option<&str>) {
    let guid = sender.map(|_| {
        crate::engine(vm).units.list[crate::units::PLAYER.0]
            .guid
            .clone()
    });
    chat_line(vm, event, text, sender, ("", 0.0), guid);
}

/// A chat line's event, the payload as [`game_message`]'s: the text, its
/// author (also the fifth value), the language's name and ID, the author's
/// GUID.
fn chat_line(
    vm: &mut Vm,
    event: &str,
    text: &str,
    sender: Option<&str>,
    language: (&str, f64),
    guid: Option<String>,
) {
    let line = {
        let e = crate::engine_mut(vm);
        e.world.next_chat_line += 1;
        e.world.next_chat_line
    };
    let s = |vm: &mut Vm, t: &str| vm.str(t);
    let name = sender.unwrap_or("");
    let discord = vm.new_table();
    for k in ["forwardedMessage", "globalName", "lastOnlineName"] {
        let key = vm.str(k);
        let empty = vm.str("");
        let _ = vm.heap.table_mut(discord).set(key, empty);
    }
    for k in [
        "fromDiscord",
        "hasAttachment",
        "hasEmbed",
        "hasEmoji",
        "hasError",
        "hasForwardedMessage",
        "hasPoll",
        "hasSticker",
    ] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(discord).set(key, Value::Bool(false));
    }
    for k in ["type", "userID"] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(discord).set(key, Value::Num(0.0));
    }
    let guid = match guid {
        Some(g) => vm.str(&g),
        None => Value::Nil,
    };
    let args = vec![
        s(vm, text),
        s(vm, name),
        s(vm, language.0),
        s(vm, ""),
        s(vm, name),
        s(vm, ""),
        Value::Num(0.0),
        Value::Num(0.0),
        s(vm, ""),
        Value::Num(language.1),
        Value::Num(line as f64),
        guid,
        Value::Num(0.0),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
        Value::Table(discord),
    ];
    crate::widgets::fire_event(vm, event, &args);
}

/// `C_ChatInfo.SendChatMessage(message, chatType, languageID, target)`
/// (`SendChatMessage`): the server's answer on the next frame (decision
/// T360; unverified on WoW Forever: the lines' order and payloads, the
/// leader's chat types, channels). The player hears what they say, yell
/// and emote (`CHAT_MSG_SAY`, …, in their language), and their party,
/// raid, instance, guild and officer lines when they are in one, else the
/// server's error (`ERR_NOT_IN_GROUP`, `ERR_NOT_IN_RAID`,
/// `ERR_GUILD_PLAYER_NOT_IN_GUILD`). A whisper shows as sent
/// (`CHAT_MSG_WHISPER_INFORM`) to the player or a player the world has,
/// a whisper to the player arriving too (`CHAT_MSG_WHISPER`); to a name
/// the world does not have, `ERR_CHAT_PLAYER_NOT_FOUND_S`. A channel's
/// message is not answered.
fn send_chat(vm: &mut Vm, c: NativeCall) -> Ret {
    let message = match vm.hidden(c.arg(1)) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => n.to_string(),
        _ => String::new(),
    };
    let chat_type = text(vm, c.arg(2))
        .unwrap_or_else(|| "SAY".into())
        .to_uppercase();
    if message.is_empty() && chat_type != "AFK" && chat_type != "DND" {
        return Ok(vec![]);
    }
    let target = text(vm, c.arg(4)).filter(|t| !t.is_empty());
    let me = player_full_name(vm);
    let my_guid = engine(vm).units.list[units::PLAYER.0].guid.clone();
    let language = default_language(vm);
    let in_group = !engine(vm).units.group.is_empty();
    let in_raid = in_group && engine(vm).units.raid;
    let err = |vm: &mut Vm, key: &str, fallback: &str| -> String {
        match vm.get_global(key) {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => fallback.to_string(),
        }
    };
    // (event, text, author, author's GUID; a system line has none).
    let mut lines: Vec<(String, String, Option<String>, Option<String>)> = Vec::new();
    let mine = |event: &str| {
        (
            event.to_string(),
            message.clone(),
            Some(me.clone()),
            Some(my_guid.clone()),
        )
    };
    match chat_type.as_str() {
        // Away and busy: the message is their text (decision T362).
        "AFK" | "DND" => {
            crate::replies::toggle_status(vm, &chat_type, &message);
            return Ok(vec![]);
        }
        "SAY" | "YELL" | "EMOTE" => lines.push(mine(&format!("CHAT_MSG_{chat_type}"))),
        "PARTY" | "INSTANCE_CHAT" if in_group => lines.push(mine(&format!("CHAT_MSG_{chat_type}"))),
        "PARTY" | "INSTANCE_CHAT" => {
            let t = err(vm, "ERR_NOT_IN_GROUP", "You aren't in a party.");
            lines.push(("CHAT_MSG_SYSTEM".into(), t, None, None));
        }
        "RAID" if in_raid => lines.push(mine("CHAT_MSG_RAID")),
        "RAID" => {
            let t = err(vm, "ERR_NOT_IN_RAID", "You are not in a raid group");
            lines.push(("CHAT_MSG_SYSTEM".into(), t, None, None));
        }
        "GUILD" | "OFFICER" if crate::guild::in_guild(vm) => {
            lines.push(mine(&format!("CHAT_MSG_{chat_type}")));
        }
        "GUILD" | "OFFICER" => {
            let t = err(
                vm,
                "ERR_GUILD_PLAYER_NOT_IN_GUILD",
                "You are not in a guild.",
            );
            lines.push(("CHAT_MSG_SYSTEM".into(), t, None, None));
        }
        "WHISPER" => {
            let Some(target) = target else {
                return Ok(vec![]);
            };
            if whispers_self(vm, &target) {
                lines.push((
                    "CHAT_MSG_WHISPER_INFORM".into(),
                    message.clone(),
                    Some(me.clone()),
                    Some(my_guid.clone()),
                ));
                lines.push(mine("CHAT_MSG_WHISPER"));
            } else if let Some((name, guid)) = known_player(vm, &target) {
                lines.push((
                    "CHAT_MSG_WHISPER_INFORM".into(),
                    message.clone(),
                    Some(name),
                    Some(guid),
                ));
            } else {
                let f = err(
                    vm,
                    "ERR_CHAT_PLAYER_NOT_FOUND_S",
                    "No player named '%s' is currently playing.",
                );
                lines.push((
                    "CHAT_MSG_SYSTEM".into(),
                    f.replacen("%s", &target, 1),
                    None,
                    None,
                ));
            }
        }
        _ => {}
    }
    if lines.is_empty() {
        return Ok(vec![]);
    }
    // Answered on the next frame, as the server's addon messages are.
    let mut upvals = vec![vm.str(&language.0), Value::Num(language.1)];
    for (event, text, author, guid) in lines {
        upvals.push(vm.str(&event));
        upvals.push(vm.str(&text));
        upvals.push(author.map_or(Value::Nil, |a| vm.str(&a)));
        upvals.push(guid.map_or(Value::Nil, |g| vm.str(&g)));
    }
    let f = vm.native_with("chat answer", chat_answer, upvals);
    crate::clock::after_engine(vm, 0.0, f);
    Ok(vec![])
}

/// The server's answer to a chat message: its lines, in order.
fn chat_answer(vm: &mut Vm, c: NativeCall) -> Ret {
    let up = c.upvals.clone();
    let s = |vm: &Vm, v: Value| match v {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    };
    let language = (
        s(vm, up[0]).unwrap_or_default(),
        match up[1] {
            Value::Num(n) => n,
            _ => 0.0,
        },
    );
    for chunk in up[2..].chunks(4) {
        let event = s(vm, chunk[0]).unwrap_or_default();
        let text = s(vm, chunk[1]).unwrap_or_default();
        let author = s(vm, chunk[2]);
        let guid = s(vm, chunk[3]);
        // A system line has no language.
        let lang = if author.is_some() {
            (language.0.as_str(), language.1)
        } else {
            ("", 0.0)
        };
        chat_line(vm, &event, &text, author.as_deref(), lang, guid);
    }
    Ok(vec![])
}

/// The player's language: `GetDefaultLanguage()`'s name and ID.
fn default_language(vm: &mut Vm) -> (String, f64) {
    let f = vm.get_global("GetDefaultLanguage");
    if f.is_nil() {
        return (String::new(), 0.0);
    }
    let unit = vm.str("player");
    let r = vm.call(f, &[unit]).unwrap_or_default();
    let name = match r.first() {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(*s)).into_owned(),
        _ => String::new(),
    };
    let id = match r.get(1) {
        Some(Value::Num(n)) => *n,
        _ => 0.0,
    };
    (name, id)
}

/// Whether a whisper's target is the player (as addon messages reach them:
/// the full name, or the first name without a last one).
fn whispers_self(vm: &Vm, target: &str) -> bool {
    let me = player_full_name(vm);
    let player = &engine(vm).units.list[units::PLAYER.0];
    target.eq_ignore_ascii_case(&me)
        || (player.last_name.is_none() && target.eq_ignore_ascii_case(&player.name))
}

/// Another player the world has by that name (with or without the realm):
/// their full name and GUID.
fn known_player(vm: &Vm, target: &str) -> Option<(String, String)> {
    let wanted = target.split('-').next().unwrap_or(target);
    engine(vm)
        .units
        .list
        .iter()
        .skip(1)
        .find(|u| u.player && u.name.eq_ignore_ascii_case(wanted))
        .map(|u| {
            let full = match (&u.last_name, &u.realm) {
                (Some(last), _) => format!("{} {last}", u.name),
                (None, Some(realm)) => format!("{}-{realm}", u.name),
                (None, None) => u.name.clone(),
            };
            (full, u.guid.clone())
        })
}

pub fn receive_control(vm: &mut Vm, c: NativeCall) -> Ret {
    let prefix = text(vm, c.arg(1)).unwrap_or_default();
    let message = match c.arg(2) {
        Value::Str(s) => vm.str_bytes(s).to_vec(),
        _ => Vec::new(),
    };
    let chat_type = text(vm, c.arg(3)).unwrap_or_else(|| "PARTY".into());
    let sender = text(vm, c.arg(4)).unwrap_or_else(|| "Someone-Realm".into());
    deliver(vm, &prefix, &message, &chat_type, &sender, "");
    Ok(vec![])
}

/// `sentAddonMessages()`: what the addon sent, as tables (`prefix`,
/// `message`, `chatType`, `target`).
pub fn sent_control(vm: &mut Vm, _c: NativeCall) -> Ret {
    let sent = engine(vm).world.chat.sent.clone();
    let out = vm.new_table();
    for (i, m) in sent.iter().enumerate() {
        let t = vm.new_table();
        let fields = [
            ("prefix", vm.str(&m.prefix)),
            ("message", vm.bytes(&m.message)),
            ("chatType", vm.str(&m.chat_type)),
            (
                "target",
                m.target.as_deref().map_or(Value::Nil, |s| vm.str(s)),
            ),
        ];
        for (k, v) in fields {
            let key = vm.str(k);
            let _ = vm.heap.table_mut(t).set(key, v);
        }
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num(i as f64 + 1.0), Value::Table(t));
    }
    Ok(vec![Value::Table(out)])
}

/// `systemMessage(text)`: a message of the server's own, as a friend's
/// coming online (`CHAT_MSG_SYSTEM`, the game's message payload; recorded,
/// `profession`: "… has gone offline.", "|Hplayer:…|h[…]|h has come
/// online.").
pub fn system_message_control(vm: &mut Vm, c: NativeCall) -> Ret {
    let line = vm.check_str(&c, 1)?;
    let line = String::from_utf8_lossy(vm.str_bytes(line)).into_owned();
    game_message(vm, "CHAT_MSG_SYSTEM", &line, None);
    Ok(vec![])
}

/// `slash(line)`: runs `/cmd args` as the chat box does, the user's action
/// (a hardware event). Returns whether a command took it.
/// The default chat frame's edit box and its `SetText`, when Blizzard's chat
/// is loaded (its `ChatFrameEditBoxBaseMixin` there: `SendText`).
fn chat_edit_box(vm: &mut Vm) -> Option<(Value, Value)> {
    // (through metatables: `SetText` is the widget's method)
    let get = |vm: &mut Vm, t: Value, key: &str| -> Value {
        if !matches!(t, Value::Table(_)) {
            return Value::Nil;
        }
        let k = vm.str(key);
        vm.index(t, k, None).unwrap_or(Value::Nil)
    };
    let frame = vm.get_global("DEFAULT_CHAT_FRAME");
    let edit = get(vm, frame, "editBox");
    let send = get(vm, edit, "SendText");
    let set_text = get(vm, edit, "SetText");
    match (send, set_text) {
        (Value::Func(_), Value::Func(_)) => Some((edit, set_text)),
        _ => None,
    }
}

pub fn slash_control(vm: &mut Vm, c: NativeCall) -> Ret {
    let line = vm.check_str(&c, 1)?;
    let line = String::from_utf8_lossy(vm.str_bytes(line)).into_owned();
    let line = line.trim();
    // With Blizzard's chat: the line typed into the chat's edit box and sent
    // as the player does it (decision T233), so the client's own lookup
    // runs: its secure commands, chat types, emotes, the commands it moved
    // out of `SlashCmdList` into its hash tables (all of Blizzard's: they
    // were never found), and its help line for an unknown one.
    if let Some((edit, set_text)) = chat_edit_box(vm)
        && let Some(id) = widgets::widget_of(vm, edit)
    {
        let text = vm.str(line);
        // Typed, then Enter: the edit box's own handler sends it and closes
        // the edit box, as the key does.
        crate::input::as_user(vm, |vm| {
            vm.call_from_engine(set_text, &[edit, text]);
            widgets::run_script(vm, id, "OnEnterPressed", &[]);
        });
        return Ok(vec![Value::Bool(true)]);
    }
    let (cmd, msg) = match line.split_once(char::is_whitespace) {
        Some((cmd, rest)) => (cmd.to_string(), rest.trim_start().to_string()),
        None => (line.to_string(), String::new()),
    };
    let Value::Table(list) = vm.get_global("SlashCmdList") else {
        return Ok(vec![Value::Bool(false)]);
    };
    let mut k = Value::Nil;
    let mut found = None;
    while let Some((key, handler)) = vm.heap.table(list).next(k).ok().flatten() {
        k = key;
        let Value::Str(name) = key else { continue };
        let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
        for i in 1.. {
            match vm.get_global(&format!("SLASH_{name}{i}")) {
                Value::Str(s) => {
                    if String::from_utf8_lossy(vm.str_bytes(s)).eq_ignore_ascii_case(&cmd) {
                        found = Some(handler);
                    }
                }
                _ => break,
            }
        }
        if found.is_some() {
            break;
        }
    }
    let Some(handler) = found else {
        return Ok(vec![Value::Bool(false)]);
    };
    let msg = vm.str(&msg);
    // Typed and sent with Enter: the user's action, a hardware event (an
    // addon's group finder search from a slash command was allowed,
    // `/suu probe social`).
    crate::input::as_user(vm, |vm| vm.call_from_engine(handler, &[msg, Value::Nil]));
    Ok(vec![Value::Bool(true)])
}

/// The functions with behaviour here.
/// At login every chat window's `AddMessage` is hooked (decision T514):
/// Blizzard's chat windows add their lines with a Lua mixin's
/// `AddMessage` the engine does not run, so the hook logs them as the
/// engine's own does.
pub fn hook_chat_windows(vm: &mut Vm) {
    let hook = vm.get_global("hooksecurefunc");
    if hook.is_nil() {
        return;
    }
    let windows: Vec<(usize, wowlua_vm::TableRef)> = (0..engine(vm).world.widgets.len())
        .filter(|&i| widgets::chat_window(vm, widgets::WidgetId(i)))
        .map(|i| (i, engine(vm).world.widgets[i].table))
        .filter(|(i, _)| !engine(vm).world.chat_hooked.contains(i))
        .collect();
    for (i, table) in windows {
        let record = vm.native("chat log", record_chat_line);
        let method = vm.str("AddMessage");
        if vm
            .pcall(hook, &[Value::Table(table), method, record])
            .is_ok()
        {
            engine_mut(vm).world.chat_hooked.insert(i);
        }
    }
}

/// A hooked chat window's `AddMessage(self, text, r, g, b, …)`.
fn record_chat_line(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(id) = widgets::widget_of(vm, c.arg(1)) {
        widgets::log_chat_line(vm, id, &c);
    }
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_ChatInfo.RegisterAddonMessagePrefix", register_prefix),
        (
            "C_ChatInfo.IsAddonMessagePrefixRegistered",
            is_prefix_registered,
        ),
        (
            "C_ChatInfo.GetRegisteredAddonMessagePrefixes",
            registered_prefixes,
        ),
        ("C_ChatInfo.InChatMessagingLockdown", messaging_lockdown),
        ("C_ChatInfo.SendAddonMessage", send),
        ("C_ChatInfo.SendChatMessage", send_chat),
        ("C_ChatInfo.SendAddonMessageLogged", send),
    ]
}
