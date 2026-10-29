//! The viewmodel tweak commands only ask the host; it decides whether they apply.

use super::{Console, Effect};

#[test]
fn viewmodel_tweak_commands_reach_the_host() {
    let mut console = Console::default();
    assert_eq!(
        console.submit("viewmodel_tweak_mode 1; viewmodel_tweak_reset; viewmodel_tweak_mode 0"),
        vec![
            Effect::ViewmodelTweakMode(true),
            Effect::ViewmodelTweakReset,
            Effect::ViewmodelTweakMode(false)
        ]
    );
    assert!(console
        .submit("viewmodel_tweak_mode; viewmodel_tweak_mode 2; viewmodel_tweak_reset 1")
        .is_empty());
}
