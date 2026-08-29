//! Small helpers for reading facts out of decompiled C#. Shared by the table modules so each one
//! is about the shape of its table, not about re-deriving how to find a `Factory.CreateBoolSet`.

use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;

/// Read a decompiled file, replacing invalid UTF-8 rather than failing — ilspycmd's output carries
/// the occasional stray byte, and the old Python read these with `errors="replace"`.
pub fn read_lossy(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The members of a `Factory.CreateBoolSet(a, b, c, …)` — the tile/entity ids the game flips to
/// `true`. Panics if the set is missing, or if it opens with a `true`/`false` default this reader
/// does not model (none of the sets it is used on do).
pub fn bool_set(cs: &str, name: &str) -> Vec<u32> {
    let re = Regex::new(&format!(
        r"public static bool\[\] {} = Factory\.CreateBoolSet\(([^)]*)\);",
        regex::escape(name)
    ))
    .unwrap();
    let body = re
        .captures(cs)
        .unwrap_or_else(|| panic!("no bool set {name}"))
        .get(1)
        .unwrap()
        .as_str()
        .trim();
    if body.is_empty() {
        return Vec::new();
    }
    let parts: Vec<&str> = body
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    assert!(
        parts[0] != "true" && parts[0] != "false",
        "{name} flips its default, which this reader does not model"
    );
    parts.iter().map(|p| p.parse().unwrap()).collect()
}

/// A `Factory.CreateIntSet(default, key0, val0, key1, val1, …)`: the default, then the key→value
/// pairs. Returned as `(default, map)`; callers that only want the keys read `map.keys()`.
pub fn int_set(cs: &str, name: &str) -> (i64, BTreeMap<u32, i64>) {
    let re = Regex::new(&format!(
        r"public static int\[\] {} = Factory\.CreateIntSet\(([^)]*)\);",
        regex::escape(name)
    ))
    .unwrap();
    let body = re
        .captures(cs)
        .unwrap_or_else(|| panic!("no int set {name}"))
        .get(1)
        .unwrap()
        .as_str();
    let nums: Vec<i64> = Regex::new(r"-?\d+")
        .unwrap()
        .find_iter(body)
        .map(|m| m.as_str().parse().unwrap())
        .collect();
    let default = nums[0];
    let mut values = BTreeMap::new();
    let rest = &nums[1..];
    let mut i = 0;
    while i + 1 < rest.len() {
        values.insert(rest[i] as u32, rest[i + 1]);
        i += 2;
    }
    (default, values)
}

/// The single integer named `name` in a `public static readonly ushort name = N;` declaration.
pub fn ushort_const(cs: &str, name: &str) -> usize {
    Regex::new(&format!(
        r"public static readonly ushort {} = (\d+);",
        regex::escape(name)
    ))
    .unwrap()
    .captures(cs)
    .unwrap_or_else(|| panic!("no ushort const {name}"))
    .get(1)
    .unwrap()
    .as_str()
    .parse()
    .unwrap()
}
