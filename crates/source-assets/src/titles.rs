//! scripts/titles.txt: the client text messages env_message and game titles show (the fields
//! of the SDK's client_textmessage_t). `$` lines set the parameters of the messages after
//! them; a message is a name line, `{`, its text lines and `}`.
use std::collections::BTreeMap;

/// One text message: its drawing parameters and text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextMessage {
    /// 0 fade in/out, 1 fade in/out with flicker, 2 scan out one character at a time.
    pub effect: i32,
    pub color: [u8; 4],
    /// The scan highlight (effect 2).
    pub color2: [u8; 4],
    /// Screen fractions; -1 centers, other negatives align to the right/bottom.
    pub x: f32,
    pub y: f32,
    pub fadein: f32,
    pub fadeout: f32,
    pub holdtime: f32,
    pub fxtime: f32,
    /// The text, lines joined with `\n`.
    pub text: String,
}

impl Default for TextMessage {
    fn default() -> Self {
        Self {
            effect: 0,
            color: [255; 4],
            color2: [255; 4],
            x: -1.,
            y: -1.,
            fadein: 0.,
            fadeout: 0.,
            holdtime: 0.,
            fxtime: 0.,
            text: String::new(),
        }
    }
}

fn color(values: &[f32]) -> Option<[u8; 4]> {
    (values.len() >= 3).then(|| {
        let channel = |i: usize| values.get(i).map_or(255, |v| v.clamp(0., 255.) as u8);
        [channel(0), channel(1), channel(2), channel(3)]
    })
}

/// Every message by its name in upper case (TextMessageGet compares names without case).
pub fn parse(text: &str) -> BTreeMap<String, TextMessage> {
    let mut out = BTreeMap::new();
    let mut params = TextMessage::default();
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if let Some(command) = trimmed.strip_prefix('$') {
            let mut words = command.split_whitespace();
            let name = words.next().unwrap_or("").to_ascii_lowercase();
            let values: Vec<f32> = words.filter_map(|w| w.parse().ok()).collect();
            let first = values.first().copied();
            match (name.as_str(), first) {
                ("position", Some(x)) if values.len() >= 2 => (params.x, params.y) = (x, values[1]),
                ("effect", Some(v)) => params.effect = v as i32,
                ("fadein", Some(v)) => params.fadein = v,
                ("fadeout", Some(v)) => params.fadeout = v,
                ("holdtime", Some(v)) => params.holdtime = v,
                ("fxtime", Some(v)) => params.fxtime = v,
                ("color", _) => params.color = color(&values).unwrap_or(params.color),
                ("color2", _) => params.color2 = color(&values).unwrap_or(params.color2),
                _ => {}
            }
            continue;
        }
        // A message: its name, then `{` and the text up to `}`.
        if lines.by_ref().map(str::trim).find(|l| !l.is_empty()) != Some("{") {
            continue;
        }
        let body: Vec<&str> = lines.by_ref().take_while(|l| l.trim() != "}").collect();
        out.insert(
            trimmed.to_ascii_uppercase(),
            TextMessage {
                text: body.join("\n"),
                ..params.clone()
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_apply_to_the_messages_after_them() {
        let text = "// comment\r\n$position -1 0.58\r\n$effect 2\r\n$color 100 100 100\r\n\
                    $color2 240 110 0\r\n$fadein 0.01\r\n$fxtime 0.25\r\n$holdtime 3.5\r\n\
                    $fadeout 1.5\r\n\r\nCHAPTER1_TITLE\r\n{\r\n#HL2_Chapter1_Title\r\n}\r\n\
                    $holdtime 9\r\nTwo\r\n{\r\nfirst\r\nsecond\r\n}\r\n";
        let messages = parse(text);
        let title = &messages["CHAPTER1_TITLE"];
        assert_eq!(title.text, "#HL2_Chapter1_Title");
        assert_eq!((title.x, title.y, title.effect), (-1., 0.58, 2));
        assert_eq!(title.color, [100, 100, 100, 255]);
        assert_eq!(title.color2, [240, 110, 0, 255]);
        assert_eq!(
            (title.fadein, title.fxtime, title.holdtime),
            (0.01, 0.25, 3.5)
        );
        assert_eq!(title.fadeout, 1.5);
        let two = &messages["TWO"];
        assert_eq!((two.text.as_str(), two.holdtime), ("first\nsecond", 9.));
    }

    /// The installed titles.txt (owned file).
    #[test]
    #[ignore]
    fn owned_titles_hold_the_chapter_titles() {
        let root = crate::install::discover().expect("installation");
        let vfs = crate::vpk::Vfs::mount(&root).expect("vfs");
        let data = vfs.read("scripts/titles.txt").unwrap().expect("titles.txt");
        let messages = parse(&crate::keyvalues::decode_text(&data).unwrap());
        let title = &messages["CHAPTER1_TITLE"];
        assert_eq!(title.text, "#HL2_Chapter1_Title");
        assert_eq!((title.x, title.y, title.effect), (-1., 0.58, 2));
        assert_eq!(
            (title.fadein, title.holdtime, title.fadeout),
            (0.01, 3.5, 1.5)
        );
    }
}
