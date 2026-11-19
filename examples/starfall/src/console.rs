//! The developer console: commands that tune presentation and restart the
//! run. Commands never edit the simulated world directly; a restart goes
//! through the recorded input like the R key, so replays stay exact.
use quasar_ui::console::{ArgKind, Choice, CommandSet, Matched};

/// What the game's systems currently look like, for suggestions.
#[derive(Clone, Debug, Default)]
pub struct ConsoleState {
    pub volume: f32,
    pub shake: f32,
    pub hit_stop: bool,
}

/// A parsed console command.
#[derive(Clone, Debug, PartialEq)]
pub enum ConsoleCommand {
    Help,
    Restart,
    /// Master volume, 0..=1.
    Volume(f32),
    /// Camera shake scale, 0..=1.
    Shake(f32),
    HitStop(bool),
}

pub fn commands() -> CommandSet<ConsoleState> {
    let mut set = CommandSet::new();
    set.kind("percent", ArgKind::Number { min: 0, max: 100 })
        .kind(
            "switch",
            ArgKind::choices(|state: &ConsoleState, _| {
                let current = if state.hit_stop { "on" } else { "off" };
                vec![
                    Choice::new("on", if current == "on" { "current" } else { "" }),
                    Choice::new("off", if current == "off" { "current" } else { "" }),
                ]
            }),
        )
        .command("help", "List commands")
        .command("restart", "Restart the run")
        .command("volume <percent>", "Master volume")
        .command("shake <percent>", "Camera shake strength")
        .command("hitstop <switch>", "Freeze frames on big hits");
    set
}

/// The command a matched line means.
pub fn interpret(matched: &Matched) -> ConsoleCommand {
    let percent = |m: &Matched| m.number(0).unwrap_or(100) as f32 / 100.0;
    match matched.command {
        0 => ConsoleCommand::Help,
        1 => ConsoleCommand::Restart,
        2 => ConsoleCommand::Volume(percent(matched)),
        3 => ConsoleCommand::Shake(percent(matched)),
        _ => ConsoleCommand::HitStop(matched.word(0) == Some("on")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_lines_parse_into_commands() {
        let set = commands();
        let state = ConsoleState::default();
        let run = |line: &str| interpret(&set.parse(line, &state).unwrap());
        assert_eq!(run("/volume 40"), ConsoleCommand::Volume(0.4));
        assert_eq!(run("shake 0"), ConsoleCommand::Shake(0.0));
        assert_eq!(run("/hitstop on"), ConsoleCommand::HitStop(true));
        assert_eq!(run("restart"), ConsoleCommand::Restart);
        assert!(set.parse("/volume 400", &state).is_err());
        let words: Vec<String> = set
            .suggest("/hit", &state)
            .items
            .into_iter()
            .map(|c| c.word)
            .collect();
        assert_eq!(words, ["hitstop"]);
    }
}
