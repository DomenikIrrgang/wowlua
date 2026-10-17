//! The chat channels the player is in (a world profile's `channels`,
//! recorded by `/stdtest snapshot`; `mock.setChannels`), as the client
//! lists them (recorded in Thunder Bluff): `GetChannelList` the joined
//! channels by number; the channel pane's list (`GetChannelDisplayInfo`)
//! a header for each category (Group, World, Custom, Discord; its count
//! only when it has channels) followed by its channels; `GetChannelName`
//! a channel's full name; General's local ID its number.
//!
//! Unverified: what `GetNumGroupChannels` counts (recorded 1 without a
//! group); collapsed headers; joining and leaving; a channel's info
//! without a recording (its name as its shortcut, IDs 0).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// A joined channel.
#[derive(Clone, Debug, Default)]
pub struct Channel {
    /// `General`.
    pub name: String,
    /// `General - Thunder Bluff`.
    pub full_name: String,
    /// `CHANNEL_CATEGORY_WORLD`, `…_CUSTOM`, `…_GROUP`.
    pub category: String,
    pub disabled: bool,
    /// `C_ChatInfo.GetChannelInfoFromIdentifier`'s: the shortcut (by
    /// default the name), the instance, the zone channel's ID, the
    /// permanent type (`Enum.PermanentChatChannelType`).
    pub shortcut: Option<String>,
    pub instance_id: f64,
    pub zone_channel_id: f64,
    pub channel_type: f64,
    /// `C_ChatInfo.IsChannelRegional`.
    pub regional: bool,
}

/// The channels, by number (the first is 1).
#[derive(Clone, Debug)]
pub struct Channels {
    pub joined: Vec<Channel>,
    pub group_channels: f64,
    /// `C_ChatInfo.IsRegionalServiceAvailable`.
    pub regional_service: bool,
}

impl Default for Channels {
    fn default() -> Self {
        Channels {
            joined: Vec::new(),
            group_channels: 1.0,
            regional_service: true,
        }
    }
}

/// The pane's categories, in order.
const CATEGORIES: &[&str] = &[
    "CHANNEL_CATEGORY_GROUP",
    "CHANNEL_CATEGORY_WORLD",
    "CHANNEL_CATEGORY_CUSTOM",
    "CHANNEL_CATEGORY_DISCORD",
];

/// A row of the channel pane.
enum Row {
    Header {
        category: &'static str,
        count: usize,
    },
    Channel {
        number: usize,
    },
}

fn rows(vm: &Vm) -> Vec<Row> {
    let joined = &engine(vm).world.character.channels.joined;
    let mut out = Vec::new();
    for &category in CATEGORIES {
        let numbers: Vec<usize> = joined
            .iter()
            .enumerate()
            .filter(|(_, c)| c.category == category)
            .map(|(i, _)| i + 1)
            .collect();
        out.push(Row::Header {
            category,
            count: numbers.len(),
        });
        out.extend(numbers.into_iter().map(|number| Row::Channel { number }));
    }
    out
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn get_channel_list(vm: &mut Vm, _c: NativeCall) -> Ret {
    let joined = engine(vm).world.character.channels.joined.clone();
    let mut out = Vec::new();
    for (i, c) in joined.iter().enumerate() {
        out.push(Value::Num(i as f64 + 1.0));
        out.push(vm.str(&c.name));
        out.push(Value::Bool(c.disabled));
    }
    Ok(out)
}

fn get_num_display_channels(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(rows(vm).len() as f64)])
}

/// `GetChannelDisplayInfo(i)`: name, header, collapsed, number, count,
/// active, category, channel type.
fn get_channel_display_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = num(c.arg(1)).filter(|i| *i >= 1.0) else {
        return Ok(vec![]);
    };
    let rows = rows(vm);
    let Some(row) = rows.get(i as usize - 1) else {
        return Ok(vec![]);
    };
    Ok(match *row {
        Row::Header { category, count } => {
            let title = match vm.get_global(category) {
                Value::Str(s) => Value::Str(s),
                _ => vm.str(""),
            };
            let category = vm.str(category);
            vec![
                title,
                Value::Bool(true),
                Value::Bool(false),
                Value::Nil,
                if count > 0 {
                    Value::Num(count as f64)
                } else {
                    Value::Nil
                },
                Value::Nil,
                category,
                Value::Num(0.0),
            ]
        }
        Row::Channel { number } => {
            let c = engine(vm).world.character.channels.joined[number - 1].clone();
            let name = vm.str(&c.name);
            let category = vm.str(&c.category);
            vec![
                name,
                Value::Bool(false),
                Value::Nil,
                Value::Num(number as f64),
                Value::Nil,
                Value::Bool(!c.disabled),
                category,
                Value::Num(1.0),
            ]
        }
    })
}

/// `GetChannelName(number or name)`: the number, the full name, 0, false;
/// `0, nil, 0, false` for none.
fn get_channel_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let joined = engine(vm).world.character.channels.joined.clone();
    let found = match vm.hidden(c.arg(1)) {
        Value::Num(n) if n >= 1.0 => joined.get(n as usize - 1).map(|ch| (n as usize, ch)),
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            joined
                .iter()
                .enumerate()
                .find(|(_, ch)| ch.name.eq_ignore_ascii_case(&name))
                .map(|(i, ch)| (i + 1, ch))
        }
        _ => None,
    };
    Ok(match found {
        Some((number, ch)) => vec![
            Value::Num(number as f64),
            vm.str(&ch.full_name),
            Value::Num(0.0),
            Value::Bool(false),
        ],
        None => vec![
            Value::Num(0.0),
            Value::Nil,
            Value::Num(0.0),
            Value::Bool(false),
        ],
    })
}

