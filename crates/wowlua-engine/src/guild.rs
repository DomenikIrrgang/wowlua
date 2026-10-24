//! The player's guild: its ranks and their rights, its roster, its bank's
//! money, its message of the day and info text, its professions, and the
//! guild as Blizzard's communities see it (a club of the guild type).
//!
//! The engine's camelot character is in a guild as the recorded one is
//! (the full probe, WoW Forever 1.60.1): its leader (rank index 0), in a
//! guild of five ranks and 27 members, the player the only one online
//! (`GetNumGuildMembers` 27, 1). The guild's name is the ingest's
//! placeholder "Guild", the other members are "Otherplayer Othername"
//! (the ingest's placeholders for other players), their GUIDs the
//! engine's own. Of the roster only the player's row and the second
//! member's were recorded (an offline member of the lowest rank, level 3,
//! 24 days and an hour away); the other 25 members, the ranks' names
//! (Blizzard's defaults for a new guild), the other ranks' rights and the
//! professions after the first two are estimates (unverified).
//!
//! The mock's `joinGuild(guild?)` puts the player in a guild (that one by
//! default, or one a test describes: its rank, ranks, members, texts,
//! bank money), `leaveGuild()` in none; `defineGuildMember`,
//! `updateGuildMember` and `removeGuildMember` change its roster.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, units};

/// How many rights a rank has (`C_GuildInfo.GuildControlGetRankFlags`:
/// 22 recorded, Blizzard's `GUILDCONTROL_OPTION1` to `22`).
pub const FLAGS: usize = 22;

/// The rights by their flag (1-based, as `GuildControlGetRankFlags`
/// numbers them; Blizzard_GuildControlUI's checkboxes' IDs and its
/// `GUILDCONTROL_OPTION<n>` names).
mod flag {
    pub const SPEAK: usize = 2;
    pub const OFFICER_LISTEN: usize = 3;
    pub const PROMOTE: usize = 5;
    pub const DEMOTE: usize = 6;
    pub const INVITE: usize = 7;
    pub const REMOVE: usize = 8;
    pub const SET_MOTD: usize = 9;
    pub const EDIT_PUBLIC_NOTE: usize = 10;
    pub const VIEW_OFFICER_NOTE: usize = 11;
    pub const EDIT_OFFICER_NOTE: usize = 12;
    pub const MODIFY_INFO: usize = 13;
    pub const BANK_REPAIR: usize = 15;
    pub const WITHDRAW_GOLD: usize = 16;
    pub const MODIFY_BANK_TABS: usize = 19;
    pub const OFFICER: usize = 22;
}

/// A rank: its name and rights (`flags[n - 1]` is flag `n`).
#[derive(Clone, Debug)]
pub struct Rank {
    pub name: String,
    pub flags: [bool; FLAGS],
}

impl Rank {
    fn with(name: &str, on: &[usize]) -> Rank {
        let mut flags = [false; FLAGS];
        for &f in on {
            flags[f - 1] = true;
        }
        Rank {
            name: name.into(),
            flags,
        }
    }
}

/// Another member of the guild.
#[derive(Clone, Debug)]
pub struct Member {
    /// The mock's handle for it.
    pub handle: u32,
    pub name: String,
    pub last_name: Option<String>,
    pub guid: String,
    /// The class's file (`WARRIOR`).
    pub class: String,
    pub race: String,
    pub level: f64,
    /// The rank's index (0 the leader).
    pub rank: usize,
    pub zone: String,
    pub note: String,
    pub officer_note: String,
    pub online: bool,
    /// 0 none, 1 away, 2 busy.
    pub status: f64,
    /// An offline member's time away: years, months, days, hours.
    pub last_online: [f64; 4],
    pub achievement_points: f64,
    pub achievement_rank: f64,
}

/// A profession's header in the guild's professions list
/// (`GetGuildTradeSkillInfo`).
#[derive(Clone, Debug)]
pub struct TradeSkill {
    pub skill: f64,
    pub icon: f64,
    pub name: String,
    pub players: f64,
    pub online: f64,
}

/// The player's guild.
#[derive(Clone, Debug)]
pub struct Guild {
    pub name: String,
    /// The player's rank index (0 the leader).
    pub rank: usize,
    pub ranks: Vec<Rank>,
    /// The other members, in the order they were defined.
    pub members: Vec<Member>,
    pub motd: String,
    pub info: String,
    /// The player's own notes and place in the achievement ranking.
    pub note: String,
    pub officer_note: String,
    pub achievement_rank: f64,
    /// The guild bank's money (copper) and bought tabs.
    pub bank_money: f64,
    pub bank_tabs: usize,
    /// The guild's club (`C_Club`): its ID and when the player joined it.
    pub club_id: f64,
    pub join_time: f64,
    pub trade_skills: Vec<TradeSkill>,
    /// The roster's selected row (`SetGuildRosterSelection`), 0 none.
    pub selection: usize,
    next_handle: u32,
}

/// The recorded club ID of the guild (`C_Club.GetGuildClubId`, the full
/// probe).
const CLUB_ID: f64 = 22684421.0;
/// The recorded `joinTime` (microseconds; `C_Club.GetSubscribedClubs`).
const JOIN_TIME: f64 = 1.789749175e15;
/// No limit on what the leader may take from the bank: 2^64 (recorded
/// `GetGuildBankWithdrawMoney` 1.844674407371e+19).
const UNLIMITED: f64 = 18446744073709551616.0;

impl Guild {
    /// A guild of the default ranks with the player in it, at `rank`, and
    /// no one else.
    pub fn new(rank: usize) -> Guild {
        Guild {
            name: "Guild".into(),
            rank,
            ranks: default_ranks(),
            members: Vec::new(),
            motd: String::new(),
            info: String::new(),
            note: String::new(),
            officer_note: String::new(),
            achievement_rank: 1.0,
            bank_money: 0.0,
            bank_tabs: 0,
            club_id: CLUB_ID,
            join_time: JOIN_TIME,
            trade_skills: Vec::new(),
            selection: 0,
            next_handle: 1,
        }
    }

    /// The recorded character's guild (the full probe): the player its
    /// leader among 27 members, none of the others online.
    pub fn recorded() -> Guild {
        let mut g = Guild::new(0);
        // The player last of the achievement ranking, the second member
        // 25th (recorded; all without points, -1).
        g.achievement_rank = 27.0;
        const CLASSES: &[(&str, &str)] = &[
            ("WARRIOR", "Orc"),
            ("HUNTER", "Troll"),
            ("ROGUE", "Undead"),
            ("PRIEST", "Troll"),
            ("SHAMAN", "Tauren"),
            ("MAGE", "Undead"),
            ("WARLOCK", "Orc"),
            ("DRUID", "Tauren"),
        ];
        for i in 0..26usize {
            let (class, race) = CLASSES[i % CLASSES.len()];
            let mut m = g.member("Otherplayer", Some("Othername"));
            m.class = class.into();
            m.race = race.into();
            // The second row recorded: level 3, 24 days and an hour away,
            // 25th by achievements. The others' levels and absences
            // estimated (unverified), their achievement places the rest of
            // 1 to 26.
            if i == 0 {
                m.level = 3.0;
                m.last_online = [0.0, 0.0, 24.0, 1.0];
                m.achievement_rank = 25.0;
            } else {
                m.level = (1 + (i * 7) % 20) as f64;
                m.last_online = [0.0, (i % 3) as f64, (i * 5 % 28) as f64, (i % 24) as f64];
                m.achievement_rank = if i == 1 { 26.0 } else { 26.0 - i as f64 };
            }
            g.members.push(m);
        }
        // The guild's professions, each its header, collapsed: Alchemy's
        // and Blacksmithing's recorded (5 players, none online; 4, one
        // online: the player), the other eight estimated by the classic
        // professions in alphabetical order (unverified: which ten, their
        // counts, the icons after the first two).
        let skills: &[(f64, f64, &str, f64, f64)] = &[
            (171.0, 136240.0, "Alchemy", 5.0, 0.0),
            (164.0, 136241.0, "Blacksmithing", 4.0, 1.0),
            (185.0, 133971.0, "Cooking", 3.0, 0.0),
            (333.0, 136244.0, "Enchanting", 2.0, 0.0),
            (202.0, 136243.0, "Engineering", 2.0, 0.0),
            (182.0, 136246.0, "Herbalism", 6.0, 0.0),
            (165.0, 136247.0, "Leatherworking", 3.0, 0.0),
            (186.0, 136248.0, "Mining", 5.0, 1.0),
            (393.0, 134366.0, "Skinning", 4.0, 0.0),
            (197.0, 136249.0, "Tailoring", 4.0, 0.0),
        ];
        g.trade_skills = skills
            .iter()
            .map(|&(skill, icon, name, players, online)| TradeSkill {
                skill,
                icon,
                name: name.into(),
                players,
                online,
            })
            .collect();
        g
    }

