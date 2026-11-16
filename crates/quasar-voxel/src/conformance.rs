//! Record/replay harness for parity checks.
//!
//! A [`Recording`] captures a policy-driven stage's inputs and outputs as
//! named byte sections (snapshots, visit orders, propagated levels, emitted
//! geometry). Encodings are deterministic, so two runs of the same stage with
//! the same policies produce identical recordings, and [`Recording::compare`]
//! reports the first differing byte. Games point the same tooling at their
//! own reference data to prove parity.
use crate::chunk::ChannelId;
use crate::layout::ChunkLayout;
use crate::storage::Raw;
use crate::volume::{Snapshot, Volume};
use quasar_math::IVec3;

const MAGIC: &[u8; 4] = b"QVR1";

/// Named byte sections of one recorded stage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recording {
    pub stage: String,
    pub sections: Vec<(String, Vec<u8>)>,
}

/// The first difference between two recordings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    Stage {
        expected: String,
        actual: String,
    },
    MissingSection(String),
    ExtraSection(String),
    /// First differing byte (or the shorter length) of a section.
    Bytes {
        section: String,
        offset: usize,
        expected_len: usize,
        actual_len: usize,
    },
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stage { expected, actual } => {
                write!(f, "stage {actual:?}, expected {expected:?}")
            }
            Self::MissingSection(s) => write!(f, "missing section {s:?}"),
            Self::ExtraSection(s) => write!(f, "unexpected section {s:?}"),
            Self::Bytes {
                section,
                offset,
                expected_len,
                actual_len,
            } => write!(
                f,
                "section {section:?} differs at byte {offset} (lengths {expected_len} vs {actual_len})"
            ),
        }
    }
}

impl Recording {
    pub fn new(stage: &str) -> Self {
        Self {
            stage: stage.to_owned(),
            sections: Vec::new(),
        }
    }
    pub fn push(&mut self, name: &str, bytes: Vec<u8>) -> &mut Self {
        self.sections.push((name.to_owned(), bytes));
        self
    }
    pub fn section(&self, name: &str) -> Option<&[u8]> {
        self.sections
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.as_slice())
    }
    /// Self-describing binary form (magic, stage, then length-prefixed sections).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(MAGIC);
        let put = |out: &mut Vec<u8>, bytes: &[u8]| {
            out.extend((bytes.len() as u64).to_le_bytes());
            out.extend(bytes);
        };
        put(&mut out, self.stage.as_bytes());
        out.extend((self.sections.len() as u64).to_le_bytes());
        for (name, bytes) in &self.sections {
            put(&mut out, name.as_bytes());
            put(&mut out, bytes);
        }
        out
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        struct Reader<'a>(&'a [u8]);
        impl<'a> Reader<'a> {
            fn take(&mut self, n: usize) -> Option<&'a [u8]> {
                if self.0.len() < n {
                    return None;
                }
                let (head, tail) = self.0.split_at(n);
                self.0 = tail;
                Some(head)
            }
            fn len(&mut self) -> Option<usize> {
                usize::try_from(u64::from_le_bytes(self.take(8)?.try_into().ok()?)).ok()
            }
            fn bytes(&mut self) -> Option<Vec<u8>> {
                let n = self.len()?;
                self.take(n).map(<[u8]>::to_vec)
            }
        }
        let mut r = Reader(bytes.strip_prefix(MAGIC)?);
        let stage = String::from_utf8(r.bytes()?).ok()?;
        let count = r.len()?;
        let mut sections = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let name = String::from_utf8(r.bytes()?).ok()?;
            sections.push((name, r.bytes()?));
        }
        r.0.is_empty().then_some(Self { stage, sections })
    }
    /// The first difference from `expected`, or `None` when identical.
    pub fn compare(&self, expected: &Self) -> Option<Mismatch> {
        if self.stage != expected.stage {
            return Some(Mismatch::Stage {
                expected: expected.stage.clone(),
                actual: self.stage.clone(),
            });
        }
        for (name, want) in &expected.sections {
            let Some(got) = self.section(name) else {
                return Some(Mismatch::MissingSection(name.clone()));
            };
            if got != want.as_slice() {
                let offset = got
                    .iter()
                    .zip(want)
                    .position(|(a, b)| a != b)
                    .unwrap_or(got.len().min(want.len()));
                return Some(Mismatch::Bytes {
                    section: name.clone(),
                    offset,
                    expected_len: want.len(),
                    actual_len: got.len(),
                });
            }
        }
        self.sections
            .iter()
            .find(|(n, _)| expected.section(n).is_none())
            .map(|(n, _)| Mismatch::ExtraSection(n.clone()))
    }
}

