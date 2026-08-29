//! `crates/terrustia-proto/src/projectile_data.rs` — stats for every projectile type.
//!
//! `Terraria/Projectile.cs`'s `SetDefaults(int Type)` is a flat `if (type == N) { ... } else if
//! (type == N) { ... }` chain preceded by a block of defaults, so it parses without needing to
//! understand C#. Anything whose body is not plain field assignments is skipped (it never gets a
//! size, so it is dropped as unclear rather than guessed at). Names come from
//! `Terraria.ID/ProjectileID.cs`'s constants, for readable logs.
//!
//! Ported from `gen_projectiles.py`.

use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;

use crate::csharp::read_lossy;

/// One parsed type's stats, using the game's own field names and defaults.
#[derive(Clone)]
struct Stats {
    width: i64,
    height: i64,
    ai_style: i64,
    penetrate: i64,
    time_left: i64,
    extra_updates: i64,
    tile_collide: bool,
    hostile: bool,
    knock_back: f64,
}

/// `SetDefaults` sets these before the per-type chain; anything a type does not override keeps
/// them.
const DEFAULTS: Stats = Stats {
    width: 0,
    height: 0,
    ai_style: 0,
    penetrate: 1,
    time_left: 3600,
    extra_updates: 0,
    tile_collide: true,
    hostile: false,
    knock_back: 0.0,
};

/// `ProjectileID`'s constant names, for readable logs. The first declaration of a value wins.
fn read_names(root: &Path) -> BTreeMap<i64, String> {
    let text = read_lossy(&root.join("Terraria.ID/ProjectileID.cs"));
    let re = Regex::new(r"public const short (\w+)\s*=\s*(\d+);").unwrap();
    let mut names: BTreeMap<i64, String> = BTreeMap::new();
    for caps in re.captures_iter(&text) {
        let value: i64 = caps[2].parse().unwrap();
        names.entry(value).or_insert_with(|| caps[1].to_string());
    }
    names
}

/// Fold a block's assignments for one type.
///
/// A grouped block narrows again inside itself: `if (type == 76) ... else if (type == 77) ...
/// else ...`, and the trailing bare `else` belongs to whichever member fell through. When the
/// condition line is not immediately followed by a brace-only line, its body is not captured at
/// all here (the following statement falls through to plain assignment, unconditionally) —
/// exactly as the original parser reads it.
fn read(lines: &[&str], want: i64) -> Stats {
    let if_one = Regex::new(r"^\s*if \(type == (\d+)\)\s*$").unwrap();
    let elif_one = Regex::new(r"^\s*else if \(type == (\d+)\)\s*$").unwrap();
    let else_one = Regex::new(r"^\s*else\s*$").unwrap();

    let mut stats = DEFAULTS;
    let mut depth: i64 = 0;
    let mut taken_at: BTreeMap<i64, bool> = BTreeMap::new();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let m_if = if_one.captures(line);
        let m_elif = elif_one.captures(line);
        let m_else = else_one.is_match(line);

        if m_if.is_some() || m_elif.is_some() || m_else {
            let already = if m_elif.is_some() || m_else {
                *taken_at.get(&depth).unwrap_or(&false)
            } else {
                false
            };
            let take = if let Some(c) = &m_if {
                c[1].parse::<i64>().unwrap() == want
            } else if let Some(c) = &m_elif {
                !already && c[1].parse::<i64>().unwrap() == want
            } else {
                !already
            };
            taken_at.insert(depth, already || take);

            i += 1;
            if i < lines.len() && lines[i].trim() == "{" {
                i += 1;
                let mut inner: i64 = 1;
                let mut body: Vec<&str> = Vec::new();
                while i < lines.len() && inner > 0 {
                    inner +=
                        lines[i].matches('{').count() as i64 - lines[i].matches('}').count() as i64;
                    if inner > 0 {
                        body.push(lines[i]);
                    }
                    i += 1;
                }
                if take {
                    for b in &body {
                        assign(b, &mut stats);
                    }
                }
            }
            continue;
        }
        assign(line, &mut stats);
        depth += line.matches('{').count() as i64 - line.matches('}').count() as i64;
        i += 1;
    }
    stats
}

