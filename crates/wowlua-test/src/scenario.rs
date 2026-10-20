//! A scenario's log as the replays compare it (`/stdtest engine scenario
//! <name>`, `crates/wowlua-cli/tests/engine_probe.rs`) and the coverage
//! report counts it (`cargo xtask coverage-report`): each line without its
//! time, the world's noise and the recordings' own mistakes left out.

/// A recording's log (`tests/client/scenarios/<name>.lua`, a table of
/// numbered lines) in its order.
pub fn recording(src: &[u8]) -> Result<Vec<String>, String> {
    use wowlua_vm::{Value, Vm};
    let mut vm = Vm::new();
    let f = vm.load(src, "=recording")?;
    let r = vm.call(f, &[]).map_err(|e| format!("{:?}", e.value))?;
    let Some(Value::Table(t)) = r.first().copied() else {
        return Err("the recording is not a table".into());
    };
    let text = |vm: &Vm, s| String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    let mut lines: Vec<(String, String)> = vm
        .heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((text(&vm, k), text(&vm, v))),
            _ => None,
        })
        .filter(|(k, _)| k.bytes().all(|b| b.is_ascii_digit()))
        .collect();
    lines.sort();
    Ok(lines.into_iter().map(|(_, v)| v).collect())
}

/// A log's lines as compared: each [`line`]'s, the answers to the
/// player's moving left out with it (`UnitOnTaxi` after
/// `PLAYER_STARTED_MOVING`, every frame while following).
pub fn lines(log: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut after_moving = false;
    for l in log {
        let body = l.split_once(' ').map_or(l.as_str(), |(_, b)| b);
        if body.starts_with("= ") {
            if after_moving {
                continue;
            }
        } else {
            after_moving = body.starts_with("PLAYER_STARTED_MOVING")
                || body.starts_with("PLAYER_STOPPED_MOVING");
        }
        if let Some(line) = line(l) {
            out.push(line);
        }
    }
    out
}

/// A pattern compiled on first use.
type Pattern = std::sync::LazyLock<regex::Regex>;

/// A scenario log's line as compared: without its time; none for the
/// world's noise and the recordings' own mistakes.
pub fn line(line: &str) -> Option<String> {
    let body = line.split_once(' ').map_or(line, |(_, b)| b);
    const NOISE: &[&str] = &[
        "NAME_PLATE_",
        "UNIT_POWER_FREQUENT",
        "SPELL_UPDATE_USABLE",
        "DAMAGE_METER_",
        "WORLD_CURSOR_",
        "CURSOR_CHANGED",
        "UPDATE_MOUSEOVER_UNIT",
        "GLOBAL_",
        "MODIFIER_STATE_CHANGED",
        "CONSOLE_MESSAGE",
        "PLAYER_SOFT_",
        "ACTION_RANGE_CHECK_UPDATE",
        "CRITERIA_UPDATE",
        "PLAYER_STARTED_",
        "PLAYER_STOPPED_",
        "ITEM_DATA_LOAD_RESULT",
        "SPELL_DATA_LOAD_RESULT",
        "QUEST_DATA_LOAD_RESULT",
        "ADDON_ACTION_BLOCKED",
        "SPELL_ACTIVATION_OVERLAY_HIDE",
        "UNIT_HAPPINESS",
    ];
    if NOISE.iter().any(|n| body.starts_with(n))
        || body.contains("nameplate")
        || body.contains("softinteract")
    {
        return None;
    }
    // The recordings' mistaken asks (fixed in the recorder since), and the
    // asks a protected call blocks in combat.
    const MISTAKEN: &[&str] = &[
        "= UnitIsEnemy(",
        "= UnitIsFriend(",
        "= UnitCanAttack(",
        "= UnitCanAssist(",
        "= UnitIsCivilian(",
        "= GetSpellCooldown(",
        "= C_Spell.GetSpellCooldown(",
        "= GetLootMethod(",
        "= C_PartyInfo.GetLootMethod(",
        "= CheckInteractDistance(",
    ];
    if MISTAKEN.iter().any(|m| body.starts_with(m)) {
        return None;
    }
    // Players' GUIDs as the recordings have them (scrubbed); a chat
    // message's line number (the session's). (Each pattern compiled once:
    // a scenario has thousands of lines.)
    static GUID: Pattern = Pattern::new(|| regex::Regex::new(r"Player-\d+-[0-9A-F]{8}").unwrap());
    static LINE: Pattern = Pattern::new(|| {
        regex::Regex::new(r"^(CHAT_MSG_.*; number )\d+(; (?:nil nil|string [^;]*); number 0;)")
            .unwrap()
    });
    let (guid, line) = (&*GUID, &*LINE);
    let body = guid.replace_all(body, "Player-0000-00000000");
    // A cast's GUID by its kind and spell (its server, map and number are
    // the session's).
    static CAST: Pattern =
        Pattern::new(|| regex::Regex::new(r"Cast-(\d+)-\d+-\d+-\d+-(\d+)-[0-9A-F]+").unwrap());
    let cast = &*CAST;
    let body = cast.replace_all(&body, "Cast-$1-$2");
    // A party's ID, the session's.
    static PARTY: Pattern = Pattern::new(|| regex::Regex::new(r"Party-\d+-\d+-[0-9A-F]+").unwrap());
    let party = &*PARTY;
    let body = party.replace_all(&body, "Party-<id>");
    // A cast bar's ID, the session's.
    static BAR: Pattern = Pattern::new(|| regex::Regex::new(r"CastBar-[0-9A-F]+").unwrap());
    let bar = &*BAR;
    let body = bar.replace_all(&body, "CastBar-<id>");
    // An item's GUID, the server's (a made item's: the session's).
    static ITEM: Pattern = Pattern::new(|| regex::Regex::new(r"Item-\d+-0-[0-9A-F]{16}").unwrap());
    let item = &*ITEM;
    let body = item.replace_all(&body, "Item-<id>");
    // A value's address, the session's (the ingest drops the client's).
    static ADDRESS: Pattern = Pattern::new(|| {
        regex::Regex::new(r"(function|table|userdata|thread): 0x[0-9a-f]+").unwrap()
    });
    let body = ADDRESS.replace_all(&body, "$1");
    let body = line.replace(&body, "${1}<line>${2}").into_owned();
    // An answer's trailing nils dropped (the first recordings' recorder
    // lost them).
    if let Some((call, answer)) = body.split_once(" -> ")
        && let Some((n, values)) = answer.split_once(": ")
        && n.parse::<usize>().is_ok()
    {
        let mut list: Vec<&str> = if values.is_empty() {
            vec![]
        } else {
            values.split(", ").collect()
        };
        while list.last() == Some(&"nil nil") {
            list.pop();
        }
        return Some(format!("{call} -> {}: {}", list.len(), list.join(", ")));
    }
    Some(body)
}