    /// A new member with defaults (offline, the lowest rank, a warrior),
    /// its handle and GUID its own.
    fn member(&mut self, name: &str, last_name: Option<&str>) -> Member {
        let handle = self.next_handle;
        self.next_handle += 1;
        Member {
            handle,
            name: name.into(),
            last_name: last_name.map(str::to_string),
            // The engine's own GUIDs for players (as `units`' spawns'),
            // in a range of their own.
            guid: format!("Player-4621-{:08X}", 0x0B00_0000 + handle),
            class: "WARRIOR".into(),
            race: "Orc".into(),
            level: 1.0,
            rank: self.ranks.len().saturating_sub(1),
            zone: String::new(),
            note: String::new(),
            officer_note: String::new(),
            online: false,
            status: 0.0,
            last_online: [0.0, 0.0, 1.0, 0.0],
            achievement_points: -1.0,
            achievement_rank: f64::from(handle),
        }
    }

    /// Whether the player's rank has right `n`.
    pub fn can(&self, n: usize) -> bool {
        self.ranks
            .get(self.rank)
            .is_some_and(|r| r.flags.get(n - 1).copied().unwrap_or(false))
    }

    /// Whether member `m`'s rank has right `n`.
    fn member_can(&self, m: &Member, n: usize) -> bool {
        self.ranks
            .get(m.rank)
            .is_some_and(|r| r.flags.get(n - 1).copied().unwrap_or(false))
    }

    fn rank_name(&self, index: usize) -> String {
        self.ranks
            .get(index)
            .map(|r| r.name.clone())
            .unwrap_or_default()
    }
}

/// The ranks of a new guild, Blizzard's default names (unverified: the
/// recorded names are scrubbed). The leader's rights recorded (all but
/// 14, Create Guild Event as camelot numbers it, and 18, Requires
/// Authenticator); the others' estimated: officers most rights but the
/// guild's info, its bank and its leadership, the rest guild chat alone
/// (unverified).
fn default_ranks() -> Vec<Rank> {
    let leader: Vec<usize> = (1..=FLAGS).filter(|f| *f != 14 && *f != 18).collect();
    vec![
        Rank::with("Guild Master", &leader),
        Rank::with(
            "Officer",
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 17, 20, 22],
        ),
        Rank::with("Veteran", &[1, 2]),
        Rank::with("Member", &[1, 2]),
        Rank::with("Initiate", &[1, 2]),
    ]
}

// ---------------------------------------------------------------- shared

fn guild(vm: &Vm) -> Option<&Guild> {
    engine(vm).world.chat.guild.as_ref()
}

fn guild_mut(vm: &mut Vm) -> Option<&mut Guild> {
    engine_mut(vm).world.chat.guild.as_mut()
}

/// Whether the player is in a guild.
pub fn in_guild(vm: &Vm) -> bool {
    guild(vm).is_some()
}

fn can(vm: &Vm, n: usize) -> bool {
    guild(vm).is_some_and(|g| g.can(n))
}

fn text(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn num(vm: &Vm, v: Value) -> Option<f64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n),
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).trim().parse().ok(),
        _ => None,
    }
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn set_str(vm: &mut Vm, t: TableRef, key: &str, s: &str) {
    let v = vm.str(s);
    set(vm, t, key, v);
}

fn list(vm: &mut Vm, values: Vec<Value>) -> Value {
    let t = vm.new_table();
    for (i, v) in values.into_iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num(i as f64 + 1.0), v);
    }
    Value::Table(t)
}

fn player_guid(vm: &Vm) -> String {
    engine(vm).units.list[units::PLAYER.0].guid.clone()
}

/// The player's name as the roster has it: on WoW Forever the first and
/// last name, else Name-Realm (as `chat`'s and `social`'s).
fn player_full_name(vm: &Vm) -> String {
    let p = &engine(vm).units.list[units::PLAYER.0];
    match &p.last_name {
        Some(last) => format!("{} {last}", p.name),
        None => format!(
            "{}-{}",
            p.name,
            p.realm
                .clone()
                .unwrap_or_else(|| "Realm".into())
                .replace([' ', '-'], "")
        ),
    }
}

fn member_full_name(m: &Member) -> String {
    match &m.last_name {
        Some(last) => format!("{} {last}", m.name),
        None => m.name.clone(),
    }
}

/// A roster row: the player, or the other member at that place in
/// `members`.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Player,
    Member(usize),
}

/// The roster in its order: the player first, the other members online,
/// then those offline, each in the order they joined (unverified: the
/// client's own sort; recorded the player first and an offline member
/// second).
fn roster(g: &Guild) -> Vec<Row> {
    let mut rows = vec![Row::Player];
    rows.extend(
        g.members
            .iter()
            .enumerate()
            .filter(|(_, m)| m.online)
            .map(|(i, _)| Row::Member(i)),
    );
    rows.extend(
        g.members
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.online)
            .map(|(i, _)| Row::Member(i)),
    );
    rows
}

/// The row at a roster index (1-based).
fn row_at(vm: &Vm, index: Option<f64>) -> Option<Row> {
    let g = guild(vm)?;
    let i = index?.trunc();
    if i < 1.0 {
        return None;
    }
    roster(g).get(i as usize - 1).copied()
}

fn row_rank(g: &Guild, row: Row) -> usize {
    match row {
        Row::Player => g.rank,
        Row::Member(i) => g.members[i].rank,
    }
}

/// The member a GUID names: the player's own, another's.
fn row_by_guid(vm: &Vm, guid: &str) -> Option<Row> {
    let g = guild(vm)?;
    if guid == player_guid(vm) {
        return Some(Row::Player);
    }
    g.members
        .iter()
        .position(|m| m.guid == guid)
        .map(Row::Member)
}

/// The member a name names (the roster's full name, or a first name alone
/// for a member without a last name; case-insensitive).
fn row_by_name(vm: &Vm, name: &str) -> Option<Row> {
    let g = guild(vm)?;
    if name.eq_ignore_ascii_case(&player_full_name(vm)) {
        return Some(Row::Player);
    }
    g.members
        .iter()
        .position(|m| name.eq_ignore_ascii_case(&member_full_name(m)))
        .map(Row::Member)
}

/// The member a unit is: the player, another unit whose GUID is a
/// member's.
fn row_by_unit(vm: &Vm, v: Value) -> Option<Row> {
    let token = text(vm, v)?;
    let id = units::resolve(&engine(vm).units, &token)?;
    let guid = engine(vm).units.list[id.0].guid.clone();
    row_by_guid(vm, &guid)
}

/// Fires an event a round trip later (the server's answer).
fn later_event(vm: &mut Vm, event: &'static str, args: Vec<Value>) {
    let mut upvals = vec![vm.str(event)];
    upvals.extend(args);
    let native = vm.native_with(event, fire_upvals, upvals);
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, native);
}

fn fire_upvals(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Str(s)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    let event = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    let args: Vec<Value> = c.upvals[1..].to_vec();
    crate::widgets::fire_event(vm, &event, &args);
    Ok(vec![])
}

/// The roster changed: `GUILD_ROSTER_UPDATE(false)` a round trip later
/// (unverified: the client's argument, whether it may ask again).
fn roster_changed(vm: &mut Vm) {
    later_event(vm, "GUILD_ROSTER_UPDATE", vec![Value::Bool(false)]);
}

// ---------------------------------------------------------------- membership

fn is_in_guild(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(in_guild(vm))])
}

/// `GetGuildInfo(unit)`: the guild's name, the unit's rank's name and
/// index, and the realm (nil for the player's own); for a unit in no
/// guild nil, nil, 0, nil (the full probe: "player" the guild and rank 0;
/// "x", -1, 0, 1, 2 so). Another unit that is a member: its rank
/// (unverified).
fn get_guild_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let row = row_by_unit(vm, c.arg(1));
    let (Some(g), Some(row)) = (guild(vm), row) else {
        return Ok(vec![Value::Nil, Value::Nil, Value::Num(0.0), Value::Nil]);
    };
    let rank = row_rank(g, row);
    let (name, rank_name) = (g.name.clone(), g.rank_name(rank));
    Ok(vec![
        vm.str(&name),
        vm.str(&rank_name),
        Value::Num(rank as f64),
        Value::Nil,
    ])
}

/// `GetNumGuildMembers()`: the members and those online (the full probe:
/// 27 and 1, two values).
fn get_num_guild_members(vm: &mut Vm, _c: NativeCall) -> Ret {
    let (total, online) = guild(vm).map_or((0, 0), |g| {
        (
            g.members.len() + 1,
            g.members.iter().filter(|m| m.online).count() + 1,
        )
    });
    Ok(vec![Value::Num(total as f64), Value::Num(online as f64)])
}