/// Snapshot bytes: edge, pad, coordinate, then raw values (little endian).
pub fn record_snapshot<R: Raw>(snapshot: &Snapshot<R>) -> Vec<u8> {
    let mut out = Vec::with_capacity(snapshot.data().len() * 4 + 24);
    out.extend((snapshot.edge() as u32).to_le_bytes());
    out.extend(snapshot.pad().to_le_bytes());
    for c in snapshot.coord().to_array() {
        out.extend(c.to_le_bytes());
    }
    for v in snapshot.data() {
        out.extend(v.to_u32().to_le_bytes());
    }
    out
}

/// Rebuilds a snapshot from [`record_snapshot`] bytes (replay input).
pub fn snapshot_from_bytes<R: Raw>(bytes: &[u8]) -> Option<Snapshot<R>> {
    let word = |i: usize| -> Option<u32> {
        Some(u32::from_le_bytes(
            bytes.get(i * 4..i * 4 + 4)?.try_into().ok()?,
        ))
    };
    let edge = word(0)? as usize;
    let pad = word(1)?;
    let coord = IVec3::new(word(2)? as i32, word(3)? as i32, word(4)? as i32);
    let count = edge.checked_mul(edge)?.checked_mul(edge)?;
    if bytes.len() != (5 + count) * 4 || edge <= 2 * pad as usize {
        return None;
    }
    let data = (0..count)
        .map(|i| word(5 + i).map(R::from_u32))
        .collect::<Option<Vec<R>>>()?;
    let mut snapshot = Snapshot::new();
    snapshot.fill_with(edge, pad, coord, data);
    Some(snapshot)
}

/// Visit order bytes (e.g. a visibility traversal).
pub fn record_visits(visits: &[IVec3]) -> Vec<u8> {
    visits
        .iter()
        .flat_map(|v| v.to_array())
        .flat_map(i32::to_le_bytes)
        .collect()
}

/// Levels of a `u8` channel over an inclusive box in `y, z, x` order.
pub fn record_levels<L: ChunkLayout>(
    volume: &Volume<L>,
    channel: ChannelId<u8>,
    min: IVec3,
    max: IVec3,
) -> Vec<u8> {
    let mut out = Vec::new();
    volume.for_each_in_box(channel, min, max, |_, v| out.push(v));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Schema;
    use crate::layout::Cube16;
    use crate::storage::StorageKind;

    #[test]
    fn recordings_round_trip_and_report_the_first_difference() {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, 0u16);
        let mut volume = Volume::<Cube16>::new(schema);
        volume.set(block, IVec3::new(1, 2, 3), 9);
        let mut snap = Snapshot::new();
        volume.snapshot(block, IVec3::ZERO, 1, &mut snap);
        let mut a = Recording::new("mesh");
        a.push("input", record_snapshot(&snap))
            .push("visits", record_visits(&[IVec3::ONE, IVec3::ZERO]));
        let decoded = Recording::decode(&a.encode()).unwrap();
        assert_eq!(decoded, a);
        assert_eq!(decoded.compare(&a), None);
        let replayed: Snapshot<u16> = snapshot_from_bytes(a.section("input").unwrap()).unwrap();
        assert_eq!(replayed.at(1, 2, 3), 9);
        assert_eq!(record_snapshot(&replayed), a.section("input").unwrap());
        let mut b = a.clone();
        b.sections[1].1[5] ^= 1;
        assert_eq!(
            b.compare(&a),
            Some(Mismatch::Bytes {
                section: "visits".into(),
                offset: 5,
                expected_len: 24,
                actual_len: 24
            })
        );
        let mut c = a.clone();
        c.sections.pop();
        assert_eq!(
            c.compare(&a),
            Some(Mismatch::MissingSection("visits".into()))
        );
        assert_eq!(a.compare(&c), Some(Mismatch::ExtraSection("visits".into())));
        assert!(Recording::decode(b"nope").is_none());
        assert!(Recording::decode(&a.encode()[..10]).is_none());
        assert!(snapshot_from_bytes::<u16>(&[0; 12]).is_none());
    }
}
