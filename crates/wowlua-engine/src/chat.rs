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
    /// Whether the player is in a guild (`joinGuild`).
    pub guild: bool,
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
            "GUILD" | "OFFICER" if !engine(vm).world.chat.guild => result::NOT_IN_GUILD,
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
    let args = vec![
        s(vm, text),
        s(vm, name),
        s(vm, ""),
        s(vm, ""),
        s(vm, name),
        s(vm, ""),
        Value::Num(0.0),
        Value::Num(0.0),
        s(vm, ""),
        Value::Num(0.0),
        Value::Num(line as f64),
        if sender.is_some() {
            let g = crate::engine(vm).units.list[crate::units::PLAYER.0]
                .guid
                .clone();
            vm.str(&g)
        } else {
            Value::Nil
        },
        Value::Num(0.0),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
        Value::Table(discord),
    ];
    crate::widgets::fire_event(vm, event, &args);
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

/// `joinGuild([on])`: the player is in a guild (or not).
pub fn join_guild_control(vm: &mut Vm, c: NativeCall) -> Ret {
    let on = !c.has(1) || c.arg(1).truthy();
    engine_mut(vm).world.chat.guild = on;
    Ok(vec![])
}

fn is_in_guild(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).world.chat.guild)])
}

/// `slash(line)`: runs `/cmd args` as the chat box does. Returns whether a
/// command took it.
pub fn slash_control(vm: &mut Vm, c: NativeCall) -> Ret {
    let line = vm.check_str(&c, 1)?;
    let line = String::from_utf8_lossy(vm.str_bytes(line)).into_owned();
    let line = line.trim();
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
    vm.call_from_engine(handler, &[msg, Value::Nil]);
    Ok(vec![Value::Bool(true)])
}

/// The functions with behaviour here.
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
        ("C_ChatInfo.SendAddonMessageLogged", send),
        ("IsInGuild", is_in_guild),
    ]
}