/// `SetGuildRosterSelection(index)`: the roster's row selected, 0 (or
/// one past the roster) none (unverified: no event).
fn set_guild_roster_selection(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = match vm.hidden(c.arg(1)) {
        Value::Num(n) if n >= 1.0 => n as usize,
        _ => 0,
    };
    if let Some(g) = guild_mut(vm) {
        g.selection = if index <= g.members.len() + 1 {
            index
        } else {
            0
        };
    }
    Ok(vec![])
}

/// `GetGuildRosterSelection()`: the row selected, 0 none.
fn get_guild_roster_selection(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = guild(vm).map_or(0, |g| g.selection);
    Ok(vec![Value::Num(n as f64)])
}

/// `GetGuildRosterInfo(index)`: the member's name, rank name and index,
/// level, class name, zone, public and officer notes, online, status,
/// class file, achievement points and place, mobile, can be resurrected
/// by scroll, guild standing, GUID (warcraft.wiki.gg's order). Recorded
/// (the full probe): the player's row (rank 0, online, no achievement
/// points -1, 27th, standing 3) and an offline member's (rank 4, its
/// strings given); for an index no member has (-1, 0) nil, nil, 0, 0, nil
/// × 5, a status text, nil, 0, 0, false, false, 0, nil. The officer note
/// empty for a rank that may not see it (warcraft.wiki.gg; unverified).
fn get_guild_roster_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = num(vm, c.arg(1));
    let Some(row) = row_at(vm, index) else {
        let (nil, zero) = (Value::Nil, Value::Num(0.0));
        let status = vm.str("");
        return Ok(vec![
            nil,
            nil,
            zero,
            zero,
            nil,
            nil,
            nil,
            nil,
            nil,
            status,
            nil,
            zero,
            zero,
            Value::Bool(false),
            Value::Bool(false),
            zero,
            nil,
        ]);
    };
    let g = guild(vm).cloned().expect("in a guild");
    let see_officer = g.can(flag::VIEW_OFFICER_NOTE);
    let (name, rank, level, class, zone, note, officer, online, status, ach, place, guid) =
        match row {
            Row::Player => {
                let p = &engine(vm).units.list[units::PLAYER.0];
                (
                    player_full_name(vm),
                    g.rank,
                    p.level,
                    p.class.clone(),
                    engine(vm).world.character.position.zone.clone(),
                    g.note.clone(),
                    g.officer_note.clone(),
                    true,
                    0.0,
                    -1.0,
                    g.achievement_rank,
                    p.guid.clone(),
                )
            }
            Row::Member(i) => {
                let m = &g.members[i];
                (
                    member_full_name(m),
                    m.rank,
                    m.level,
                    m.class.clone(),
                    m.zone.clone(),
                    m.note.clone(),
                    m.officer_note.clone(),
                    m.online,
                    m.status,
                    m.achievement_points,
                    m.achievement_rank,
                    m.guid.clone(),
                )
            }
        };
    let class_name = units::class_name_of(&class).unwrap_or("").to_string();
    let officer = if see_officer { officer } else { String::new() };
    Ok(vec![
        vm.str(&name),
        vm.str(&g.rank_name(rank)),
        Value::Num(rank as f64),
        Value::Num(level),
        vm.str(&class_name),
        vm.str(&zone),
        vm.str(&note),
        vm.str(&officer),
        Value::Bool(online),
        Value::Num(status),
        vm.str(&class),
        Value::Num(ach),
        Value::Num(place),
        Value::Bool(false),
        Value::Bool(false),
        // Standing 3 for every member (recorded; no guild reputation on
        // Forever).
        Value::Num(3.0),
        vm.str(&guid),
    ])
}

/// `GetGuildRosterLastOnline(index)`: an offline member's years, months,
/// days and hours away; an online member's (the player's) and no
/// member's four nils (the full probe: 2, an offline member, 0, 0, 24, 1;
/// 1, -1, 0 nils).
fn get_guild_roster_last_online(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = num(vm, c.arg(1));
    let away = match (guild(vm), row_at(vm, index)) {
        (Some(g), Some(Row::Member(i))) if !g.members[i].online => Some(g.members[i].last_online),
        _ => None,
    };
    Ok(match away {
        Some(t) => t.iter().map(|&n| Value::Num(n)).collect(),
        None => vec![Value::Nil; 4],
    })
}

/// `IsGuildLeader()`: the player's rank the first (the full probe: true
/// for the leader).
fn is_guild_leader(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(guild(vm).is_some_and(|g| g.rank == 0))])
}

/// `IsGuildMember(unit)`: the unit a member (the full probe: "player"
/// true; "x" and numbers false; another member's unit unverified).
fn is_guild_member(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(row_by_unit(vm, c.arg(1)).is_some())])
}

/// `UnitIsInMyGuild(unit)`: the unit a member of the player's guild (the
/// full probe: "player" true, others and a GUID false; another member's
/// unit unverified).
fn unit_is_in_my_guild(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(row_by_unit(vm, c.arg(1)).is_some())])
}

/// `IsPlayerInGuildFromGUID(guid)`: the GUID a member's (the full probe:
/// the player's own true, others false).
fn is_player_in_guild_from_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    let guid = text(vm, c.arg(1)).unwrap_or_default();
    Ok(vec![Value::Bool(row_by_guid(vm, &guid).is_some())])
}

/// `C_GuildInfo.MemberExistsByName(name)`: a member by its roster name
/// (the full probe: the player's first name alone false, the roster
/// naming it with its last name; unverified otherwise).
fn member_exists_by_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = text(vm, c.arg(1)).unwrap_or_default();
    Ok(vec![Value::Bool(row_by_name(vm, &name).is_some())])
}

/// `C_GuildInfo.GetGuildRankOrder(guid)`: a member's rank order (index +
/// 1); for a GUID no member has, the player's own (the full probe: "",
/// "x", a creature's GUID and the player's all 1, the leader's order;
/// unverified: whether another member's GUID gives theirs). Outside a
/// guild 0 (unverified).
fn get_guild_rank_order(vm: &mut Vm, c: NativeCall) -> Ret {
    let guid = text(vm, c.arg(1)).unwrap_or_default();
    let row = row_by_guid(vm, &guid).unwrap_or(Row::Player);
    let order = guild(vm).map_or(0.0, |g| row_rank(g, row) as f64 + 1.0);
    Ok(vec![Value::Num(order)])
}

// ---------------------------------------------------------------- ranks

/// A rank order argument as the client reads it (`1.5` is 1).
fn order_arg(vm: &Vm, v: Value) -> Option<i64> {
    num(vm, v).map(|n| n.trunc() as i64)
}

/// `GuildControlGetNumRanks()`: the guild's ranks (the full probe: 5);
/// none outside one (unverified).
fn guild_control_get_num_ranks(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = guild(vm).map_or(0, |g| g.ranks.len());
    Ok(vec![Value::Num(n as f64)])
}

/// `GuildControlGetRankName(rankOrder)`: the rank's name at its order
/// (index + 1); "" for no rank there (the full probe: strings for -1, 0,
/// 1 and 2, their texts scrubbed; unverified: the empty one).
fn guild_control_get_rank_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let order = order_arg(vm, c.arg(1)).unwrap_or(0);
    let name = guild(vm)
        .filter(|_| order >= 1)
        .map(|g| g.rank_name(order as usize - 1))
        .unwrap_or_default();
    Ok(vec![vm.str(&name)])
}

/// `C_GuildInfo.GuildControlGetRankFlags(rankOrder)`: the rank's 22
/// rights; for order 0 an empty list, for an order past the last rank 22
/// falses (the full probe: 0 {}, 1 and 1.5 the leader's, 2147483647 all
/// false). Outside a guild an empty list (unverified).
fn guild_control_get_rank_flags(vm: &mut Vm, c: NativeCall) -> Ret {
    let order = order_arg(vm, c.arg(1)).unwrap_or(0);
    let flags: Option<[bool; FLAGS]> = match guild(vm) {
        Some(_) if order < 1 => None,
        Some(g) => Some(
            g.ranks
                .get(order as usize - 1)
                .map_or([false; FLAGS], |r| r.flags),
        ),
        None => None,
    };
    let values = flags.map_or_else(Vec::new, |f| f.iter().map(|&b| Value::Bool(b)).collect());
    Ok(vec![list(vm, values)])
}

/// `GuildControlGetAllowedShifts(rankOrder)`: whether the rank may move
/// up and down: not above the second (the leader's stays first), not
/// below the last; the leader's rank and orders outside the ranks
/// "invalid rank order" (the full probe: 2 false, true; 1, 0, -1 and
/// strings an error, (1, 1)'s "invalid rank order"; unverified: other
/// orders, the strings' message).
fn guild_control_get_allowed_shifts(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = guild(vm).map_or(0, |g| g.ranks.len()) as i64;
    match order_arg(vm, c.arg(1)) {
        Some(order) if order >= 2 && order <= n => {
            Ok(vec![Value::Bool(order > 2), Value::Bool(order < n)])
        }
        _ => Err(vm.error("invalid rank order")),
    }
}

