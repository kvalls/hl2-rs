//! Lightweight MDL activity-name extraction without decoding bones, poses, or `.ani` data.

use super::{bytes, offset, relative, string};
use crate::{i32le, vpk::Vfs};
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;

const MAX_INCLUDE_DEPTH: usize = 8;
const MAX_INCLUDED_MODELS: usize = 64;
const MAX_MODEL_BYTES: usize = 64 * 1024 * 1024;
const MAX_SEQUENCES: usize = 4096;
const SEQUENCE_STRIDE: usize = 212;

/// `$includemodel` paths from the MDL header (offsets 336/340, stride 8), normalized to
/// lowercase `/`.
pub(super) fn include_model_paths(data: &[u8]) -> Result<Vec<String>> {
    let includes = offset(data, 336)?;
    if includes > MAX_INCLUDED_MODELS {
        bail!("included model table exceeds limit");
    }
    let base = offset(data, 340)?;
    (0..includes)
        .map(|id| {
            let at = base + id * 8;
            Ok(string(data, relative(data, at, at + 4)?)?
                .replace('\\', "/")
                .to_lowercase())
        })
        .collect()
}

#[derive(Default)]
struct ActivityState {
    seen: BTreeSet<String>,
    bytes: usize,
}

/// Collect all non-empty uppercased sequence activity names from `path` and its
/// `$includemodel`s without decoding bones, `.ani` files, pose frames, or autolayers.
pub fn activity_names(vfs: &Vfs, path: &str) -> Result<BTreeSet<String>> {
    let data = vfs.read(path)?.context("MDL missing for animation")?;
    let mut activities = BTreeSet::new();
    let mut state = ActivityState::default();
    collect_activities(vfs, path, &data, &mut activities, &mut state, 0)?;
    Ok(activities)
}

