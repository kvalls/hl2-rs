//! `script_intro`: the Half-Life 2 intro's view blends (SDK server/hl2/script_intro.cpp
//! `CScriptIntro`, client/hl2/c_script_intro.cpp `C_ScriptIntro`). While active, the view is
//! composed from up to two blend passes of `scripted/intro_screenspaceeffect` over two
//! pictures: the camera entity's view (`SetCameraViewEntity`, the shader's "scene") and the
//! ordinary view (the player's or its point_viewcontrol's, the shader's "gman"). The intro's
//! fade replaces the screen fade while its alpha is not zero.

/// The composition of one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct IntroFrame {
    /// The camera entity whose view is the shader's first picture (black without one).
    pub camera: Option<usize>,
    /// (blend mode 0..=9, alpha): passes added over black (SRC_ALPHA, ONE).
    pub passes: Vec<(i32, f32)>,
    /// The fade color (0..255) and alpha (0..255), drawn over the composite.
    pub fade: [f32; 4],
    /// The camera view's field of view (`SetFOV` and its blends), if set.
    pub camera_fov: Option<f32>,
}

/// The intro's presentation: the active script_intro and env_credits' logo.
#[derive(Clone, Debug)]
pub struct Intro {
    pub script: ScriptIntro,
    /// ShowLogo: when the logo was shown and how long it holds.
    pub logo: Option<(f64, f32)>,
    /// RollCredits: when the intro names started.
    pub credits: Option<f64>,
    /// env_starfield: on, and its density (CEnvStarfield: off, density 1).
    pub starfield: (bool, f32),
    /// The HUD text messages shown (env_message, game_text), with their start.
    pub messages: Vec<(HudText, f64)>,
}

/// A HUD text message: a titles.txt name (env_message, UTIL_ShowMessage) or game_text's own
/// parameters on a channel (UTIL_HudMessage).
#[derive(Clone, Debug, PartialEq)]
pub enum HudText {
    Title(String),
    Text {
        message: source_assets::titles::TextMessage,
        channel: i32,
    },
}

impl Intro {
    /// The starfield's density while it is on.
    pub fn starfield(&self) -> Option<f32> {
        self.starfield.0.then_some(self.starfield.1)
    }
}