/// `GetNumMembersInRank(rankOrder)`: the members at that rank (the full
/// probe: 1 the leader alone, 2 none, -1 and 0 none).
fn get_num_members_in_rank(vm: &mut Vm, c: NativeCall) -> Ret {
    let order = order_arg(vm, c.arg(1)).unwrap_or(0);
    let n = match guild(vm) {
        Some(g) if order >= 1 => roster(g)
            .into_iter()
            .filter(|&r| row_rank(g, r) as i64 == order - 1)
            .count(),
        _ => 0,
    };
    Ok(vec![Value::Num(n as f64)])
}

/// Whether the player may set the member's rank to `to` (an index): the
/// member below the player's rank and `to` below it too, with the right
/// to promote (up) or demote (down).
fn may_assign(g: &Guild, row: Row, to: usize) -> bool {
    let from = row_rank(g, row);
    if row == Row::Player || from <= g.rank || to <= g.rank || to >= g.ranks.len() {
        return false;
    }
    (to < from && g.can(flag::PROMOTE)) || (to > from && g.can(flag::DEMOTE))
}

/// `GetPromotionRank(index)`: the rank index the member at that roster
/// place would be promoted to, nil when the player may not (the full
/// probe: 2, an Initiate at index 4, 3; 1, the leader itself, and -1, 0
/// nil; unverified otherwise).
fn get_promotion_rank(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = num(vm, c.arg(1));
    let rank = match (guild(vm), row_at(vm, index)) {
        (Some(g), Some(row)) => {
            let from = row_rank(g, row);
            (from > 0 && may_assign(g, row, from - 1)).then(|| from - 1)
        }
        _ => None,
    };
    Ok(vec![rank.map_or(Value::Nil, |r| Value::Num(r as f64))])
}

/// `GetDemotionRank(index)`: the rank index a demotion would give, nil
/// when the player may not (as `GetPromotionRank`; unverified).
fn get_demotion_rank(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = num(vm, c.arg(1));
    let rank = match (guild(vm), row_at(vm, index)) {
        (Some(g), Some(row)) => {
            let from = row_rank(g, row);
            may_assign(g, row, from + 1).then_some(from + 1)
        }
        _ => None,
    };
    Ok(vec![rank.map_or(Value::Nil, |r| Value::Num(r as f64))])
}

/// `IsGuildRankAssignmentAllowed(index, rankOrder)`: a member at that
/// roster place, and the player one who may change ranks (the full
/// probe: (1, 1) true, the leader asking of itself; unverified
/// otherwise).
fn is_guild_rank_assignment_allowed(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = num(vm, c.arg(1));
    let row = row_at(vm, index);
    Ok(vec![Value::Bool(row.is_some() && rank_changer(vm))])
}

/// `C_GuildInfo.IsGuildRankAssignmentAllowed(guid, rankOrder)`: as the
/// global by the member's GUID (the full probe: the player's own GUID
/// true for any order, "", "x" and a creature's false; unverified
/// otherwise).
fn c_is_guild_rank_assignment_allowed(vm: &mut Vm, c: NativeCall) -> Ret {
    let guid = text(vm, c.arg(1)).unwrap_or_default();
    let row = row_by_guid(vm, &guid);
    Ok(vec![Value::Bool(row.is_some() && rank_changer(vm))])
}

/// The player may promote or demote.
fn rank_changer(vm: &Vm) -> bool {
    can(vm, flag::PROMOTE) || can(vm, flag::DEMOTE)
}

// ---------------------------------------------------------------- rights

macro_rules! rights {
    ($($name:ident: $flag:expr;)*) => {
        $(
            fn $name(vm: &mut Vm, _c: NativeCall) -> Ret {
                Ok(vec![Value::Bool(can(vm, $flag))])
            }
        )*
    };
}

// Each the player's rank's right (the full probe: each true for the
// leader, whose rank has them; outside a guild false, unverified).
rights! {
    can_speak: flag::SPEAK;
    can_invite: flag::INVITE;
    can_promote: flag::PROMOTE;
    can_demote: flag::DEMOTE;
    can_remove: flag::REMOVE;
    can_edit_motd: flag::SET_MOTD;
    can_edit_public_note: flag::EDIT_PUBLIC_NOTE;
    can_view_officer_note: flag::VIEW_OFFICER_NOTE;
    can_edit_officer_note: flag::EDIT_OFFICER_NOTE;
    can_edit_info: flag::MODIFY_INFO;
    can_bank_repair: flag::BANK_REPAIR;
    can_withdraw_money: flag::WITHDRAW_GOLD;
    can_edit_bank_tabs: flag::MODIFY_BANK_TABS;
    is_officer: flag::OFFICER;
}

/// `CanEditGuildTabInfo(tab)`: a tab (1 on) the rank may modify; for 0
/// and below nothing (the full probe: -1 and 0 no values, 1 and 2 true
/// with no tab bought; unverified past them).
fn can_edit_guild_tab_info(vm: &mut Vm, c: NativeCall) -> Ret {
    match num(vm, c.arg(1)) {
        Some(tab) if tab >= 1.0 => Ok(vec![Value::Bool(can(vm, flag::MODIFY_BANK_TABS))]),
        _ => Ok(vec![]),
    }
}

/// `CanSignPetition()`: true (the full probe, in a guild; unverified
/// outside one and with a petition offered).
fn can_sign_petition(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// `C_GuildInfo.GetPreferredPlaySettingsFeatures()` (the full probe, in a
/// guild: true, true) and `C_GuildInfo.RequestPreferredPlaySettings()`
/// (true): in a guild; outside one false (unverified).
fn play_settings_features(vm: &mut Vm, _c: NativeCall) -> Ret {
    let on = in_guild(vm);
    Ok(vec![Value::Bool(on), Value::Bool(on)])
}

fn request_play_settings(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(in_guild(vm))])
}

// ---------------------------------------------------------------- texts

/// The message of the day (`GetGuildRosterMOTD`, `C_GuildInfo.GetMOTD`;
/// recorded a string, its text scrubbed): "" outside a guild.
fn get_motd(vm: &mut Vm, _c: NativeCall) -> Ret {
    let motd = guild(vm).map(|g| g.motd.clone()).unwrap_or_default();
    Ok(vec![vm.str(&motd)])
}

/// `C_GuildInfo.GetMOTD()` and `GetGuildRosterMOTD()` (Blizzard's
/// deprecated guild script makes the second the first): as [`get_motd`],
/// but blocked for addons in combat lockdown, nothing answered (decision
/// T265; found in game: `ADDON_ACTION_BLOCKED` from SuuUI's guild window
/// opened in a fight, `UNKNOWN()` through `pcall`).
fn get_roster_motd(vm: &mut Vm, c: NativeCall) -> Ret {
    if crate::protection::refuses_in_combat(vm) {
        return Ok(vec![]);
    }
    get_motd(vm, c)
}

/// The guild's info text (`GetGuildInfoText`, `C_GuildInfo.GetInfoText`;
/// recorded a string, scrubbed).
fn get_info_text(vm: &mut Vm, _c: NativeCall) -> Ret {
    let info = guild(vm).map(|g| g.info.clone()).unwrap_or_default();
    Ok(vec![vm.str(&info)])
}

/// `C_GuildInfo.SetMOTD(motd)`, `GuildSetMOTD(motd)`: the message of the
/// day, told (`GUILD_MOTD(motd)`) a round trip later, for a rank that may
/// set it (unverified: the client's events).
fn set_motd(vm: &mut Vm, c: NativeCall) -> Ret {
    let motd = text(vm, c.arg(1)).unwrap_or_default();
    if !can(vm, flag::SET_MOTD) {
        return Ok(vec![]);
    }
    if let Some(g) = guild_mut(vm) {
        g.motd = motd.clone();
    }
    let v = vm.str(&motd);
    later_event(vm, "GUILD_MOTD", vec![v]);
    Ok(vec![])
}

/// `C_GuildInfo.SetInfoText(text)`, `SetGuildInfoText(text)`: the info
/// text, for a rank that may change it (`GUILD_TEXT_UPDATE`? unverified;
/// the roster's update told).
fn set_info_text(vm: &mut Vm, c: NativeCall) -> Ret {
    let info = text(vm, c.arg(1)).unwrap_or_default();
    if !can(vm, flag::MODIFY_INFO) {
        return Ok(vec![]);
    }
    if let Some(g) = guild_mut(vm) {
        g.info = info;
    }
    roster_changed(vm);
    Ok(vec![])
}