fn collect_activities(
    vfs: &Vfs,
    path: &str,
    data: &[u8],
    activities: &mut BTreeSet<String>,
    state: &mut ActivityState,
    depth: usize,
) -> Result<()> {
    if depth > MAX_INCLUDE_DEPTH {
        bail!("included animation model depth budget exceeded");
    }
    if !state.seen.insert(path.to_lowercase()) {
        return Ok(());
    }
    state.bytes = state
        .bytes
        .checked_add(data.len())
        .context("animation byte budget overflow")?;
    if state.seen.len() > MAX_INCLUDED_MODELS || state.bytes > MAX_MODEL_BYTES {
        bail!("included animation model budget exceeded");
    }
    let sequences = offset(data, 188)?;
    if sequences > MAX_SEQUENCES {
        bail!("sequence table too large");
    }
    let base = offset(data, 192)?;
    bytes(data, base, sequences * SEQUENCE_STRIDE)?;
    for id in 0..sequences {
        let at = base + id * SEQUENCE_STRIDE;
        if i32le(data, at + 8)? != 0 {
            let activity = string(data, relative(data, at, at + 8)?)?;
            if !activity.is_empty() {
                activities.insert(activity.to_uppercase());
            }
        }
    }
    for include in include_model_paths(data)? {
        if let Some(bytes) = vfs.read(&include)? {
            let _ = collect_activities(vfs, &include, &bytes, activities, state, depth + 1);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_mdl(activities: &[&str], includes: &[&str]) -> Vec<u8> {
        let header_size = 512usize;
        let seq_base = header_size;
        let inc_base = seq_base + activities.len() * SEQUENCE_STRIDE;
        let str_base = inc_base + includes.len() * 8;
        let mut data = vec![0u8; str_base];
        data[188..192].copy_from_slice(&(activities.len() as i32).to_le_bytes());
        data[192..196].copy_from_slice(&(seq_base as i32).to_le_bytes());
        data[336..340].copy_from_slice(&(includes.len() as i32).to_le_bytes());
        data[340..344].copy_from_slice(&(inc_base as i32).to_le_bytes());

        fn push_str(data: &mut Vec<u8>, s: &str) -> usize {
            let pos = data.len();
            data.extend_from_slice(s.as_bytes());
            data.push(0);
            pos
        }
        for (i, activity) in activities.iter().enumerate() {
            if !activity.is_empty() {
                let at = seq_base + i * SEQUENCE_STRIDE;
                let str_pos = push_str(&mut data, activity);
                let rel = (str_pos - at) as i32;
                data[at + 8..at + 12].copy_from_slice(&rel.to_le_bytes());
            }
        }
        for (i, include) in includes.iter().enumerate() {
            let at = inc_base + i * 8;
            let str_pos = push_str(&mut data, include);
            let rel = (str_pos - at) as i32;
            data[at + 4..at + 8].copy_from_slice(&rel.to_le_bytes());
        }
        data
    }

    #[test]
    fn synthetic_mdl_activities_extracts_includes_uppercases_and_validates_bounds() {
        let root = std::env::temp_dir().join(format!("hl2-rs-activities-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("hl2/models")).unwrap();

        let main = synthetic_mdl(
            &["act_idle", "", "ACT_WALK", "act_Run"],
            &["models\\Shared.mdl"],
        );
        let shared = synthetic_mdl(
            &["act_reload", "ACT_IDLE"],
            &["models/missing.mdl", "models/main.mdl"],
        );
        std::fs::write(root.join("hl2/models/main.mdl"), &main).unwrap();
        std::fs::write(root.join("hl2/models/shared.mdl"), &shared).unwrap();

        let vfs = Vfs::mount(&root).unwrap();
        assert_eq!(
            activity_names(&vfs, "models/main.mdl").unwrap(),
            BTreeSet::from([
                "ACT_IDLE".to_string(),
                "ACT_RELOAD".to_string(),
                "ACT_RUN".to_string(),
                "ACT_WALK".to_string(),
            ])
        );
        assert!(activity_names(&vfs, "models/does_not_exist.mdl").is_err());

        let mut too_many_seq = synthetic_mdl(&[], &[]);
        too_many_seq[188..192].copy_from_slice(&(MAX_SEQUENCES as i32 + 1).to_le_bytes());
        std::fs::write(root.join("hl2/models/bad_seq_count.mdl"), &too_many_seq).unwrap();
        assert!(activity_names(&vfs, "models/bad_seq_count.mdl").is_err());

        let mut truncated = synthetic_mdl(&["ACT_IDLE"], &[]);
        truncated.truncate(512 + 100);
        std::fs::write(root.join("hl2/models/truncated.mdl"), &truncated).unwrap();
        assert!(activity_names(&vfs, "models/truncated.mdl").is_err());

        let mut bad_offset = synthetic_mdl(&["ACT_IDLE"], &[]);
        bad_offset[512 + 8..512 + 12].copy_from_slice(&i32::MAX.to_le_bytes());
        std::fs::write(root.join("hl2/models/bad_off.mdl"), &bad_offset).unwrap();
        assert!(activity_names(&vfs, "models/bad_off.mdl").is_err());

        let mut too_many_inc = synthetic_mdl(&[], &[]);
        too_many_inc[336..340].copy_from_slice(&(MAX_INCLUDED_MODELS as i32 + 1).to_le_bytes());
        assert!(include_model_paths(&too_many_inc).is_err());

        let mut activities = BTreeSet::new();
        let mut state = ActivityState::default();
        assert!(collect_activities(
            &vfs,
            "models/main.mdl",
            &main,
            &mut activities,
            &mut state,
            MAX_INCLUDE_DEPTH + 1,
        )
        .is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_activity_names_match_full_rig_load() {
        let vfs = Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        for path in [
            "models/police.mdl",
            "models/combine_soldier.mdl",
            "models/combine_super_soldier.mdl",
            "models/combine_soldier_prisonguard.mdl",
            "models/humans/group01/male_01.mdl",
            "models/barney.mdl",
            "models/alyx.mdl",
            "models/monk.mdl",
        ] {
            let expected = crate::animation::load(&vfs, path, &BTreeSet::new())
                .unwrap()
                .activities;
            let actual = activity_names(&vfs, path).unwrap();
            assert_eq!(actual, expected, "{path}");
        }
    }
}
