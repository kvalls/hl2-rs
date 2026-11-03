use super::AdvancedChoice;
use crate::config::{AdvancedVideoSettings, Config, ShaderDetail, VideoSettings};
use crate::extensions::{RenderBudget, VideoExtension, VideoPolicy};

const KEY: &str = "fake_below";
const BUDGET: RenderBudget = RenderBudget {
    flat_sky: Some([0.5; 3]),
    nearest: true,
    plain_materials: true,
    no_shadows: true,
    extra_picmip: 1,
    extra_root_lod: 1,
};

#[derive(Debug)]
struct Fake;
impl VideoExtension for Fake {
    fn render_budget(&self, video: &VideoSettings) -> RenderBudget {
        if self.is_below_lowest(&video.advanced, AdvancedChoice::ShaderDetail) {
            BUDGET
        } else {
            RenderBudget::default()
        }
    }
    fn below_lowest_token(&self, choice: AdvancedChoice) -> Option<&'static str> {
        (choice == AdvancedChoice::ShaderDetail).then_some("#fake")
    }
    fn is_below_lowest(&self, video: &AdvancedVideoSettings, _: AdvancedChoice) -> bool {
        video.extensions.bool(KEY).unwrap_or(false)
    }
    fn set_below_lowest(&self, video: &mut AdvancedVideoSettings, _: AdvancedChoice, on: bool) {
        video.extensions.set_bool(KEY, on);
    }
}

fn fake() -> Config {
    let mut config = Config::default();
    config.video.policy = VideoPolicy::new(Fake);
    config.video.advanced.policy = config.video.policy.clone();
    config
}

#[test]
fn a_policy_item_takes_the_first_row_and_shifts_the_others() {
    let mut config = fake();
    let video = &mut config.video.advanced;
    let choice = AdvancedChoice::ShaderDetail;
    assert_eq!(choice.selected(video), 2);
    choice.select(video, 0);
    assert_eq!(choice.selected(video), 0);
    assert_eq!(video.shader_detail, ShaderDetail::Low);
    choice.select(video, 1);
    assert_eq!(choice.selected(video), 1);
    assert_eq!(video.shader_detail, ShaderDetail::Low);
    choice.select(video, 3);
    assert_eq!(video.shader_detail, ShaderDetail::VeryHigh);
    assert_eq!(AdvancedChoice::WaterDetail.selected(video), 1);
}

#[test]
fn a_budget_reaches_load_settings_and_reloads_only_for_what_loading_uses() {
    let mut config = fake();
    let plain = config.video.load_detail();
    let (root_lod, picmip) = (config.video.root_lod(), config.video.picmip());
    AdvancedChoice::ShaderDetail.select(&mut config.video.advanced, 0);
    assert_eq!(config.video.render_budget(), BUDGET);
    assert_eq!(config.video.root_lod(), root_lod + 1);
    assert_eq!(config.video.picmip(), picmip + 1);
    assert_ne!(config.video.load_detail(), plain);
    assert_eq!(config.video.load_detail().3.flat_sky, None);
    assert!(!config.video.load_detail().3.no_shadows);
}

#[test]
fn without_a_policy_the_rows_and_budget_are_the_retail_ones() {
    let mut config = Config::default();
    let video = &mut config.video.advanced;
    AdvancedChoice::ShaderDetail.select(video, 0);
    assert_eq!(video.shader_detail, ShaderDetail::Low);
    assert_eq!(AdvancedChoice::ShaderDetail.selected(video), 0);
    assert_eq!(config.video.render_budget(), RenderBudget::default());
}