/// `C_GuildInfo.SetNote(guid, note, isPublic)`: a member's public or
/// officer note, for a rank that may edit it (unverified: the client's
/// events).
fn set_note(vm: &mut Vm, c: NativeCall) -> Ret {
    let guid = text(vm, c.arg(1)).unwrap_or_default();
    let note = text(vm, c.arg(2)).unwrap_or_default();
    let public = c.arg(3).truthy();
    let right = if public {
        flag::EDIT_PUBLIC_NOTE
    } else {
        flag::EDIT_OFFICER_NOTE
    };
    let Some(row) = row_by_guid(vm, &guid) else {
        return Ok(vec![]);
    };
    if !can(vm, right) {
        return Ok(vec![]);
    }
    if let Some(g) = guild_mut(vm) {
        let target = match row {
            Row::Player => (&mut g.note, &mut g.officer_note),
            Row::Member(i) => {
                let m = &mut g.members[i];
                (&mut m.note, &mut m.officer_note)
            }
        };
        if public {
            *target.0 = note;
        } else {
            *target.1 = note;
        }
    }
    roster_changed(vm);
    Ok(vec![])
}

// ---------------------------------------------------------------- roster actions

/// A member's rank set (`may_assign`), the roster told.
fn assign(vm: &mut Vm, row: Option<Row>, to: impl Fn(usize) -> Option<usize>) {
    let Some(row) = row else { return };
    let Some(g) = guild_mut(vm) else { return };
    let Some(to) = to(row_rank(g, row)) else {
        return;
    };
    if !may_assign(g, row, to) {
        return;
    }
    if let Row::Member(i) = row {
        g.members[i].rank = to;
    }
    roster_changed(vm);
}

/// `GuildPromote(name)`, `C_GuildInfo.Promote(name)`: the member one rank
/// up (unverified: the client's system message and events).
fn promote(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = text(vm, c.arg(1)).unwrap_or_default();
    let row = row_by_name(vm, &name);
    assign(vm, row, |r| r.checked_sub(1));
    Ok(vec![])
}

/// `GuildDemote(name)`, `C_GuildInfo.Demote(name)`: one rank down.
fn demote(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = text(vm, c.arg(1)).unwrap_or_default();
    let row = row_by_name(vm, &name);
    assign(vm, row, |r| Some(r + 1));
    Ok(vec![])
}

/// `SetGuildMemberRank(index, rankOrder)`: the member at that roster
/// place to that rank.
fn set_guild_member_rank(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = num(vm, c.arg(1));
    let order = order_arg(vm, c.arg(2)).unwrap_or(0);
    let row = row_at(vm, index);
    if order >= 1 {
        assign(vm, row, |_| Some(order as usize - 1));
    }
    Ok(vec![])
}

/// A member removed (by a rank that may remove, below the player's).
fn remove_row(vm: &mut Vm, row: Option<Row>) {
    let Some(Row::Member(i)) = row else { return };
    let Some(g) = guild_mut(vm) else { return };
    if !g.can(flag::REMOVE) || g.members[i].rank <= g.rank {
        return;
    }
    g.members.remove(i);
    roster_changed(vm);
}

/// `GuildUninvite(name)`, `C_GuildInfo.Uninvite(name)`.
fn uninvite(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = text(vm, c.arg(1)).unwrap_or_default();
    let row = row_by_name(vm, &name);
    remove_row(vm, row);
    Ok(vec![])
}

/// `C_GuildInfo.RemoveFromGuild(guid)`.
fn remove_from_guild(vm: &mut Vm, c: NativeCall) -> Ret {
    let guid = text(vm, c.arg(1)).unwrap_or_default();
    let row = row_by_guid(vm, &guid);
    remove_row(vm, row);
    Ok(vec![])
}

/// `GuildSetLeader(name)`, `C_GuildInfo.SetLeader(name)`: the leader
/// hands the guild to a member, taking its rank (unverified).
fn set_leader(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = text(vm, c.arg(1)).unwrap_or_default();
    let Some(Row::Member(i)) = row_by_name(vm, &name) else {
        return Ok(vec![]);
    };
    let Some(g) = guild_mut(vm) else {
        return Ok(vec![]);
    };
    if g.rank != 0 {
        return Ok(vec![]);
    }
    g.rank = g.members[i].rank.max(1);
    g.members[i].rank = 0;
    roster_changed(vm);
    player_guild_changed(vm);
    Ok(vec![])
}

/// `GuildLeave()`, `C_GuildInfo.Leave()`: the player leaves (the leader
/// must hand the guild on first; unverified: the client's refusal).
fn leave(vm: &mut Vm, _c: NativeCall) -> Ret {
    if guild(vm).is_some_and(|g| g.rank != 0 || g.members.is_empty()) {
        engine_mut(vm).world.chat.guild = None;
        player_guild_changed(vm);
    }
    Ok(vec![])
}

/// `GuildDisband()`, `C_GuildInfo.Disband()`: the leader's guild gone
/// (unverified).
fn disband(vm: &mut Vm, _c: NativeCall) -> Ret {
    if guild(vm).is_some_and(|g| g.rank == 0) {
        engine_mut(vm).world.chat.guild = None;
        player_guild_changed(vm);
    }
    Ok(vec![])
}

/// `C_GuildInfo.GuildRoster()`: the roster asked for, told a round trip
/// later (`GUILD_ROSTER_UPDATE`; unverified: the client's throttle).
fn guild_roster(vm: &mut Vm, _c: NativeCall) -> Ret {
    if in_guild(vm) {
        roster_changed(vm);
    }
    Ok(vec![])
}

/// The player's guild changed: `PLAYER_GUILD_UPDATE("player")` (the
/// loading screen's, recorded; on joining and leaving unverified).
fn player_guild_changed(vm: &mut Vm) {
    let player = vm.str("player");
    later_event(vm, "PLAYER_GUILD_UPDATE", vec![player]);
}

// ---------------------------------------------------------------- bank

/// `GetGuildBankMoney()`: the bank's money (the full probe: 0).
fn get_guild_bank_money(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(guild(vm).map_or(0.0, |g| g.bank_money))])
}

/// `GetNumGuildBankTabs()`: the tabs bought (the full probe: 0).
fn get_num_guild_bank_tabs(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        guild(vm).map_or(0.0, |g| g.bank_tabs as f64),
    )])
}

/// `GetGuildBankTabCost()`: the next tab's price: 100 gold for the first
/// (the full probe: 1000000 with none bought), then 250, 500, 1000, 2500
/// and 5000 gold, none past the sixth (warcraft.wiki.gg's guild bank
/// prices; unverified past the first). Outside a guild 0 (unverified).
fn get_guild_bank_tab_cost(vm: &mut Vm, _c: NativeCall) -> Ret {
    const GOLD: [f64; 6] = [100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0];
    let cost = guild(vm)
        .and_then(|g| GOLD.get(g.bank_tabs))
        .map_or(0.0, |gold| gold * 10000.0);
    Ok(vec![Value::Num(cost)])
}

/// `GetGuildBankWithdrawMoney()`: what the player may take today: the
/// leader without limit, 2^64 (the full probe); any other rank its bank's
/// money if it may withdraw, else none (unverified: the ranks' daily
/// limits, not kept).
fn get_guild_bank_withdraw_money(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = match guild(vm) {
        Some(g) if g.rank == 0 => UNLIMITED,
        Some(g) if g.can(flag::WITHDRAW_GOLD) => g.bank_money,
        _ => 0.0,
    };
    Ok(vec![Value::Num(n)])
}

// ---------------------------------------------------------------- professions

/// `GetNumGuildTradeSkill()`: the professions list's rows, each a
/// collapsed header (the full probe: 10).
fn get_num_guild_trade_skill(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = guild(vm).map_or(0, |g| g.trade_skills.len());
    Ok(vec![Value::Num(n as f64)])
}

/// `GetGuildTradeSkillInfo(index)`: a header's skill line, collapsed,
/// icon, name, members online, visible (those online), with it, seven
/// nils, false and 0; nothing for no row (the full probe: 1 Alchemy, 2
/// Blacksmithing; -1, 0 nothing).
fn get_guild_trade_skill_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = num(vm, c.arg(1)).map_or(0.0, f64::trunc);
    let skill = guild(vm)
        .filter(|_| index >= 1.0)
        .and_then(|g| g.trade_skills.get(index as usize - 1).cloned());
    let Some(s) = skill else {
        return Ok(vec![]);
    };
    let mut out = vec![
        Value::Num(s.skill),
        Value::Bool(true),
        Value::Num(s.icon),
        vm.str(&s.name),
        Value::Num(s.online),
        Value::Num(s.online),
        Value::Num(s.players),
    ];
    out.extend([Value::Nil; 7]);
    out.extend([Value::Bool(false), Value::Num(0.0)]);
    Ok(out)
}

