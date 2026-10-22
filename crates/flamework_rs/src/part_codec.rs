//! [`FwFilePart`] as bytes, for the compiler's incremental state on disk (`Plugin::encode_file_part` /
//! `decode_file_part`).
//!
//! **Format, version 2.** All integers little-endian.
//! - header: the magic `FWPT`, then the format version as a `u16` ([`FORMAT_VERSION`]);
//! - a string: its UTF-8 length as a `u32`, then the bytes; a list: its length as a `u32`, then the items;
//! - the part: `file_id`; `new_ids` (each `internal_id`, `name`, `file_name`); `late_keys`; `events` (each a tag byte,
//!   `0` a glob use: `glob`, `text`, `macro_name`, `line`, `column`; `1` a path use: `path`, `macro_name`, `line`,
//!   `column`, `raises` as a byte `0` / `1`); `unported` (each `what`, `milestone`, then `pos` as `offset`, `line`,
//!   `character`); `env_inputs` (each `name`, `hash`, in the part's order: `inputs.env` keeps the TS transformer's
//!   order). Numbers are `u32`.
//!
//! Version 1 (before `env_inputs`) is not read: a version 1 part decodes to `None`, so the compiler transforms the
//! file again.
//!
//! Decoding rejects (`None`) another magic or version, a truncated input, bytes left over, invalid UTF-8, an unknown
//! tag or flag, and a milestone this version does not name. It never panics.
//!
//! **Changing [`FwFilePart`]** (a field added, removed or retyped) changes the format: bump [`FORMAT_VERSION`]. The
//! encoder destructures every struct it writes, so a new field does not compile until it is encoded, and the golden
//! test pins the bytes of the current version.

use rbxts_plugin::Utf16Pos;

use crate::build_info::{GlobUse, PathUse};
use crate::ids::NewId;
use crate::plugin::{FwFilePart, PathEvent, Unported};

const MAGIC: &[u8; 4] = b"FWPT";

/// The version of the byte format; bumped on every change of what is written.
pub const FORMAT_VERSION: u16 = 2;

