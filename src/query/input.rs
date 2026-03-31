#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedInput {
    Exit,
    Help,
    Prompt(String),
    Slash(SlashCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    ModelList,
    ModelUse(String),
    SkillsList,
    SkillsShow(String),
    SkillsUse(String),
    SkillsClear,
    FilesRead(String),
    FilesList(Option<String>),
    FilesGrep { pattern: String, path: Option<String> },
    ConfigShow,
    Doctor,
    Diff,
    Tasks,
    Permissions,
    Plan,
    Unknown(String),
}

pub fn parse_input(input: &str) -> ParsedInput {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("/exit") || trimmed.eq_ignore_ascii_case("exit") {
        return ParsedInput::Exit;
    }
    if trimmed.eq_ignore_ascii_case("/help") {
        return ParsedInput::Help;
    }
    if !trimmed.starts_with('/') {
        return ParsedInput::Prompt(trimmed.to_string());
    }

    let mut parts = trimmed.split_whitespace();
    let command = parts.next().unwrap_or_default();
    let parsed = match command {
        "/model" => match parts.next() {
            Some("list") | None => SlashCommand::ModelList,
            Some("use") => SlashCommand::ModelUse(parts.collect::<Vec<_>>().join(" ")),
            _ => SlashCommand::Unknown(trimmed.to_string()),
        },
        "/skills" => match parts.next() {
            Some("list") | None => SlashCommand::SkillsList,
            Some("show") => SlashCommand::SkillsShow(parts.collect::<Vec<_>>().join(" ")),
            Some("use") => SlashCommand::SkillsUse(parts.collect::<Vec<_>>().join(" ")),
            Some("clear") => SlashCommand::SkillsClear,
            _ => SlashCommand::Unknown(trimmed.to_string()),
        },
        "/files" => match parts.next() {
            Some("read") => SlashCommand::FilesRead(parts.collect::<Vec<_>>().join(" ")),
            Some("list") => SlashCommand::FilesList(parts.next().map(|s| s.to_string())),
            Some("grep") => {
                let pattern = parts.next().unwrap_or_default().to_string();
                let path = parts.next().map(|s| s.to_string());
                SlashCommand::FilesGrep { pattern, path }
            }
            _ => SlashCommand::Unknown(trimmed.to_string()),
        },
        "/config" => SlashCommand::ConfigShow,
        "/doctor" => SlashCommand::Doctor,
        "/diff" => SlashCommand::Diff,
        "/tasks" => SlashCommand::Tasks,
        "/permissions" => SlashCommand::Permissions,
        "/plan" => SlashCommand::Plan,
        _ => SlashCommand::Unknown(trimmed.to_string()),
    };
    ParsedInput::Slash(parsed)
}

#[cfg(test)]
mod tests {
    use super::{ParsedInput, SlashCommand, parse_input};

    #[test]
    fn parses_new_command_surface() {
        assert_eq!(
            parse_input("/model list"),
            ParsedInput::Slash(SlashCommand::ModelList)
        );
        assert_eq!(
            parse_input("/files grep todo src"),
            ParsedInput::Slash(SlashCommand::FilesGrep {
                pattern: "todo".to_string(),
                path: Some("src".to_string())
            })
        );
    }
}