// ---------------------------------------------------------------- calendar

/// `C_Calendar.GetDefaultGuildFilter()`: the levels from and to the
/// maximum, 60, and the guild's lowest rank (the full probe, in a guild
/// of five ranks: 60, 60, 5); outside a guild rank 0 (unverified).
fn get_default_guild_filter(vm: &mut Vm, _c: NativeCall) -> Ret {
    let rank = guild(vm).map_or(0, |g| g.ranks.len());
    let t = vm.new_table();
    set(vm, t, "minLevel", Value::Num(60.0));
    set(vm, t, "maxLevel", Value::Num(60.0));
    set(vm, t, "rank", Value::Num(rank as f64));
    Ok(vec![Value::Table(t)])
}

// ---------------------------------------------------------------- the club

/// The guild's club ID when `v` is it.
fn guild_club(vm: &Vm, v: Value) -> Option<f64> {
    let id = guild(vm)?.club_id;
    (num(vm, v) == Some(id)).then_some(id)
}

/// `C_Club.GetGuildClubId()`: the guild's club (the full probe:
/// 22684421); nil outside a guild (unverified).
fn get_guild_club_id(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        guild(vm).map_or(Value::Nil, |g| Value::Num(g.club_id)),
    ])
}

/// The guild as a club (`ClubInfo`): avatar 0, the message of the day as
/// its broadcast, its info as its description, type Guild (2), not
/// cross-faction, when the player joined, its members, its name, no
/// social queueing (recorded, `C_Club.GetSubscribedClubs`: these fields
/// and no others).
fn club_info(vm: &mut Vm) -> Value {
    let Some(g) = guild(vm).cloned() else {
        return Value::Nil;
    };
    let t = vm.new_table();
    set(vm, t, "avatarId", Value::Num(0.0));
    set_str(vm, t, "broadcast", &g.motd);
    set(vm, t, "clubId", Value::Num(g.club_id));
    set(vm, t, "clubType", Value::Num(2.0));
    set(vm, t, "crossFaction", Value::Bool(false));
    set_str(vm, t, "description", &g.info);
    set(vm, t, "joinTime", Value::Num(g.join_time));
    set(
        vm,
        t,
        "memberCount",
        Value::Num(g.members.len() as f64 + 1.0),
    );
    set_str(vm, t, "name", &g.name);
    set(vm, t, "socialQueueingEnabled", Value::Bool(false));
    Value::Table(t)
}

/// `C_Club.GetSubscribedClubs()`: the guild's club alone (recorded);
/// none outside a guild.
fn get_subscribed_clubs(vm: &mut Vm, _c: NativeCall) -> Ret {
    let info = club_info(vm);
    let values = if info.is_nil() { vec![] } else { vec![info] };
    Ok(vec![list(vm, values)])
}

/// `C_Club.GetClubInfo(clubId)`: the guild's club by its ID; nil for any
/// other (recorded "", "x", {} nil; the guild's by ID unverified).
fn get_club_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if guild_club(vm, c.arg(1)).is_none() {
        return Ok(vec![Value::Nil]);
    }
    Ok(vec![club_info(vm)])
}

/// A roster row's club member ID: the player 1, another member its
/// handle + 1 (unverified: the client's IDs).
fn member_id(g: &Guild, row: Row) -> f64 {
    match row {
        Row::Player => 1.0,
        Row::Member(i) => f64::from(g.members[i].handle) + 1.0,
    }
}

fn row_by_member_id(g: &Guild, id: f64) -> Option<Row> {
    if id == 1.0 {
        return Some(Row::Player);
    }
    g.members
        .iter()
        .position(|m| f64::from(m.handle) + 1.0 == id)
        .map(Row::Member)
}

/// `C_Club.GetClubMembers(clubId [, streamId])`: the guild's members'
/// IDs, in the roster's order; the officer stream's those whose rank
/// hears officer chat (unverified). Any other club none.
fn get_club_members(vm: &mut Vm, c: NativeCall) -> Ret {
    let stream = num(vm, c.arg(2));
    let ids: Vec<Value> = match (guild_club(vm, c.arg(1)), guild(vm)) {
        (Some(_), Some(g)) => roster(g)
            .into_iter()
            .filter(|&r| {
                stream != Some(2.0)
                    || match r {
                        Row::Player => g.can(flag::OFFICER_LISTEN),
                        Row::Member(i) => g.member_can(&g.members[i], flag::OFFICER_LISTEN),
                    }
            })
            .map(|r| Value::Num(member_id(g, r)))
            .collect(),
        _ => Vec::new(),
    };
    Ok(vec![list(vm, ids)])
}

/// A member as the club sees it (`ClubMemberInfo`): its ID, name, role
/// (the leader the owner, an officer's rank a moderator, else a member),
/// presence, GUID, notes (the officer note for a rank that sees it),
/// class and race IDs, level, zone, achievement points, its time away
/// when offline, its rank's name and order (unverified: the client's
/// fields for a guild's member, none recorded).
fn club_member_info(vm: &mut Vm, row: Row) -> Value {
    let Some(g) = guild(vm).cloned() else {
        return Value::Nil;
    };
    let t = vm.new_table();
    let id = member_id(&g, row);
    let (name, guid, class, race, level, zone, note, officer, online, status, points, away) =
        match row {
            Row::Player => {
                let p = &engine(vm).units.list[units::PLAYER.0];
                (
                    player_full_name(vm),
                    p.guid.clone(),
                    p.class.clone(),
                    p.race.clone(),
                    p.level,
                    engine(vm).world.character.position.zone.clone(),
                    g.note.clone(),
                    g.officer_note.clone(),
                    true,
                    0.0,
                    -1.0,
                    None,
                )
            }
            Row::Member(i) => {
                let m = &g.members[i];
                (
                    member_full_name(m),
                    m.guid.clone(),
                    m.class.clone(),
                    m.race.clone(),
                    m.level,
                    m.zone.clone(),
                    m.note.clone(),
                    m.officer_note.clone(),
                    m.online,
                    m.status,
                    m.achievement_points,
                    (!m.online).then_some(m.last_online),
                )
            }
        };
    let rank = row_rank(&g, row);
    let officer_rank = g
        .ranks
        .get(rank)
        .is_some_and(|r| r.flags[flag::OFFICER - 1]);
    let role = if rank == 0 {
        1.0
    } else if officer_rank {
        3.0
    } else {
        4.0
    };
    let presence = match (online, status) {
        (false, _) => 3.0,
        (true, 1.0) => 4.0,
        (true, 2.0) => 5.0,
        _ => 1.0,
    };
    set(vm, t, "isSelf", Value::Bool(row == Row::Player));
    set(vm, t, "memberId", Value::Num(id));
    set_str(vm, t, "name", &name);
    set(vm, t, "role", Value::Num(role));
    set(vm, t, "presence", Value::Num(presence));
    set(vm, t, "clubType", Value::Num(2.0));
    set_str(vm, t, "guid", &guid);
    set_str(vm, t, "memberNote", &note);
    if g.can(flag::VIEW_OFFICER_NOTE) {
        set_str(vm, t, "officerNote", &officer);
    }
    set(vm, t, "classID", Value::Num(units::class_id(&class)));
    if let (_, Some(race)) = units::race_file_and_id(&race) {
        set(vm, t, "race", Value::Num(race));
    }
    set(vm, t, "level", Value::Num(level));
    set_str(vm, t, "zone", &zone);
    set(vm, t, "achievementPoints", Value::Num(points));
    if let Some([y, mo, d, h]) = away {
        set(vm, t, "lastOnlineYear", Value::Num(y));
        set(vm, t, "lastOnlineMonth", Value::Num(mo));
        set(vm, t, "lastOnlineDay", Value::Num(d));
        set(vm, t, "lastOnlineHour", Value::Num(h));
    }
    set_str(vm, t, "guildRank", &g.rank_name(rank));
    set(vm, t, "guildRankOrder", Value::Num(rank as f64 + 1.0));
    set(vm, t, "isRemoteChat", Value::Bool(false));
    Value::Table(t)
}

/// `C_Club.GetMemberInfo(clubId, memberId)`: a guild member's
/// (`club_member_info`); nil for any other (recorded "x" and "" nil).
fn get_member_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(vm, c.arg(2));
    let row = match (guild_club(vm, c.arg(1)), guild(vm), id) {
        (Some(_), Some(g), Some(id)) => row_by_member_id(g, id),
        _ => None,
    };
    Ok(vec![row.map_or(Value::Nil, |r| club_member_info(vm, r))])
}

/// `C_Club.GetMemberInfoForSelf(clubId)`: the player's in the guild's
/// club; nil for any other (recorded "x", "" nil).
fn get_member_info_for_self(vm: &mut Vm, c: NativeCall) -> Ret {
    if guild_club(vm, c.arg(1)).is_none() {
        return Ok(vec![Value::Nil]);
    }
    Ok(vec![club_member_info(vm, Row::Player)])
}