/// The joined channel `v` names: its number, or its name (any case).
fn channel_named(vm: &Vm, v: Value) -> Option<(usize, Channel)> {
    let joined = &engine(vm).world.character.channels.joined;
    let by_number = |n: f64| {
        (n >= 1.0)
            .then(|| joined.get(n as usize - 1).map(|c| (n as usize, c.clone())))
            .flatten()
    };
    match vm.hidden(v) {
        Value::Num(n) => by_number(n),
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            match name.parse::<f64>() {
                Ok(n) => by_number(n),
                Err(_) => joined
                    .iter()
                    .enumerate()
                    .find(|(_, c)| c.name.eq_ignore_ascii_case(&name))
                    .map(|(i, c)| (i + 1, c.clone())),
            }
        }
        _ => None,
    }
}

/// `C_ChatInfo.GetChannelInfoFromIdentifier(identifier)`: a joined
/// channel's info; nothing for another.
fn channel_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((number, ch)) = channel_named(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    let name = vm.str(&ch.name);
    let shortcut = vm.str(ch.shortcut.as_deref().unwrap_or(&ch.name));
    for (k, v) in [
        ("name", name),
        ("shortcut", shortcut),
        ("localID", Value::Num(number as f64)),
        ("instanceID", Value::Num(ch.instance_id)),
        ("zoneChannelID", Value::Num(ch.zone_channel_id)),
        ("channelType", Value::Num(ch.channel_type)),
    ] {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, v);
    }
    Ok(vec![Value::Table(t)])
}

fn is_channel_regional(vm: &mut Vm, c: NativeCall) -> Ret {
    let regional = channel_named(vm, c.arg(1)).is_some_and(|(_, ch)| ch.regional);
    Ok(vec![Value::Bool(regional)])
}

fn regional_service(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        engine(vm).world.character.channels.regional_service,
    )])
}

fn get_num_group_channels(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        engine(vm).world.character.channels.group_channels,
    )])
}

/// `C_ChatInfo.GetGeneralChannelLocalID()`: General's number, if joined.
fn general_local_id(vm: &mut Vm, _c: NativeCall) -> Ret {
    let joined = &engine(vm).world.character.channels.joined;
    Ok(vec![
        joined
            .iter()
            .position(|c| c.name == "General")
            .map_or(Value::Nil, |i| Value::Num(i as f64 + 1.0)),
    ])
}

/// `C_ChatInfo.GetGeneralChannelID()`: General's channel ID (recorded).
fn general_id(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1.0)])
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn text(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

/// `mock.setChannels({ joined = { { name, fullName, category, disabled,
/// shortcut, instanceID, zoneChannelID, channelType, regional }, … },
/// groupChannels, regionalService })`: the channels the player is in
/// (`CHANNEL_UI_UPDATE`).
pub fn control_set_channels(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("setChannels: a table expected"));
    };
    let mut joined = Vec::new();
    if let Value::Table(list) = field(vm, t, "joined") {
        let n = vm.heap.table(list).border();
        for i in 1..=n {
            let Value::Table(ch) = vm.heap.table(list).get(Value::Num(i as f64)) else {
                continue;
            };
            let name = field(vm, ch, "name");
            let Some(name) = text(vm, name) else {
                return Err(vm.error("setChannels: each channel's name expected"));
            };
            let full = field(vm, ch, "fullName");
            let category = field(vm, ch, "category");
            let shortcut = field(vm, ch, "shortcut");
            let number = |vm: &mut Vm, k: &str| num(field(vm, ch, k)).unwrap_or(0.0);
            joined.push(Channel {
                full_name: text(vm, full).unwrap_or_else(|| name.clone()),
                name,
                category: text(vm, category)
                    .unwrap_or_else(|| "CHANNEL_CATEGORY_CUSTOM".to_string()),
                disabled: field(vm, ch, "disabled").truthy(),
                shortcut: text(vm, shortcut),
                instance_id: number(vm, "instanceID"),
                zone_channel_id: number(vm, "zoneChannelID"),
                channel_type: number(vm, "channelType"),
                regional: field(vm, ch, "regional").truthy(),
            });
        }
    }
    let group = num(field(vm, t, "groupChannels"));
    let service = field(vm, t, "regionalService");
    let channels = &mut engine_mut(vm).world.character.channels;
    channels.joined = joined;
    if let Some(g) = group {
        channels.group_channels = g;
    }
    if let Value::Bool(b) = service {
        channels.regional_service = b;
    }
    widgets::fire_event(vm, "CHANNEL_UI_UPDATE", &[]);
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetChannelList", get_channel_list),
        ("GetNumDisplayChannels", get_num_display_channels),
        ("GetChannelDisplayInfo", get_channel_display_info),
        ("GetChannelName", get_channel_name),
        ("GetNumGroupChannels", get_num_group_channels),
        ("C_ChatInfo.GetGeneralChannelLocalID", general_local_id),
        ("C_ChatInfo.GetGeneralChannelID", general_id),
        ("C_ChatInfo.GetChannelInfoFromIdentifier", channel_info),
        ("C_ChatInfo.IsChannelRegional", is_channel_regional),
        ("C_ChatInfo.IsRegionalServiceAvailable", regional_service),
    ]
}
