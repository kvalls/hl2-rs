//! Compiled caption dictionaries (`resource/closecaption_<language>.dat`, SDK
//! `captioncompiler.h`). A `VCCD` version 1 file is a 24-byte header (magic, version, block
//! count, block size, directory entry count, data offset), a directory of 12-byte entries
//! (CRC32 of the lower-cased token, block, offset in the block, byte length) and fixed-size
//! blocks of NUL-terminated UCS-2 text. Every search path may contribute a dictionary; the
//! first one holding a token wins (CHudCloseCaption::InitCaptionDictionary / AddToken).
//! Retail streams blocks asynchronously; here the whole file is decoded at load.
use crate::vpk::Vfs;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;

const MAGIC: &[u8; 4] = b"VCCD";
const VERSION: i32 = 1;
const HEADER: usize = 24;
const ENTRY: usize = 12;
/// The SDK rejects larger directories.
const MAX_ENTRIES: usize = 64 * 1024;

/// CaptionLookup_t::SetHash: CRC32 of the lower-cased token.
pub fn token_hash(token: &str) -> u32 {
    crc32fast::hash(token.to_ascii_lowercase().as_bytes())
}

/// One compiled dictionary: caption text by token hash.
#[derive(Clone, Debug, Default)]
pub struct Dictionary {
    entries: HashMap<u32, String>,
}

impl Dictionary {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < HEADER || &data[..4] != MAGIC {
            bail!("not a compiled caption file");
        }
        let version = crate::i32le(data, 4)?;
        if version != VERSION {
            bail!("unsupported compiled caption version {version}");
        }
        let blocks = usize::try_from(crate::i32le(data, 8)?).context("negative block count")?;
        let block_size = usize::try_from(crate::i32le(data, 12)?).context("negative block size")?;
        let count = usize::try_from(crate::i32le(data, 16)?).context("negative directory")?;
        let data_offset =
            usize::try_from(crate::i32le(data, 20)?).context("negative data offset")?;
        if count > MAX_ENTRIES {
            bail!("caption directory of {count} entries");
        }
        let directory = crate::bytes(data, HEADER, count * ENTRY)?;
        let mut entries = HashMap::with_capacity(count);
        for entry in crate::records(directory, ENTRY)? {
            let hash = crate::u32le(entry, 0)?;
            let block = usize::try_from(crate::i32le(entry, 4)?).context("negative block")?;
            let offset = usize::from(crate::u16le(entry, 8)?);
            let length = usize::from(crate::u16le(entry, 10)?);
            if block >= blocks || offset + length > block_size {
                bail!("caption entry {hash:08x} outside its block");
            }
            let start = data_offset + block * block_size + offset;
            let text = ucs2(crate::bytes(data, start, length)?);
            // The sorted SDK directory finds one entry per hash; keep the first.
            entries.entry(hash).or_insert(text);
        }
        Ok(Self { entries })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, token: &str) -> Option<&str> {
        self.entries
            .get(&token_hash(token))
            .or_else(|| self.entries.get(&token_hash(&token.replace('/', "\\"))))
            .or_else(|| self.entries.get(&token_hash(&token.replace('\\', "/"))))
            .map(String::as_str)
    }
}