/// The guild club's streams: Guild (1, `Enum.ClubStreamType.Guild`) and,
/// for a rank that hears officer chat, Officer (2, leaders and
/// moderators only); their IDs 1 and 2 (unverified: the client's IDs,
/// names, subjects).
fn guild_streams(vm: &mut Vm) -> Vec<Value> {
    let officer = can(vm, flag::OFFICER_LISTEN);
    let mut out = vec![stream(vm, 1.0)];
    if officer {
        out.push(stream(vm, 2.0));
    }
    out
}

fn stream(vm: &mut Vm, id: f64) -> Value {
    let t = vm.new_table();
    set(vm, t, "streamId", Value::Num(id));
    set_str(vm, t, "name", if id == 1.0 { "Guild" } else { "Officer" });
    set_str(vm, t, "subject", "");
    set(vm, t, "leadersAndModeratorsOnly", Value::Bool(id == 2.0));
    set(vm, t, "streamType", Value::Num(id));
    set(vm, t, "creationTime", Value::Num(0.0));
    Value::Table(t)
}

/// `C_Club.GetStreams(clubId)`: the guild club's streams; any other club
/// none (recorded "", "x", {} empty).
fn get_streams(vm: &mut Vm, c: NativeCall) -> Ret {
    let streams = if guild_club(vm, c.arg(1)).is_some() {
        guild_streams(vm)
    } else {
        Vec::new()
    };
    Ok(vec![list(vm, streams)])
}

/// `C_Club.GetStreamInfo(clubId, streamId)`: one of the guild's streams;
/// nil else.
fn get_stream_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(vm, c.arg(2));
    if guild_club(vm, c.arg(1)).is_none() {
        return Ok(vec![Value::Nil]);
    }
    let found = match id {
        Some(1.0) => stream(vm, 1.0),
        Some(2.0) if can(vm, flag::OFFICER_LISTEN) => stream(vm, 2.0),
        _ => Value::Nil,
    };
    Ok(vec![found])
}

/// `C_Club.AreMembersReady(clubId)`: the guild's club's members known
/// (unverified); any other false (recorded).
fn are_members_ready(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(guild_club(vm, c.arg(1)).is_some())])
}

// ---------------------------------------------------------------- controls

fn str_field(vm: &mut Vm, t: TableRef, key: &str) -> Option<String> {
    let k = vm.str(key);
    match vm.heap.table(t).get(k) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn num_field(vm: &mut Vm, t: TableRef, key: &str) -> Option<f64> {
    let k = vm.str(key);
    match vm.heap.table(t).get(k) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn bool_field(vm: &mut Vm, t: TableRef, key: &str) -> Option<bool> {
    let k = vm.str(key);
    match vm.heap.table(t).get(k) {
        Value::Bool(b) => Some(b),
        _ => None,
    }
}

fn table_field(vm: &mut Vm, t: TableRef, key: &str) -> Option<TableRef> {
    let k = vm.str(key);
    match vm.heap.table(t).get(k) {
        Value::Table(t) => Some(t),
        _ => None,
    }
}

fn items(vm: &Vm, t: TableRef) -> Vec<Value> {
    (1..)
        .map_while(|i| match vm.heap.table(t).get(Value::Num(f64::from(i))) {
            Value::Nil => None,
            v => Some(v),
        })
        .collect()
}

/// A member's fields from a control's table: `{ name, lastName?, guid?,
/// class?, race?, level?, rank?, zone?, note?, officerNote?, online?,
/// status?, lastOnline? = { years, months, days, hours },
/// achievementPoints?, achievementRank? }`.
fn apply_member(vm: &mut Vm, t: TableRef, m: &mut Member, ranks: usize) -> Result<(), String> {
    if let Some(v) = str_field(vm, t, "name") {
        m.name = v;
    }
    if let Some(v) = str_field(vm, t, "lastName") {
        m.last_name = Some(v);
    }
    if let Some(v) = str_field(vm, t, "guid") {
        m.guid = v;
    }
    if let Some(v) = str_field(vm, t, "class") {
        let v = v.to_uppercase();
        if units::class_name_of(&v).is_none() {
            return Err(format!("class: no class {v}"));
        }
        m.class = v;
    }
    if let Some(v) = str_field(vm, t, "race") {
        m.race = v;
    }
    if let Some(v) = num_field(vm, t, "level") {
        m.level = v;
    }
    if let Some(v) = num_field(vm, t, "rank") {
        if v < 0.0 || v as usize >= ranks {
            return Err(format!("rank: an index from 0 to {} expected", ranks - 1));
        }
        m.rank = v as usize;
    }
    if let Some(v) = str_field(vm, t, "zone") {
        m.zone = v;
    }
    if let Some(v) = str_field(vm, t, "note") {
        m.note = v;
    }
    if let Some(v) = str_field(vm, t, "officerNote") {
        m.officer_note = v;
    }
    if let Some(v) = bool_field(vm, t, "online") {
        m.online = v;
    }
    if let Some(v) = num_field(vm, t, "status") {
        m.status = v;
    }
    if let Some(away) = table_field(vm, t, "lastOnline") {
        let values = items(vm, away);
        for (slot, v) in m.last_online.iter_mut().zip(values) {
            if let Value::Num(n) = v {
                *slot = n;
            }
        }
    }
    if let Some(v) = num_field(vm, t, "achievementPoints") {
        m.achievement_points = v;
    }
    if let Some(v) = num_field(vm, t, "achievementRank") {
        m.achievement_rank = v;
    }
    if m.name.is_empty() {
        return Err("a member's name expected".into());
    }
    Ok(())
}

/// `joinGuild(guild?)`: the player in a guild: with no table the recorded
/// one (the player its leader among 26 others); with one, a guild of the
/// default ranks with these fields: `name`, `rank` (the player's index, 0
/// the leader), `ranks` (`{ { name, flags? = { [n] = true, … } }, … }`,
/// the leader's first), `members` (as `defineGuildMember`'s; none by
/// default), `motd`, `info`, `bankMoney`, `bankTabs`. `PLAYER_GUILD_UPDATE`
/// a round trip later.
pub fn control_join(vm: &mut Vm, c: NativeCall) -> Ret {
    let g = match c.arg(1) {
        Value::Table(t) => match guild_of(vm, t) {
            Ok(g) => g,
            Err(e) => return Err(vm.error(&format!("joinGuild: {e}"))),
        },
        Value::Nil => Guild::recorded(),
        _ => return Err(vm.error("joinGuild: a table or nothing expected")),
    };
    engine_mut(vm).world.chat.guild = Some(g);
    player_guild_changed(vm);
    Ok(vec![])
}

fn guild_of(vm: &mut Vm, t: TableRef) -> Result<Guild, String> {
    let mut g = Guild::new(0);
    if let Some(ranks) = table_field(vm, t, "ranks") {
        let mut out = Vec::new();
        for v in items(vm, ranks) {
            let Value::Table(r) = v else {
                return Err("ranks: a list of { name, flags? } expected".into());
            };
            let name = str_field(vm, r, "name").ok_or("ranks: each a name")?;
            let mut flags = [false; FLAGS];
            if let Some(f) = table_field(vm, r, "flags") {
                for (i, slot) in flags.iter_mut().enumerate() {
                    *slot = vm.heap.table(f).get(Value::Num(i as f64 + 1.0)).truthy();
                }
            } else if out.is_empty() {
                flags = default_ranks()[0].flags;
            } else {
                flags[0] = true;
                flags[1] = true;
            }
            out.push(Rank { name, flags });
        }
        if out.len() < 2 {
            return Err("ranks: at least two expected".into());
        }
        g.ranks = out;
    }
    if let Some(v) = str_field(vm, t, "name") {
        g.name = v;
    }
    if let Some(v) = num_field(vm, t, "rank") {
        if v < 0.0 || v as usize >= g.ranks.len() {
            return Err(format!(
                "rank: an index from 0 to {} expected",
                g.ranks.len() - 1
            ));
        }
        g.rank = v as usize;
    }
    if let Some(v) = str_field(vm, t, "motd") {
        g.motd = v;
    }
    if let Some(v) = str_field(vm, t, "info") {
        g.info = v;
    }
    if let Some(v) = num_field(vm, t, "bankMoney") {
        g.bank_money = v;
    }
    if let Some(v) = num_field(vm, t, "bankTabs") {
        g.bank_tabs = v.max(0.0) as usize;
    }
    if let Some(members) = table_field(vm, t, "members") {
        for v in items(vm, members) {
            let Value::Table(one) = v else {
                return Err("members: a list of tables expected".into());
            };
            let mut m = g.member("", None);
            apply_member(vm, one, &mut m, g.ranks.len())?;
            g.members.push(m);
        }
    }
    Ok(g)
}

/// `leaveGuild()`: the player in no guild (`PLAYER_GUILD_UPDATE` a round
/// trip later).
pub fn control_leave(vm: &mut Vm, _c: NativeCall) -> Ret {
    if engine_mut(vm).world.chat.guild.take().is_some() {
        player_guild_changed(vm);
    }
    Ok(vec![])
}

/// `defineGuildMember(member)`: another member of the player's guild (as
/// `apply_member` reads it; offline at the lowest rank by default); its
/// handle. The roster told (`GUILD_ROSTER_UPDATE`).
pub fn control_define_member(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("defineGuildMember: a table expected"));
    };
    let Some(g) = guild_mut(vm) else {
        return Err(vm.error("defineGuildMember: the player is in no guild"));
    };
    let mut m = g.member("", None);
    let ranks = g.ranks.len();
    if let Err(e) = apply_member(vm, t, &mut m, ranks) {
        return Err(vm.error(&format!("defineGuildMember: {e}")));
    }
    let handle = m.handle;
    if let Some(g) = guild_mut(vm) {
        g.members.push(m);
    }
    roster_changed(vm);
    Ok(vec![Value::Num(f64::from(handle))])
}

