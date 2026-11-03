//! CFastZombie (npc_fastzombie; Source SDK 2013 game/server/hl2/npc_fastzombie.cpp) on
//! top of the zombie AI (DESIGN 14o session 43 fast zombie plan).

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "needs the owned installation"]
    fn owned_fast_zombie_model_has_its_leap_claws_frenzy_and_climb() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let path = "models/zombie/fast.mdl";
        let names = source_assets::animation::sequence_names(&vfs, path).unwrap();
        let rig =
            source_assets::animation::load(&vfs, path, &names.iter().cloned().collect()).unwrap();
        let events = |activity: &str| {
            let s = rig.sequences.iter().find(|s| s.activity == activity);
            let s = s.unwrap_or_else(|| panic!("{activity}"));
            let clip = &rig.clips[&s.name];
            let events: Vec<_> = clip
                .events
                .iter()
                .map(|e| (e.name.clone(), e.cycle))
                .collect();
            println!(
                "{activity} {} {} frames {events:?}",
                s.name,
                clip.frames.len()
            );
            events
        };
        assert_eq!(
            events("ACT_RANGE_ATTACK1"),
            [("AE_FASTZOMBIE_LEAP".into(), 0.8)]
        );
        let claws = events("ACT_MELEE_ATTACK1");
        assert_eq!(claws[0].0, "AE_ZOMBIE_ATTACK_RIGHT");
        assert_eq!(claws[1].0, "AE_ZOMBIE_ATTACK_LEFT");
        let gallop = events("ACT_RUN");
        assert!(gallop.iter().any(|e| e.0 == "AE_FASTZOMBIE_GALLOP_LEFT"));
        for activity in [
            "ACT_FASTZOMBIE_LEAP_STRIKE",
            "ACT_FASTZOMBIE_LAND_RIGHT",
            "ACT_FASTZOMBIE_LAND_LEFT",
            "ACT_FASTZOMBIE_FRENZY",
            "ACT_FASTZOMBIE_BIG_SLASH",
            "ACT_CLIMB_UP",
            "ACT_CLIMB_DISMOUNT",
            "ACT_JUMP",
            "ACT_WALK",
            "ACT_IDLE",
        ] {
            events(activity);
        }
        let eye = source_assets::models::read_eye_position(&vfs, path).unwrap();
        assert_eq!(eye.z, 64.);
        // HL2 ships no fast zombie torso (an episodic model).
        assert!(
            source_assets::animation::sequence_names(&vfs, "models/zombie/fast_torso.mdl").is_err()
        );
    }
}
