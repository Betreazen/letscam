//! Labels of the mobs' attack phases for the contact sheets, from the SoulsCraft server's
//! `soulscraft:state` mod messages the replay carries (its `wire.rs`, version 1). One note per mob
//! whose clip or flags changed: the clip and the server tick it started. The phase is counted from
//! there; with `/sc debug` on, every tick carries a ready label (`overhead:hold 12`) instead.

use std::collections::BTreeMap;

/// The channel SoulsCraft's server sends its mobs' state on.
pub const STATE_CHANNEL: &str = "soulscraft:state";
const VERSION: u8 = 1;
const RAGE: u8 = 1;
const GUARD: u8 = 2;
const MS_PER_TICK: f64 = 50.0;
/// Mobs named in one label at most.
const MAX_MOBS: usize = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Note {
    pub id: u32,
    pub clip: String,
    pub start: i64,
    pub flags: u8,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Batch {
    pub now: i64,
    pub notes: Vec<Note>,
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let n = usize::from(self.u8()?);
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

/// A `soulscraft:state` message; None for another version or broken bytes.
pub fn decode(data: &[u8]) -> Option<Batch> {
    let mut r = Reader(data);
    if r.u8()? != VERSION {
        return None;
    }
    let now = r.i64()?;
    let count = u16::from_le_bytes(r.take(2)?.try_into().ok()?);
    let mut notes = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let id = u32::from_le_bytes(r.take(4)?.try_into().ok()?);
        let clip = r.str()?;
        let start = r.i64()?;
        let flags = r.u8()?;
        let _size = r.u8()?;
        let label = r.str()?;
        notes.push(Note { id, clip, start, flags, label });
    }
    r.0.is_empty().then_some(Batch { now, notes })
}

/// A mob's latest note and the replay time it came at.
#[derive(Clone, Debug)]
struct Seen {
    note: Note,
    server_now: i64,
    at_ms: u32,
}

/// What the mobs are doing, as the replay's messages told it so far.
#[derive(Clone, Debug, Default)]
pub struct MobLabels {
    mobs: BTreeMap<u32, Seen>,
}

impl MobLabels {
    /// Takes a message the replay just played at `at_ms`.
    pub fn apply(&mut self, batch: Batch, at_ms: u32) {
        for note in batch.notes {
            let seen = Seen { note, server_now: batch.now, at_ms };
            self.mobs.insert(seen.note.id, seen);
        }
    }

    /// Forgets everything (a seek back in the replay).
    pub fn clear(&mut self) {
        self.mobs.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.mobs.is_empty()
    }

    /// The line for the frame at replay time `t_ms`: each mob in an attack or a stance, with
    /// `name` giving its short name (None: the mob is gone, left out). Empty when none is.
    pub fn line(&self, t_ms: u32, name: impl Fn(u32) -> Option<String>) -> String {
        let parts: Vec<String> = self
            .mobs
            .values()
            .filter_map(|seen| Some(format!("{} {}", name(seen.note.id)?, mob_text(seen, t_ms)?)))
            .take(MAX_MOBS)
            .collect();
        parts.join(" · ")
    }
}

/// One mob's part: the debug label, else the clip with the ticks since it began.
fn mob_text(seen: &Seen, t_ms: u32) -> Option<String> {
    let note = &seen.note;
    let mut text = if !note.label.is_empty() {
        note.label.clone()
    } else if note.clip.is_empty() {
        String::new()
    } else {
        let since = f64::from(t_ms.saturating_sub(seen.at_ms)) / MS_PER_TICK;
        let ticks = seen.server_now - note.start + since.floor() as i64;
        let clip = note.clip.strip_prefix("attack_").unwrap_or(&note.clip);
        format!("{clip} +{ticks}")
    };
    for (bit, word) in [(RAGE, "ярость"), (GUARD, "блок")] {
        if note.flags & bit != 0 {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(word);
        }
    }
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_str(out: &mut Vec<u8>, s: &str) {
        out.push(s.len() as u8);
        out.extend_from_slice(s.as_bytes());
    }

    /// The bytes SoulsCraft's `wire::encode` makes.
    fn encode(b: &Batch) -> Vec<u8> {
        let mut out = vec![VERSION];
        out.extend_from_slice(&b.now.to_le_bytes());
        out.extend_from_slice(&(b.notes.len() as u16).to_le_bytes());
        for n in &b.notes {
            out.extend_from_slice(&n.id.to_le_bytes());
            put_str(&mut out, &n.clip);
            out.extend_from_slice(&n.start.to_le_bytes());
            out.push(n.flags);
            out.push(0);
            put_str(&mut out, &n.label);
        }
        out
    }

    fn note(id: u32, clip: &str, start: i64, flags: u8, label: &str) -> Note {
        Note { id, clip: clip.into(), start, flags, label: label.into() }
    }

    #[test]
    fn decodes_what_soulscraft_sends() {
        let b = Batch {
            now: 1000,
            notes: vec![note(7, "attack_overhead", 990, GUARD, "overhead:hold 6"), note(9, "", 1, RAGE, "")],
        };
        let bytes = encode(&b);
        assert_eq!(decode(&bytes), Some(b));
        assert_eq!(decode(&bytes[..bytes.len() - 1]), None);
        let mut other = bytes.clone();
        other[0] = 2;
        assert_eq!(decode(&other), None);
    }

    #[test]
    fn counts_the_clip_ticks_from_its_start() {
        let mut labels = MobLabels::default();
        labels.apply(Batch { now: 1000, notes: vec![note(7, "attack_overhead", 995, 0, "")] }, 2000);
        let name = |id| (id == 7).then(|| "revenant".to_owned());
        assert_eq!(labels.line(2000, name), "revenant overhead +5");
        assert_eq!(labels.line(2149, name), "revenant overhead +7");
    }

    #[test]
    fn debug_labels_win_and_idle_mobs_are_left_out() {
        let mut labels = MobLabels::default();
        let notes = vec![
            note(3, "attack_slam", 10, 0, "slam:hit 2"),
            note(4, "", 0, 0, ""),
            note(5, "", 0, RAGE | GUARD, ""),
            note(6, "attack_lunge", 0, 0, ""),
        ];
        labels.apply(Batch { now: 12, notes }, 0);
        let names = |id| (id != 6).then(|| format!("m{id}"));
        assert_eq!(labels.line(0, names), "m3 slam:hit 2 · m5 ярость блок");
        labels.clear();
        assert!(labels.is_empty());
        assert_eq!(labels.line(0, names), "");
    }
}