/// UCS-2 little-endian text up to its NUL terminator.
fn ucs2(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|&unit| unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// The dictionaries of one game's search path, in search order.
#[derive(Clone, Debug, Default)]
pub struct Captions {
    pub dictionaries: Vec<Dictionary>,
    /// Copies that failed to decode, for diagnostics.
    pub errors: Vec<String>,
}

impl Captions {
    /// Every `resource/closecaption_<language>.dat` on the mounted search path.
    pub fn load(vfs: &Vfs, language: &str) -> Result<Self> {
        let path = format!("resource/closecaption_{language}.dat");
        let mut captions = Self::default();
        for (index, data) in vfs.read_all(&path)?.into_iter().enumerate() {
            match Dictionary::parse(&data) {
                Ok(dictionary) => captions.dictionaries.push(dictionary),
                Err(error) => captions.errors.push(format!("{path} #{index}: {error:#}")),
            }
        }
        Ok(captions)
    }

    /// The caption text of `token` from the first dictionary holding it.
    pub fn get(&self, token: &str) -> Option<&str> {
        self.dictionaries.iter().find_map(|d| d.get(token))
    }

    pub fn is_empty(&self) -> bool {
        self.dictionaries.iter().all(Dictionary::is_empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A compiled dictionary of `tokens` with one block per entry list chunk of `block_size`.
    fn compile(tokens: &[(&str, &str)], block_size: usize) -> Vec<u8> {
        let mut blocks: Vec<Vec<u8>> = vec![Vec::new()];
        let mut directory = Vec::new();
        for (token, text) in tokens {
            let mut bytes: Vec<u8> = text
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect();
            if blocks.last().unwrap().len() + bytes.len() > block_size {
                blocks.push(Vec::new());
            }
            let block = blocks.len() - 1;
            let offset = blocks[block].len();
            directory.push((token_hash(token), block, offset, bytes.len()));
            blocks[block].append(&mut bytes);
        }
        let data_offset = HEADER + directory.len() * ENTRY;
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        for value in [
            VERSION,
            blocks.len() as i32,
            block_size as i32,
            directory.len() as i32,
            data_offset as i32,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for (hash, block, offset, length) in directory {
            out.extend_from_slice(&hash.to_le_bytes());
            out.extend_from_slice(&(block as i32).to_le_bytes());
            out.extend_from_slice(&(offset as u16).to_le_bytes());
            out.extend_from_slice(&(length as u16).to_le_bytes());
        }
        for mut block in blocks {
            block.resize(block_size, 0);
            out.append(&mut block);
        }
        out
    }

    #[test]
    fn tokens_hash_case_insensitively_to_their_ucs2_text() {
        let data = compile(
            &[
                (
                    "ep_01.kl_isthison",
                    "<clr:255,255,200><I>Ahem...is this on?",
                ),
                ("HL2_Other", "Ünïcode — text"),
                ("third", "in another block"),
            ],
            96,
        );
        let dictionary = Dictionary::parse(&data).unwrap();
        assert_eq!(dictionary.len(), 3);
        assert_eq!(
            dictionary.get("EP_01.KL_IsThisOn"),
            Some("<clr:255,255,200><I>Ahem...is this on?")
        );
        assert_eq!(dictionary.get("hl2_other"), Some("Ünïcode — text"));
        assert_eq!(dictionary.get("third"), Some("in another block"));
        assert_eq!(dictionary.get("missing"), None);
        // CRC32 (IEEE) of the lower-cased bytes.
        assert_eq!(token_hash("ABC"), 0x352441c2);
    }

    #[test]
    fn the_first_dictionary_holding_a_token_wins() {
        let captions = Captions {
            dictionaries: vec![
                Dictionary::parse(&compile(&[("a", "episodic a")], 64)).unwrap(),
                Dictionary::parse(&compile(&[("a", "hl2 a"), ("b", "hl2 b")], 64)).unwrap(),
            ],
            errors: Vec::new(),
        };
        assert_eq!(captions.get("a"), Some("episodic a"));
        assert_eq!(captions.get("b"), Some("hl2 b"));
    }

    #[test]
    fn malformed_dictionaries_are_rejected() {
        assert!(Dictionary::parse(b"VCCD").is_err());
        let mut data = compile(&[("a", "text")], 64);
        data[4] = 2;
        assert!(Dictionary::parse(&data).is_err(), "version");
        let mut data = compile(&[("a", "text")], 64);
        // Entry offset past its block.
        data[HEADER + 8..HEADER + 10].copy_from_slice(&100u16.to_le_bytes());
        assert!(Dictionary::parse(&data).is_err(), "offset");
        let mut data = compile(&[("a", "text")], 64);
        data.truncate(HEADER + ENTRY + 4);
        assert!(Dictionary::parse(&data).is_err(), "truncated");
    }

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn owned_episode_one_dictionaries_follow_the_search_path() {
        let root = crate::install::discover().expect("installed game");
        let vfs = Vfs::mount_game(&root, "episodic").unwrap();
        let captions = Captions::load(&vfs, "english").unwrap();
        assert!(captions.errors.is_empty(), "{:?}", captions.errors);
        // Episode One's own dictionary, then Half-Life 2's.
        assert_eq!(captions.dictionaries.len(), 2);
        assert_eq!(captions.dictionaries[0].len(), 3153);
        assert_eq!(captions.dictionaries[1].len(), 3970);
        let line = captions.get("ep_01.kl_isthison").expect("Kleiner caption");
        assert!(
            line.starts_with("<clr:255,255,200><I>Ahem...is this on?"),
            "{line}"
        );
        // A Half-Life 2 token resolves from the inherited dictionary.
        assert!(captions.get("k_lab.kl_fiddlesticks").is_some());
    }

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn owned_kleiner_broadcast_lines_have_captions() {
        let root = crate::install::discover().expect("installed game");
        let vfs = Vfs::mount_game(&root, "episodic").unwrap();
        let captions = Captions::load(&vfs, "english").unwrap();
        let cache =
            crate::scenes::Cache::parse(vfs.read("scenes/scenes.image").unwrap().unwrap()).unwrap();
        let mut lines = 0;
        for n in 1..=13 {
            let path = format!("scenes/episode_1/c17/kleinercast{n:02}.vcd");
            let scene = cache.scene(&path).unwrap().expect(&path);
            for event in scene.events.iter().filter(|e| e.active()) {
                let Some(speech) = &event.speech else {
                    continue;
                };
                let token = if speech.caption_token.is_empty() {
                    &event.parameters[0]
                } else {
                    &speech.caption_token
                };
                println!(
                    "{path} {} type {} flags {:#x} token {token} {:.2}..{:?}",
                    event.parameters[0], speech.caption_type, speech.flags, event.start, event.end
                );
                assert!(captions.get(token).is_some(), "{path}: {token}");
                lines += 1;
            }
        }
        assert!(lines >= 13, "{lines}");
    }
}