impl Default for Intro {
    fn default() -> Self {
        Self {
            script: ScriptIntro::default(),
            logo: None,
            credits: None,
            starfield: (false, 1.),
            messages: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScriptIntro {
    active: bool,
    camera: Option<usize>,
    blend_mode: i32,
    next_blend_mode: i32,
    queued_blend_mode: i32,
    queued_next_blend_mode: i32,
    blend_start: f64,
    next_blend_time: f64,
    complete_at: Option<f64>,
    fov: i32,
    start_fov: i32,
    next_fov: i32,
    fov_blend_start: f64,
    next_fov_blend_time: f64,
    /// The latest FOV change was a blend (its end value holds after it).
    fov_blended: bool,
    /// `alternatefovchange`: spline FOV ramps.
    spline_fov: bool,
    fade_color: [f32; 3],
    fade_target: f32,
    fade_duration: f32,
    fade_started: f64,
    fade_from: f32,
}

impl Default for ScriptIntro {
    fn default() -> Self {
        Self {
            active: false,
            camera: None,
            blend_mode: 0,
            next_blend_mode: -1,
            queued_blend_mode: -1,
            queued_next_blend_mode: -1,
            blend_start: 0.,
            next_blend_time: 0.,
            complete_at: None,
            fov: 0,
            start_fov: 0,
            next_fov: 0,
            fov_blend_start: 0.,
            next_fov_blend_time: 0.,
            fov_blended: false,
            spline_fov: false,
            fade_color: [0.; 3],
            fade_target: 0.,
            fade_duration: 0.,
            fade_started: 0.,
            fade_from: 0.,
        }
    }
}

/// SimpleSplineRemapVal over 0..1.
fn spline(t: f32) -> f32 {
    t * t * (3. - 2. * t)
}

impl ScriptIntro {
    /// An input to the intro at `time`; `camera` is the entity SetCameraViewEntity names and
    /// `player_fov` the player's field of view. Returns false for inputs it does not take.
    pub fn input(
        &mut self,
        input: &str,
        parameter: &str,
        time: f64,
        camera: Option<usize>,
        player_fov: f32,
    ) -> bool {
        self.tick(time);
        let number = || parameter.trim().parse::<f32>().unwrap_or(0.);
        match input {
            "activate" => self.active = true,
            "deactivate" => self.active = false,
            "setcameraviewentity" => {
                if let Some(id) = camera {
                    self.camera = Some(id);
                }
            }
            "setblendmode" => {
                self.blend_mode = number() as i32;
                self.next_blend_mode = self.blend_mode;
                self.blend_start = time;
                self.next_blend_time = time;
                self.queued_blend_mode = -1;
                self.complete_at = None;
            }
            "setnextblendmode" => self.queued_next_blend_mode = number() as i32,
            "setnextblendtime" => {
                self.next_blend_time = time + f64::from(number());
                self.blend_start = time;
                if self.queued_blend_mode >= 0 {
                    self.blend_mode = self.queued_blend_mode;
                }
                if self.queued_next_blend_mode < 0 {
                    self.queued_next_blend_mode = self.blend_mode;
                }
                self.next_blend_mode = self.queued_next_blend_mode;
                self.queued_next_blend_mode = -1;
                self.queued_blend_mode = self.next_blend_mode;
                self.complete_at = Some(self.next_blend_time);
            }
            "setnextfov" => self.next_fov = number() as i32,
            "setfov" => {
                self.fov = number() as i32;
                self.start_fov = self.fov;
                self.fov_blended = false;
            }
            "setfovblendtime" => {
                self.start_fov = if self.next_fov_blend_time >= time {
                    self.blend_fov(time) as i32
                } else if self.fov != 0 {
                    self.fov
                } else {
                    player_fov as i32
                };
                self.next_fov_blend_time = time + f64::from(number());
                self.fov_blend_start = time;
                self.fov_blended = true;
            }
            "fadeto" => {
                let mut values = parameter
                    .split_whitespace()
                    .filter_map(|v| v.parse::<f32>().ok());
                let (Some(alpha), Some(duration)) = (values.next(), values.next()) else {
                    return true;
                };
                if alpha != self.fade_target {
                    // C_ScriptIntro::PostDataUpdate: a new target fades from the alpha shown now.
                    self.fade_from = self.fade_alpha(time);
                    self.fade_started = time;
                }
                self.fade_target = alpha;
                self.fade_duration = if duration == 0. { 0.01 } else { duration };
            }
            "setfadecolor" => {
                let values: Vec<f32> = parameter
                    .split_whitespace()
                    .filter_map(|v| v.parse().ok())
                    .collect();
                if let [r, g, b] = values[..] {
                    self.fade_color = [r, g, b];
                }
            }
            _ => return false,
        }
        true
    }

    /// The server's BlendComplete think when its time came.
    pub fn tick(&mut self, time: f64) {
        if self.complete_at.is_some_and(|t| t <= time) {
            self.blend_mode = self.next_blend_mode;
            self.blend_start = time;
            self.next_blend_time = time;
            self.queued_blend_mode = -1;
            self.complete_at = None;
        }
    }

    /// ScriptInfo_CalculateFOV.
    fn blend_fov(&self, time: f64) -> f32 {
        let span = self.next_fov_blend_time - self.fov_blend_start;
        let t = if span != 0. {
            ((time - self.fov_blend_start) / span) as f32
        } else {
            1.
        };
        let (from, to) = (self.start_fov as f32, self.next_fov as f32);
        if self.spline_fov {
            return if t >= 1. {
                to
            } else {
                from + (to - from) * spline(t.max(0.))
            };
        }
        from + (to - from) * t.clamp(0., 1.)
    }

    fn fade_alpha(&self, time: f64) -> f32 {
        let t = ((time - self.fade_started) / f64::from(self.fade_duration.max(0.01))) as f32;
        self.fade_from + (self.fade_target - self.fade_from) * t.clamp(0., 1.)
    }

    /// The frame at `time` while the intro is active (C_ScriptIntro::ClientThink).
    pub fn frame(&self, time: f64) -> Option<IntroFrame> {
        if !self.active {
            return None;
        }
        let span = self.next_blend_time - self.blend_start;
        let perc = if span != 0. {
            ((time - self.blend_start) / span).clamp(0., 1.) as f32
        } else {
            1.
        };
        let passes = if self.next_blend_time > time && perc < 1. {
            vec![(self.blend_mode, 1. - perc), (self.next_blend_mode, perc)]
        } else {
            vec![(
                if perc >= 1. && span != 0. {
                    self.next_blend_mode
                } else {
                    self.blend_mode
                },
                1.,
            )]
        };
        let camera_fov = if self.fov_blended && time <= self.next_fov_blend_time {
            Some(self.blend_fov(time))
        } else if self.fov_blended {
            Some(self.next_fov as f32)
        } else {
            (self.fov != 0).then_some(self.fov as f32)
        };
        let [r, g, b] = self.fade_color;
        Some(IntroFrame {
            camera: self.camera,
            passes,
            fade: [r, g, b, self.fade_alpha(time)],
            camera_fov,
        })
    }

    pub fn set_spline_fov(&mut self, spline: bool) {
        self.spline_fov = spline;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intro_blends_fades_and_fovs_follow_the_sdk() {
        let mut intro = ScriptIntro::default();
        let none = None;
        assert!(intro.frame(0.).is_none(), "inactive");
        intro.input("activate", "", 0., none, 75.);
        intro.input("fadeto", "255 0.01", 0., none, 75.);
        assert_eq!(intro.frame(1.).unwrap().fade[3], 255.);
        // scene1_start: FadeTo 0 over 3 s from 10 s.
        intro.input("fadeto", "0 3", 10., none, 75.);
        assert!((intro.frame(11.5).unwrap().fade[3] - 127.5).abs() < 0.01);
        // SetBlendMode 6, then SetNextBlendMode 3 and SetNextBlendTime 4: a 4 s crossfade.
        intro.input("setblendmode", "6", 22.4, none, 75.);
        intro.input("setnextblendmode", "3", 29.0, none, 75.);
        intro.input("setnextblendtime", "4", 29.1, none, 75.);
        assert_eq!(intro.frame(31.1).unwrap().passes, vec![(6, 0.5), (3, 0.5)]);
        intro.tick(33.1);
        assert_eq!(intro.frame(34.).unwrap().passes, vec![(3, 1.)]);
        // SetFOV 170, then a SetNextFOV 75 blend over 8.2 s that holds its end value.
        intro.input("setcameraviewentity", "camera_train", 50.5, Some(7), 75.);
        intro.input("setfov", "170", 50.5, none, 75.);
        intro.input("setnextfov", "75", 50.6, none, 75.);
        intro.input("setfovblendtime", "8.2", 52.11, none, 75.);
        let frame = intro.frame(56.21).unwrap();
        assert_eq!(frame.camera, Some(7));
        assert!(
            (frame.camera_fov.unwrap() - 122.5).abs() < 0.1,
            "{:?}",
            frame.camera_fov
        );
        assert_eq!(intro.frame(70.).unwrap().camera_fov, Some(75.));
        intro.input("deactivate", "", 61., none, 75.);
        assert!(intro.frame(62.).is_none());
    }
}