/// The milestones an [`Unported`] site can name; decoding maps the text back to these.
const MILESTONES: [&str; 10] = ["F0", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9"];

/// The part as bytes. `None` when it cannot be written: a milestone this format does not name, or a string or list
/// longer than `u32::MAX`.
pub fn encode(part: &FwFilePart) -> Option<Vec<u8>> {
    let FwFilePart { file_id, new_ids, late_keys, events, unported, env_inputs } = part;
    let mut w = Writer(Vec::with_capacity(64));
    w.0.extend_from_slice(MAGIC);
    w.0.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    w.string(file_id)?;
    w.len(new_ids.len())?;
    for NewId { internal_id, name, file_name } in new_ids {
        w.string(internal_id)?;
        w.string(name)?;
        w.string(file_name)?;
    }
    w.len(late_keys.len())?;
    for key in late_keys {
        w.string(key)?;
    }
    w.len(events.len())?;
    for event in events {
        match event {
            PathEvent::Glob(GlobUse { glob, text, macro_name, line, column }) => {
                w.0.push(0);
                w.string(glob)?;
                w.string(text)?;
                w.string(macro_name)?;
                w.u32(*line);
                w.u32(*column);
            }
            PathEvent::Path(PathUse { path, macro_name, line, column, raises }) => {
                w.0.push(1);
                w.string(path)?;
                w.string(macro_name)?;
                w.u32(*line);
                w.u32(*column);
                w.0.push(u8::from(*raises));
            }
        }
    }
    w.len(unported.len())?;
    for Unported { what, milestone, pos } in unported {
        if !MILESTONES.contains(milestone) {
            return None;
        }
        w.string(what)?;
        w.string(milestone)?;
        let Utf16Pos { offset, line, character } = *pos;
        w.u32(offset);
        w.u32(line);
        w.u32(character);
    }
    w.len(env_inputs.len())?;
    for (name, hash) in env_inputs {
        w.string(name)?;
        w.string(hash)?;
    }
    Some(w.0)
}

/// The part [`encode`] wrote, or `None` for anything else.
pub fn decode(bytes: &[u8]) -> Option<FwFilePart> {
    let mut r = Reader(bytes);
    if r.take(4)? != MAGIC || u16::from_le_bytes(r.take(2)?.try_into().ok()?) != FORMAT_VERSION {
        return None;
    }
    let file_id = r.string()?;
    let new_ids = r.list(|r| Some(NewId { internal_id: r.string()?, name: r.string()?, file_name: r.string()? }))?;
    let late_keys = r.list(Reader::string)?;
    let events = r.list(|r| match r.byte()? {
        0 => Some(PathEvent::Glob(GlobUse {
            glob: r.string()?,
            text: r.string()?,
            macro_name: r.string()?,
            line: r.u32()?,
            column: r.u32()?,
        })),
        1 => Some(PathEvent::Path(PathUse {
            path: r.string()?,
            macro_name: r.string()?,
            line: r.u32()?,
            column: r.u32()?,
            raises: match r.byte()? {
                0 => false,
                1 => true,
                _ => return None,
            },
        })),
        _ => None,
    })?;
    let unported = r.list(|r| {
        let what = r.string()?;
        let milestone = r.string()?;
        let milestone = *MILESTONES.iter().find(|m| **m == milestone)?;
        let pos = Utf16Pos { offset: r.u32()?, line: r.u32()?, character: r.u32()? };
        Some(Unported { what, milestone, pos })
    })?;
    let env_inputs = r.list(|r| Some((r.string()?, r.string()?)))?;
    r.0.is_empty().then_some(FwFilePart { file_id, new_ids, late_keys, events, unported, env_inputs })
}

struct Writer(Vec<u8>);

impl Writer {
    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn len(&mut self, len: usize) -> Option<()> {
        self.u32(u32::try_from(len).ok()?);
        Some(())
    }

    fn string(&mut self, text: &str) -> Option<()> {
        self.len(text.len())?;
        self.0.extend_from_slice(text.as_bytes());
        Some(())
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn string(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }

    /// A counted list. The count is not trusted for allocation: each item must still be read.
    fn list<T>(&mut self, mut item: impl FnMut(&mut Self) -> Option<T>) -> Option<Vec<T>> {
        let count = self.u32()? as usize;
        let mut items = Vec::with_capacity(count.min(self.0.len()));
        for _ in 0..count {
            items.push(item(self)?);
        }
        Some(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rich() -> FwFilePart {
        FwFilePart {
            file_id: "src/server/main.server.ts".into(),
            new_ids: vec![
                NewId {
                    internal_id: "src/server/main.server@Service".into(),
                    name: "Service".into(),
                    file_name: "main.server".into(),
                },
                NewId { internal_id: "".into(), name: "ünïcödé ✓ 𝄞".into(), file_name: "index".into() },
            ],
            late_keys: vec!["src/server/main.server@Service".into(), "k\0with\nodd\u{feff}chars".into()],
            events: vec![
                PathEvent::Glob(GlobUse {
                    glob: "src/server/components/**/*.ts".into(),
                    text: "./components/**/*.ts".into(),
                    macro_name: "registerProvidersGlob".into(),
                    line: 12,
                    column: 3,
                }),
                PathEvent::Path(PathUse {
                    path: "src/server/services".into(),
                    macro_name: "registerProviders".into(),
                    line: 1,
                    column: 1,
                    raises: false,
                }),
                PathEvent::Path(PathUse {
                    path: "src/shared".into(),
                    macro_name: "requireModules".into(),
                    line: u32::MAX,
                    column: 0,
                    raises: true,
                }),
            ],
            unported: vec![
                Unported {
                    what: "Dependency: a Modding.Generic<T, \"dependency\"> parameter".into(),
                    milestone: "F2",
                    pos: Utf16Pos { offset: 120, line: 4, character: 8 },
                },
                Unported { what: String::new(), milestone: "F6", pos: Utf16Pos::default() },
            ],
            env_inputs: vec![
                ("ZETA".into(), "5f1c0e7d-1111-5aaa-8bbb-000000000001".into()),
                ("ALPHA".into(), "5f1c0e7d-2222-5aaa-8bbb-000000000002".into()),
                ("ünï ".into(), String::new()),
            ],
        }
    }

    #[test]
    fn round_trips() {
        for part in [rich(), FwFilePart::default()] {
            let bytes = encode(&part).expect("encodes");
            assert_eq!(decode(&bytes), Some(part));
        }
    }

    #[test]
    fn round_trips_through_the_plugin() {
        use rbxts_plugin::Plugin;
        let plugin = crate::Flamework::new();
        let part = rich();
        let bytes = plugin.encode_file_part(&part).expect("persistable");
        assert_eq!(plugin.decode_file_part(&bytes), Some(part));
        assert_eq!(plugin.decode_file_part(b"not a part"), None);
    }

    #[test]
    fn version_2_bytes_are_stable() {
        let part = FwFilePart {
            file_id: "a".into(),
            new_ids: vec![NewId { internal_id: "i".into(), name: "n".into(), file_name: "f".into() }],
            late_keys: vec!["k".into()],
            events: vec![
                PathEvent::Glob(GlobUse {
                    glob: "g".into(),
                    text: "t".into(),
                    macro_name: "m".into(),
                    line: 2,
                    column: 3,
                }),
                PathEvent::Path(PathUse {
                    path: "p".into(),
                    macro_name: "m".into(),
                    line: 4,
                    column: 5,
                    raises: true,
                }),
            ],
            unported: vec![Unported {
                what: "w".into(),
                milestone: "F3",
                pos: Utf16Pos { offset: 6, line: 7, character: 8 },
            }],
            env_inputs: vec![("Z".into(), "h1".into()), ("A".into(), "h2".into())],
        };
        let s = |t: &str| [(t.len() as u32).to_le_bytes().as_slice(), t.as_bytes()].concat();
        let n = |v: u32| v.to_le_bytes().to_vec();
        let expected: Vec<u8> = [
            b"FWPT".to_vec(),
            2u16.to_le_bytes().to_vec(),
            s("a"),
            n(1),
            s("i"),
            s("n"),
            s("f"),
            n(1),
            s("k"),
            n(2),
            vec![0],
            s("g"),
            s("t"),
            s("m"),
            n(2),
            n(3),
            vec![1],
            s("p"),
            s("m"),
            n(4),
            n(5),
            vec![1],
            n(1),
            s("w"),
            s("F3"),
            n(6),
            n(7),
            n(8),
            n(2),
            s("Z"),
            s("h1"),
            s("A"),
            s("h2"),
        ]
        .concat();
        assert_eq!(encode(&part), Some(expected.clone()));
        assert_eq!(decode(&expected), Some(part));
    }

    #[test]
    fn version_1_parts_are_not_read() {
        // A version 1 part as its encoder wrote it (no `env_inputs`): not persistable now, so the file is transformed
        // again. The same bytes with a version 2 header are short by the `env_inputs` count.
        let s = |t: &str| [(t.len() as u32).to_le_bytes().as_slice(), t.as_bytes()].concat();
        let n = |v: u32| v.to_le_bytes().to_vec();
        let body = [s("a"), n(0), n(0), n(0), n(0)].concat();
        let v1 = [b"FWPT".to_vec(), 1u16.to_le_bytes().to_vec(), body.clone()].concat();
        assert_eq!(decode(&v1), None);
        let v2_short = [b"FWPT".to_vec(), 2u16.to_le_bytes().to_vec(), body.clone()].concat();
        assert_eq!(decode(&v2_short), None);
        let v2 = [v2_short, n(0)].concat();
        assert_eq!(decode(&v2), Some(FwFilePart { file_id: "a".into(), ..FwFilePart::default() }));
    }

    #[test]
    fn env_inputs_keep_their_order() {
        let order = ["Z", "B", "A", "B2", "A"];
        let part = FwFilePart {
            env_inputs: order.iter().map(|n| ((*n).to_owned(), format!("h{n}"))).collect(),
            ..FwFilePart::default()
        };
        let back = decode(&encode(&part).unwrap()).unwrap();
        assert_eq!(back.env_inputs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), order);
        assert_eq!(back, part);
    }

    #[test]
    fn rejects_other_versions_and_damaged_input() {
        let bytes = encode(&rich()).unwrap();
        // every truncation
        for end in 0..bytes.len() {
            assert_eq!(decode(&bytes[..end]), None, "truncated at {end}");
        }
        // bytes left over
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(decode(&longer), None);
        // another version, another magic
        let mut other = bytes.clone();
        other[4..6].copy_from_slice(&3u16.to_le_bytes());
        assert_eq!(decode(&other), None);
        let mut magic = bytes.clone();
        magic[0] = b'X';
        assert_eq!(decode(&magic), None);
        // an unknown event tag, a bad flag, invalid UTF-8, an unknown milestone
        let small = |event: Vec<u8>| {
            let mut b = encode(&FwFilePart::default()).unwrap();
            b.truncate(b.len() - 12); // the empty events, unported and env_inputs counts
            b.extend_from_slice(&1u32.to_le_bytes());
            b.extend(event);
            b.extend_from_slice(&0u32.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes());
            b
        };
        let one = |t: &str| [(t.len() as u32).to_le_bytes().as_slice(), t.as_bytes()].concat();
        assert_eq!(decode(&small(vec![2])), None, "unknown tag");
        let path = |flag: u8| [vec![1], one("p"), one("m"), vec![0; 8], vec![flag]].concat();
        assert!(decode(&small(path(1))).is_some(), "the fixture itself decodes");
        assert_eq!(decode(&small(path(2))), None, "bad flag");
        let mut bad_utf8 = encode(&FwFilePart { file_id: "ab".into(), ..FwFilePart::default() }).unwrap();
        bad_utf8[10] = 0xff;
        assert_eq!(decode(&bad_utf8), None, "invalid UTF-8");
        let mut unknown = encode(&FwFilePart {
            unported: vec![Unported { what: String::new(), milestone: "F2", pos: Utf16Pos::default() }],
            ..FwFilePart::default()
        })
        .unwrap();
        let at = unknown.len() - 4 - 12 - 1; // the milestone's last byte, before the position and the env count
        unknown[at] = b'X';
        assert_eq!(decode(&unknown), None, "unknown milestone");
        // a huge count does not allocate up front
        let mut huge = encode(&FwFilePart::default()).unwrap();
        huge[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&huge), None);
    }

    #[test]
    fn an_unknown_milestone_is_not_persistable() {
        let part = FwFilePart {
            unported: vec![Unported { what: "x".into(), milestone: "F10", pos: Utf16Pos::default() }],
            ..FwFilePart::default()
        };
        assert_eq!(encode(&part), None);
    }
}