fn assign(line: &str, stats: &mut Stats) {
    macro_rules! numeric {
        ($pattern:expr, $field:ident) => {
            if let Some(c) = Regex::new($pattern).unwrap().captures(line) {
                stats.$field = c[1].parse().unwrap();
            }
        };
    }
    numeric!(r"^\s*width\s*=\s*(-?\d+);", width);
    numeric!(r"^\s*height\s*=\s*(-?\d+);", height);
    numeric!(r"^\s*aiStyle\s*=\s*(-?\d+);", ai_style);
    numeric!(r"^\s*penetrate\s*=\s*(-?\d+);", penetrate);
    numeric!(r"^\s*timeLeft\s*=\s*(-?\d+);", time_left);
    numeric!(r"^\s*extraUpdates\s*=\s*(-?\d+);", extra_updates);

    if let Some(c) = Regex::new(r"^\s*tileCollide\s*=\s*(true|false);")
        .unwrap()
        .captures(line)
    {
        stats.tile_collide = &c[1] == "true";
    }
    if let Some(c) = Regex::new(r"^\s*hostile\s*=\s*(true|false);")
        .unwrap()
        .captures(line)
    {
        stats.hostile = &c[1] == "true";
    }
    if let Some(c) = Regex::new(r"^\s*knockBack\s*=\s*(-?[\d.]+)f?;")
        .unwrap()
        .captures(line)
    {
        stats.knock_back = c[1].parse().unwrap();
    }
}

/// Every type `SetDefaults` describes, and how many types had no size and were skipped.
fn parse(root: &Path) -> (BTreeMap<i64, Stats>, usize) {
    let text = read_lossy(&root.join("Terraria/Projectile.cs"));
    let lines: Vec<&str> = text.lines().collect();

    let start = lines
        .iter()
        .position(|l| l.contains("public void SetDefaults(int Type)"))
        .expect("SetDefaults not found");

    // Conditions come singly and in groups: `if (type == 674 || type == 673)`. Missing the
    // grouped form is what left the Dark Mage with no portal and no heal.
    let chain =
        Regex::new(r"^\s*(?:else\s+)?if \((type == \d+(?:\s*\|\|\s*type == \d+)*)\)\s*$").unwrap();
    let type_id_re = Regex::new(r"type == (\d+)").unwrap();

    // Groups, in the order the file declares them, keyed by the tuple of type ids in the
    // condition. A repeated identical group re-assigns its body in place, matching how a Python
    // dict reassignment leaves the key's iteration position untouched.
    let mut groups: Vec<(Vec<i64>, Vec<&str>)> = Vec::new();

    let mut i = start;
    while i < lines.len() {
        let line = lines[i];
        if let Some(m) = chain.captures(line) {
            let kinds: Vec<i64> = type_id_re
                .captures_iter(&m[1])
                .map(|c| c[1].parse().unwrap())
                .collect();
            i += 2; // skip the `{`
            let mut depth: i64 = 1;
            let mut body: Vec<&str> = Vec::new();
            while i < lines.len() && depth > 0 {
                let inner = lines[i];
                depth += inner.matches('{').count() as i64 - inner.matches('}').count() as i64;
                if depth > 0 {
                    body.push(inner);
                }
                i += 1;
            }
            if let Some(existing) = groups.iter_mut().find(|(k, _)| *k == kinds) {
                existing.1 = body;
            } else {
                groups.push((kinds, body));
            }
            continue;
        }
        if line.starts_with("\t}") && !groups.is_empty() {
            break;
        }
        i += 1;
    }

    let mut parsed: BTreeMap<i64, Stats> = BTreeMap::new();
    let mut unclear: usize = 0;
    for (kinds, body) in &groups {
        for &kind in kinds {
            let stats = read(body, kind);
            // A type with no size never moves; treat it as one we could not read.
            if stats.width == 0 && stats.height == 0 {
                unclear += 1;
                continue;
            }
            parsed.insert(kind, stats);
        }
    }
    (parsed, unclear)
}

