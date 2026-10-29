//! Console completion: what the token being typed can become, ranked, and a help line for the
//! command being typed.
//!
//! Completion works on the line's last command (after the last `;` outside quotes). The first
//! word completes from every implemented command and cvar; later words from what that argument
//! takes ([`Arg`]): one of the app's lists of names, command names or fixed values. Matches rank
//! by prefix, then substring, then the letters in order (closest first); the retail console
//! offers prefix matches only.

use super::command::{self, Arg, Context, Spec};

/// What a line being typed can complete to: the app's commands and the names its arguments
/// complete from.
pub trait Completer {
    fn complete(&self, line: &str) -> Completion;
}

/// A completion: the text it becomes and, for commands and cvars, their description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub text: String,
    pub detail: String,
}

/// The candidates for a line, and the line before the token they replace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completion {
    pub head: String,
    pub items: Vec<Candidate>,
}

/// Byte index where the line's last command starts (after the last `;` outside quotes).
pub fn last_command_start(line: &str) -> usize {
    let mut quoted = false;
    let mut start = 0;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => start = i + 1,
            _ => {}
        }
    }
    start
}

/// The command word and the index of the argument being typed in `command` (one command):
/// `None` while the command word itself is typed.
fn argument(command: &str) -> Option<(&str, usize)> {
    let words: Vec<&str> = command.split_whitespace().collect();
    let ends_space = command.ends_with(char::is_whitespace);
    match (words.first(), words.len()) {
        (None, _) => None,
        (Some(_), 1) if !ends_space => None,
        (Some(name), n) => Some((name, if ends_space { n - 1 } else { n - 2 })),
    }
}

/// Fuzzy rank, lower is better: the exact name, then prefix matches (alphabetical among
/// themselves, as retail lists them), then substring matches by position, then the query's
/// letters in order by the letters skipped; `None` if no match.
pub fn score(name: &str, query: &str) -> Option<i32> {
    let (name, query) = (name.to_lowercase(), query.to_lowercase());
    if query.is_empty() {
        return Some(1000);
    }
    if name == query {
        return Some(0);
    }
    if name.starts_with(&query) {
        return Some(1);
    }
    if let Some(at) = name.find(&query) {
        return Some(100 + at as i32);
    }
    let mut letters = name.chars();
    let mut skipped = 0;
    for wanted in query.chars() {
        loop {
            match letters.next() {
                Some(c) if c == wanted => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
    }
    Some(300 + skipped)
}

fn names(list: &[String]) -> Vec<Candidate> {
    list.iter()
        .map(|name| Candidate {
            text: name.clone(),
            detail: String::new(),
        })
        .collect()
}

fn spec_candidate<C: Context>(spec: &Spec<C>) -> Candidate {
    Candidate {
        text: spec.name.to_owned(),
        detail: command::usage(spec).1,
    }
}

/// What the token being typed in `line` can become, best first: `commands` for the first word,
/// the app's `lists` for arguments that complete from names.
pub fn candidates<'a, C: Context>(
    line: &str,
    commands: &'static [Spec<C>],
    lists: impl Fn(C::Names) -> &'a [String],
) -> Completion {
    let start = last_command_start(line);
    let command = &line[start..];
    let lead = command.len() - command.trim_start().len();
    let (head, token, pool) = match argument(command) {
        None => (
            &line[..start + lead],
            command.trim(),
            commands.iter().map(spec_candidate).collect(),
        ),
        Some((name, index)) => {
            let token = if command.ends_with(char::is_whitespace) {
                ""
            } else {
                command.split_whitespace().last().unwrap_or("")
            };
            let pool = match command::find(commands, name).and_then(|spec| spec.args.get(index)) {
                Some(Arg::Names(list)) => names(lists(*list)),
                Some(Arg::Command) => commands.iter().map(spec_candidate).collect(),
                Some(Arg::Values(values)) => values
                    .iter()
                    .map(|v| Candidate {
                        text: (*v).to_owned(),
                        detail: String::new(),
                    })
                    .collect(),
                Some(Arg::Free) | None => Vec::new(),
            };
            (&line[..line.len() - token.len()], token, pool)
        }
    };
    let mut ranked: Vec<(i32, Candidate)> = pool
        .into_iter()
        .filter_map(|c| score(&c.text, token).map(|s| (s, c)))
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.text.cmp(&b.1.text)));
    ranked.dedup_by(|a, b| a.1.text.eq_ignore_ascii_case(&b.1.text));
    Completion {
        head: head.to_owned(),
        items: ranked.into_iter().map(|(_, c)| c).collect(),
    }
}

/// The help line for the command being typed: its usage with the argument being typed marked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgHelp {
    pub name: &'static str,
    pub args: Vec<String>,
    /// The argument being typed, if the cursor is on one the command takes.
    pub current: Option<usize>,
    pub description: String,
    pub cvar: bool,
}

/// The help line for `line`'s last command, once its command word names one of `commands`.
pub fn arg_help<C: Context>(line: &str, commands: &'static [Spec<C>]) -> Option<ArgHelp> {
    let command = &line[last_command_start(line)..];
    let name = command.split_whitespace().next()?;
    let spec = command::find(commands, name)?;
    let (args, description) = command::usage(spec);
    let typed = argument(command).map(|(_, index)| index);
    // Past the last argument: still on it when it takes the rest of the line (`echo <text>`).
    let current = typed.and_then(|index| match args.len() {
        0 => None,
        n if index < n => Some(index),
        n if args[n - 1].contains("text") => Some(n - 1),
        _ => None,
    });
    Some(ArgHelp {
        name: spec.name,
        args,
        current,
        description,
        cvar: spec.is_cvar(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking_prefers_prefix_then_substring_then_scattered_letters() {
        assert_eq!(score("map", "map"), Some(0));
        assert!(
            score("map", "map") < score("map_background", "map"),
            "exact first"
        );
        assert_eq!(
            score("map", "ma"),
            score("map_background", "ma"),
            "prefixes tie"
        );
        assert!(score("map_background", "back") > score("background_map", "back"));
        assert_eq!(score("background_map", "back"), Some(1));
        assert!(score("mat_setvideomode", "msv").is_some_and(|s| s >= 300));
        assert_eq!(score("fov", "xyz"), None);
        assert_eq!(score("Volume", "VOL"), Some(1), "case-insensitive");
    }
}
