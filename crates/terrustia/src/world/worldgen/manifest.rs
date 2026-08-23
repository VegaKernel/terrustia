//! The generation record a world file carries, and what it is good for.
//!
//! Every `.wld` ends its header with a JSON manifest: each of the hundred and six generation
//! passes, in the order they ran, and the state of the world's random generator immediately
//! afterwards. The game writes it as a diagnostic. For a port it is something better — an oracle.
//!
//! Because the generator is never reseeded during generation, the value recorded after pass N is
//! a fingerprint of every draw made by passes one to N. Implement pass N, generate with the same
//! seed, and compare: if it matches, the RNG consumption of the whole prefix is exactly right,
//! and if it does not, the first pass that disagrees is the one that is wrong. That turns a
//! hundred-and-six-link chain, where nothing can be judged until all of it is done, into
//! something that can be checked one pass at a time.
//!
//! A pass the game was told to skip records no state at all, which is what makes deferring one
//! free rather than fatal: a stub that draws nothing matches a reference generated with that pass
//! disabled, for every pass after it.
//!
//! The reader below is hand-written rather than a JSON library. The shape is fixed and known — a
//! flat array of flat objects with four fields worth having — and this crate's dependency list is
//! short on purpose.

/// One pass, as the world file records it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassResult {
    pub name: String,
    /// The generator's next value immediately after this pass ran.
    ///
    /// Sampling it *takes* a draw, so a port has to take one at the same point or it will diverge
    /// from the very first pass.
    pub rand_next: i64,
    /// A hash of the whole world after this pass. The game only fills it in when its own
    /// world-generation debugger is switched on, so it is almost always absent.
    pub hash: Option<u32>,
    /// Whether the pass was told not to run, in which case it drew nothing.
    pub skipped: bool,
    pub duration_ms: i64,
}

/// The whole record.
#[derive(Debug, Clone, Default)]
pub struct Manifest {
    pub passes: Vec<PassResult>,
    pub version: Option<String>,
    pub final_hash: Option<u32>,
}

impl Manifest {
    /// Read one out of a world header's trailing JSON.
    ///
    /// The manifest is the last field of the header and the only JSON in it, so it is found
    /// rather than seeked to — the fields before it differ by format version.
    pub fn from_header(header: &[u8]) -> Option<Self> {
        let start = find(header, b"{\"GenPassResults\"")?;
        let end = header.iter().rposition(|&b| b == b'}')? + 1;
        if end <= start {
            return None;
        }
        Self::parse(std::str::from_utf8(&header[start..end]).ok()?)
    }

    /// Parse the manifest's JSON.
    pub fn parse(text: &str) -> Option<Self> {
        let mut manifest = Manifest {
            version: string_field(text, "Version"),
            final_hash: number_field(text, "FinalHash").and_then(|v| u32::try_from(v).ok()),
            ..Default::default()
        };
        // Each pass is one flat `{...}` inside the array, so they can be taken one at a time.
        let array = &text[find(text.as_bytes(), b"[")?..];
        for chunk in array.split('{').skip(1) {
            let Some(body) = chunk.split('}').next() else {
                continue;
            };
            let Some(name) = string_field(body, "Name") else {
                continue;
            };
            manifest.passes.push(PassResult {
                name,
                rand_next: number_field(body, "RandNext").unwrap_or(0),
                hash: number_field(body, "Hash").and_then(|v| u32::try_from(v).ok()),
                skipped: bool_field(body, "Skipped"),
                duration_ms: number_field(body, "DurationMs").unwrap_or(0),
            });
        }
        (!manifest.passes.is_empty()).then_some(manifest)
    }

    /// The passes that actually ran, in order.
    pub fn ran(&self) -> impl Iterator<Item = &PassResult> {
        self.passes.iter().filter(|p| !p.skipped)
    }

    /// What the generator should read after a named pass, if the world recorded it.
    pub fn after(&self, name: &str) -> Option<i64> {
        self.passes
            .iter()
            .find(|p| p.name == name && !p.skipped)
            .map(|p| p.rand_next)
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The text after `"key":"`, up to the closing quote.
fn string_field(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\":\""))? + key.len() + 4;
    let rest = &text[at..];
    Some(rest[..rest.find('"')?].to_string())
}

/// The number after `"key":`, or `None` for a null.
fn number_field(text: &str, key: &str) -> Option<i64> {
    let at = text.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = text[at..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn bool_field(text: &str, key: &str) -> bool {
    text.find(&format!("\"{key}\":"))
        .map(|at| text[at + key.len() + 3..].trim_start().starts_with("true"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"GenPassResults":[{"Name":"Terrain","DurationMs":152,"RandNext":9436581,"Hash":null,"Skipped":false},{"Name":"Dunes","DurationMs":3,"RandNext":524322473,"Hash":null,"Skipped":false},{"Name":"Dungeon","DurationMs":0,"RandNext":0,"Hash":null,"Skipped":true},{"Name":"Final Cleanup","DurationMs":237,"RandNext":566725750,"Hash":1578873622,"Skipped":false}],"Version":"v1.4.5.6","GitSHA":"","FinalHash":1578873622}"#;

    #[test]
    fn a_manifest_reads_out_of_a_header() {
        let mut header = b"some earlier header bytes\x00\x01".to_vec();
        header.extend_from_slice(SAMPLE.as_bytes());
        let manifest = Manifest::from_header(&header).expect("the manifest should be found");
        assert_eq!(manifest.passes.len(), 4);
        assert_eq!(manifest.version.as_deref(), Some("v1.4.5.6"));
        assert_eq!(manifest.final_hash, Some(1_578_873_622));
    }

    /// The order is the order the passes ran, which is the order a port has to follow.
    #[test]
    fn the_order_is_the_running_order() {
        let manifest = Manifest::parse(SAMPLE).unwrap();
        let names: Vec<&str> = manifest.passes.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Terrain", "Dunes", "Dungeon", "Final Cleanup"]);
        assert_eq!(manifest.passes[0].rand_next, 9_436_581);
        assert_eq!(manifest.passes[0].duration_ms, 152);
    }

    /// A skipped pass has no state to match, which is what makes deferring one free.
    #[test]
    fn a_skipped_pass_is_not_an_oracle() {
        let manifest = Manifest::parse(SAMPLE).unwrap();
        assert_eq!(manifest.after("Terrain"), Some(9_436_581));
        assert_eq!(manifest.after("Dungeon"), None, "it did not run");
        assert_eq!(manifest.ran().count(), 3);
    }

    /// A null hash is absent rather than zero — the difference between "the debugger was off"
    /// and "the world hashed to nothing".
    #[test]
    fn a_null_hash_is_absent() {
        let manifest = Manifest::parse(SAMPLE).unwrap();
        assert_eq!(manifest.passes[0].hash, None);
        assert_eq!(manifest.passes[3].hash, Some(1_578_873_622));
    }

    /// A header without one says so rather than guessing.
    #[test]
    fn a_header_with_no_manifest_gives_none() {
        assert!(Manifest::from_header(b"no json here at all").is_none());
        assert!(Manifest::parse("{\"GenPassResults\":[]}").is_none());
    }
}
