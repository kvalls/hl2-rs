//! Plugin-owned placeholders; installed localization always takes precedence.
pub const PLACEHOLDER_STRINGS: &[(&str, &str)] = &[
    ("HL2RS_MotionBlurUltra", "Ultra"),
    ("HL2RS_Potato", "Potato"),
    (
        "HL2RS_StatusMotionBlur",
        "Advanced adds viewmodel animation blur. Ultra also reconstructs world/camera and projectile motion; GPU performance tuning is in progress.",
    ),
    ("HL2RS_AspectUltrawide21x9", "Ultrawide 21:9"),
    ("HL2RS_AspectUltrawide32x9", "Super Ultrawide 32:9"),
    ("HL2RS_AspectSquare1x1", "Square 1:1"),
    ("HL2RS_AspectPortrait3x4", "Portrait 3:4"),
    ("HL2RS_AspectPortrait10x16", "Portrait 10:16"),
    ("HL2RS_AspectPortrait9x16", "Portrait 9:16"),
    ("HL2RS_KeepVideoSettings", "Keep these display settings?"),
    ("HL2RS_VerticalFov", "Vertical FOV"),
    ("HL2RS_ViewmodelFov", "Viewmodel FOV"),
];