fn member_handle(vm: &mut Vm, c: &NativeCall, control: &str) -> Result<usize, wowlua_vm::LuaError> {
    let handle = match c.arg(1) {
        Value::Num(n) if n >= 1.0 && n.fract() == 0.0 => n as u32,
        _ => return Err(vm.error(&format!("{control}: a member's handle expected"))),
    };
    guild(vm)
        .and_then(|g| g.members.iter().position(|m| m.handle == handle))
        .ok_or_else(|| vm.error(&format!("{control}: no member {handle}")))
}

/// `updateGuildMember(handle, changes)`: the member changed (coming
/// online, a new level, …); the roster told.
pub fn control_update_member(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = member_handle(vm, &c, "updateGuildMember")?;
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("updateGuildMember: a table of fields expected"));
    };
    let (mut m, ranks) = {
        let g = guild(vm).expect("in a guild");
        (g.members[i].clone(), g.ranks.len())
    };
    if let Err(e) = apply_member(vm, t, &mut m, ranks) {
        return Err(vm.error(&format!("updateGuildMember: {e}")));
    }
    if let Some(g) = guild_mut(vm) {
        g.members[i] = m;
    }
    roster_changed(vm);
    Ok(vec![])
}

/// `removeGuildMember(handle)`: the member gone from the guild; the
/// roster told.
pub fn control_remove_member(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = member_handle(vm, &c, "removeGuildMember")?;
    if let Some(g) = guild_mut(vm) {
        g.members.remove(i);
    }
    roster_changed(vm);
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        // Membership.
        ("IsInGuild", is_in_guild),
        ("GetGuildInfo", get_guild_info),
        ("GetNumGuildMembers", get_num_guild_members),
        ("SetGuildRosterSelection", set_guild_roster_selection),
        ("GetGuildRosterSelection", get_guild_roster_selection),
        ("GetGuildRosterInfo", get_guild_roster_info),
        ("GetGuildRosterLastOnline", get_guild_roster_last_online),
        ("IsGuildLeader", is_guild_leader),
        ("IsGuildMember", is_guild_member),
        ("UnitIsInMyGuild", unit_is_in_my_guild),
        ("IsPlayerInGuildFromGUID", is_player_in_guild_from_guid),
        ("C_GuildInfo.MemberExistsByName", member_exists_by_name),
        ("C_GuildInfo.GetGuildRankOrder", get_guild_rank_order),
        // Ranks.
        ("GuildControlGetNumRanks", guild_control_get_num_ranks),
        ("GuildControlGetRankName", guild_control_get_rank_name),
        (
            "C_GuildInfo.GuildControlGetRankFlags",
            guild_control_get_rank_flags,
        ),
        (
            "GuildControlGetAllowedShifts",
            guild_control_get_allowed_shifts,
        ),
        ("GetNumMembersInRank", get_num_members_in_rank),
        ("GetPromotionRank", get_promotion_rank),
        ("GetDemotionRank", get_demotion_rank),
        (
            "IsGuildRankAssignmentAllowed",
            is_guild_rank_assignment_allowed,
        ),
        (
            "C_GuildInfo.IsGuildRankAssignmentAllowed",
            c_is_guild_rank_assignment_allowed,
        ),
        // Rights.
        ("C_GuildInfo.CanSpeakInGuildChat", can_speak),
        ("CanGuildInvite", can_invite),
        ("CanGuildPromote", can_promote),
        ("CanGuildDemote", can_demote),
        ("CanGuildRemove", can_remove),
        ("CanEditMOTD", can_edit_motd),
        ("CanEditPublicNote", can_edit_public_note),
        ("C_GuildInfo.CanViewOfficerNote", can_view_officer_note),
        ("C_GuildInfo.CanEditOfficerNote", can_edit_officer_note),
        ("CanEditGuildInfo", can_edit_info),
        ("CanGuildBankRepair", can_bank_repair),
        ("CanWithdrawGuildBankMoney", can_withdraw_money),
        ("CanEditGuildBankTabInfo", can_edit_bank_tabs),
        ("C_GuildInfo.IsGuildOfficer", is_officer),
        ("CanEditGuildTabInfo", can_edit_guild_tab_info),
        ("CanSignPetition", can_sign_petition),
        (
            "C_GuildInfo.GetPreferredPlaySettingsFeatures",
            play_settings_features,
        ),
        (
            "C_GuildInfo.RequestPreferredPlaySettings",
            request_play_settings,
        ),
        // Texts.
        ("GetGuildRosterMOTD", get_roster_motd),
        ("C_GuildInfo.GetMOTD", get_roster_motd),
        ("GetGuildInfoText", get_info_text),
        ("C_GuildInfo.GetInfoText", get_info_text),
        ("C_GuildInfo.SetMOTD", set_motd),
        ("GuildSetMOTD", set_motd),
        ("C_GuildInfo.SetInfoText", set_info_text),
        ("SetGuildInfoText", set_info_text),
        ("C_GuildInfo.SetNote", set_note),
        // The roster's changes.
        ("C_GuildInfo.GuildRoster", guild_roster),
        ("GuildPromote", promote),
        ("C_GuildInfo.Promote", promote),
        ("GuildDemote", demote),
        ("C_GuildInfo.Demote", demote),
        ("SetGuildMemberRank", set_guild_member_rank),
        ("GuildUninvite", uninvite),
        ("C_GuildInfo.Uninvite", uninvite),
        ("C_GuildInfo.RemoveFromGuild", remove_from_guild),
        ("GuildSetLeader", set_leader),
        ("C_GuildInfo.SetLeader", set_leader),
        ("GuildLeave", leave),
        ("C_GuildInfo.Leave", leave),
        ("GuildDisband", disband),
        ("C_GuildInfo.Disband", disband),
        // The bank.
        ("GetGuildBankMoney", get_guild_bank_money),
        ("GetNumGuildBankTabs", get_num_guild_bank_tabs),
        ("GetGuildBankTabCost", get_guild_bank_tab_cost),
        ("GetGuildBankWithdrawMoney", get_guild_bank_withdraw_money),
        // Professions.
        ("GetNumGuildTradeSkill", get_num_guild_trade_skill),
        ("GetGuildTradeSkillInfo", get_guild_trade_skill_info),
        // The calendar.
        ("C_Calendar.GetDefaultGuildFilter", get_default_guild_filter),
        // The club.
        ("C_Club.GetGuildClubId", get_guild_club_id),
        ("C_Club.GetSubscribedClubs", get_subscribed_clubs),
        ("C_Club.GetClubInfo", get_club_info),
        ("C_Club.GetClubMembers", get_club_members),
        ("C_Club.GetMemberInfo", get_member_info),
        ("C_Club.GetMemberInfoForSelf", get_member_info_for_self),
        ("C_Club.GetStreams", get_streams),
        ("C_Club.GetStreamInfo", get_stream_info),
        ("C_Club.AreMembersReady", are_members_ready),
    ]
}

#[cfg(test)]
mod tests {
    /// No function has two behaviours here.
    #[test]
    fn each_function_once() {
        let mut names: Vec<&str> = super::behaviours().iter().map(|(n, _)| *n).collect();
        let all = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all);
    }

    /// The recorded guild: 27 members, the leader's rights all but 14 and
    /// 18.
    #[test]
    fn the_recorded_guild() {
        let g = super::Guild::recorded();
        assert_eq!(g.members.len() + 1, 27);
        assert_eq!(g.ranks.len(), 5);
        assert!(g.can(1) && g.can(22) && !g.can(14) && !g.can(18));
        assert_eq!(g.trade_skills.len(), 10);
    }
}
