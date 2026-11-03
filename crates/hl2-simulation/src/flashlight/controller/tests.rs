use super::*;
use crate::flashlight::{SOUND_OFF, SOUND_ON};

fn context(now: f64, suit: bool) -> Context {
    Context {
        suit,
        now,
        ..Default::default()
    }
}

#[test]
fn suit_acquisition_unlocks_without_a_weapon_and_removal_blocks_reactivation() {
    for legacy in [true, false] {
        let mut player = PlayerFlashlight::default();
        assert!(!player.step(legacy, 1, true, None, context(0., false)).on);
        assert!(player.step(legacy, 1, true, None, context(1., true)).on);
        // SDK RemoveSuit does not turn the light off; the next activation needs a suit.
        assert!(player.step(legacy, 0, true, None, context(2., false)).on);
        assert!(!player.step(legacy, 1, true, None, context(3., false)).on);
        assert!(!player.step(legacy, 1, true, None, context(4., false)).on);
    }
}

#[test]
fn authored_disable_extinguishes_and_enable_only_restores_permission() {
    let mut player = PlayerFlashlight::default();
    player.step(true, 1, true, Some(1.), context(1., true));
    player.request_enabled(false);
    let state = player.step(true, 1, true, None, context(2., true));
    assert!(!state.on && state.disabled && !state.suit_power.active());
    assert_eq!(state.sounds, [SOUND_ON, SOUND_OFF]);
    player.request_enabled(true);
    assert!(!player.step(true, 0, true, None, context(3., true)).on);
    assert!(player.step(true, 1, true, None, context(4., true)).on);
}

#[test]
fn death_extinguishes_once_and_rejects_dead_impulses() {
    let mut player = PlayerFlashlight::default();
    player.step(false, 1, true, None, context(0., true));
    let state = player.step(false, 1, false, None, context(1., true));
    assert!(!state.on);
    assert_eq!(state.sounds, [SOUND_ON, SOUND_OFF]);
    player.step(false, 10, false, None, context(2., true));
    assert_eq!(player.state().unwrap().sounds, [SOUND_ON, SOUND_OFF]);
}

#[test]
fn pause_preserves_power_and_campaign_rebase_preserves_recharge_delay() {
    let mut player = PlayerFlashlight::default();
    let initial = player
        .step(true, 1, true, Some(1.), context(100., true))
        .power();
    assert_eq!(
        player
            .step(true, 0, true, None, context(100., true))
            .power(),
        initial
    );
    player.step(true, 1, true, None, context(100., true));
    let off = player.state().unwrap().power();
    player.rebase_clock(100., 0.);
    assert_eq!(
        player
            .step(true, 0, true, Some(0.25), context(0.25, true))
            .power(),
        off
    );
    assert!(
        player
            .step(true, 0, true, Some(0.5), context(0.75, true))
            .power()
            > off
    );
    assert!(
        PlayerFlashlight::default().state().is_none(),
        "a direct map load starts fresh"
    );
}

#[test]
fn authored_input_before_first_frame_uses_episode_twos_battery_mode() {
    let mut player = PlayerFlashlight::default();
    player.request_enabled(false);
    let state = player.step(false, 1, true, None, context(0., true));
    assert!(state.disabled && !state.on && state.battery == Some(100.));
}