fn emit(stats: &BTreeMap<i64, Stats>, names: &BTreeMap<i64, String>) -> String {
    let mut lines: Vec<String> = vec![
        "//! Stats for every projectile the game defines, generated from `Projectile.SetDefaults`."
            .into(),
        "//!".into(),
        "//! The server flies the ones an NPC fires or a trap throws; a player's own are simulated"
            .into(),
        "//! by their client and relayed, exactly as against a vanilla server. The whole table is"
            .into(),
        "//! here anyway, because [`ProjectileStats::hostile`] is what a client's claim is checked"
            .into(),
        "//! against — so a type missing from this file is a type a client could lie about.".into(),
        "//!".into(),
        "//! This was hand-written once and held 27 of them. The AI names 39, and".into(),
        "//! `Projectiles::launch` returns `None` for anything absent, so 32 kinds of shot were"
            .into(),
        "//! silently never fired: the Destroyer's lasers, Golem's fireballs, the Moon Lord's"
            .into(),
        "//! deathray, every one of the Empress of Light's attacks.".into(),
        "//!".into(),
        "//! Generated by `terrustia-codegen` from Terraria 1.4.5.7. Do not edit by hand.".into(),
        "".into(),
        "/// Everything the server needs to know about a projectile type.".into(),
        "#[derive(Debug, Clone, Copy, PartialEq)]".into(),
        "pub struct ProjectileStats {".into(),
        "    /// The `ProjectileID` constant name, for logs.".into(),
        "    pub name: &'static str,".into(),
        "    pub width: i32,".into(),
        "    pub height: i32,".into(),
        "    /// Which behaviour routine drives it.".into(),
        "    pub ai_style: i32,".into(),
        "    /// How many things it can hit before it dies. -1 means no limit.".into(),
        "    pub penetrate: i32,".into(),
        "    /// Ticks it lives for. The game's own default is 3600.".into(),
        "    pub time_left: i32,".into(),
        "    /// Whether terrain stops it.".into(),
        "    pub tile_collide: bool,".into(),
        "    /// Whether it hurts players, and so whether a client may claim to own one.".into(),
        "    pub hostile: bool,".into(),
        "    /// Extra movement steps per tick, which is how the fast ones stay accurate.".into(),
        "    pub extra_updates: i32,".into(),
        "    pub knockback: f32,".into(),
        "}".into(),
        "".into(),
        "/// How many types the table holds.".into(),
        format!("pub const COUNT: usize = {};", stats.len()),
        "".into(),
        "/// Stats for a projectile type, or `None` for one the game does not define.".into(),
        "pub fn projectile_stats(projectile_type: u16) -> Option<ProjectileStats> {".into(),
        "    let stats = match projectile_type {".into(),
    ];

    for (&kind, s) in stats {
        let name = names
            .get(&kind)
            .cloned()
            .unwrap_or_else(|| format!("Projectile{kind}"));
        lines.push(format!("        {kind} => ProjectileStats {{"));
        lines.push(format!("            name: \"{name}\","));
        lines.push(format!("            width: {},", s.width));
        lines.push(format!("            height: {},", s.height));
        lines.push(format!("            ai_style: {},", s.ai_style));
        lines.push(format!("            penetrate: {},", s.penetrate));
        lines.push(format!("            time_left: {},", s.time_left));
        lines.push(format!("            tile_collide: {},", s.tile_collide));
        lines.push(format!("            hostile: {},", s.hostile));
        lines.push(format!("            extra_updates: {},", s.extra_updates));
        lines.push(format!("            knockback: {:.1},", s.knock_back));
        lines.push("        },".into());
    }

    lines.extend(
        [
            "        _ => return None,",
            "    };",
            "    Some(stats)",
            "}",
            "",
            "#[cfg(test)]",
            "mod tests {",
            "    use super::*;",
            "",
            "    /// The table covers what the world actually throws.",
            "    #[test]",
            "    fn the_table_is_populated() {",
        ]
        .map(String::from),
    );
    lines.push(format!("        assert_eq!(COUNT, {});", stats.len()));
    lines.extend(
        [
            "    }",
            "",
            "    /// Spot checks against the game, on the ones whose absence broke a boss fight.",
            "    #[test]",
            "    fn the_boss_projectiles_are_here() {",
            "        for (kind, what) in [",
            "            (100u16, \"the Destroyer's laser\"),",
            "            (258, \"Golem's fireball\"),",
            "            (455, \"the Moon Lord's deathray\"),",
            "            (462, \"the Moon Lord's phantasmal sphere\"),",
            "            (385, \"Duke Fishron's bubble\"),",
            "            (435, \"a caster's bolt\"),",
            "        ] {",
            "            assert!(",
            "                projectile_stats(kind).is_some(),",
            "                \"{what} (type {kind}) is missing, so it would never be fired\",",
            "            );",
            "        }",
            "    }",
            "",
            "    /// A type the game does not define stays `None`.",
            "    #[test]",
            "    fn an_unknown_type_has_no_stats() {",
            "        assert!(projectile_stats(u16::MAX).is_none());",
            "    }",
            "}",
            "",
        ]
        .map(String::from),
    );

    lines.join("\n")
}

pub fn generate(root: &Path) -> String {
    let names = read_names(root);
    let (stats, unclear) = parse(root);
    assert!(
        stats.len() >= 500,
        "only parsed {} types; the parser is wrong",
        stats.len()
    );
    if unclear > 0 {
        eprintln!("note: {unclear} types had no size in SetDefaults and were skipped");
    }
    emit(&stats, &names)
}
