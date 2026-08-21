use super::registry::{COMMANDS, SlashCommand};

pub const MAX_PALETTE_ITEMS: usize = 5;

pub fn slash_query(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('/')?;
    if rest.contains('\n') {
        return None;
    }
    Some(rest.split_whitespace().next().unwrap_or(""))
}

pub fn slash_args(text: &str) -> String {
    let Some(rest) = text.strip_prefix('/') else {
        return String::new();
    };
    rest.split_once(char::is_whitespace)
        .map(|(_, args)| args.trim().to_string())
        .unwrap_or_default()
}

pub fn matching_commands(query: &str) -> Vec<&'static SlashCommand> {
    let query = query.to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|command| command.name.starts_with(&query))
        .collect()
}

pub fn palette_row_count(match_count: usize, max_height: u16) -> u16 {
    if max_height == 0 {
        return 0;
    }
    let has_footer = max_height >= 2;
    let item_budget = if has_footer {
        usize::from(max_height - 1)
    } else {
        usize::from(max_height)
    };
    let items = match_count
        .clamp(1, MAX_PALETTE_ITEMS)
        .min(item_budget.max(1));
    let footer = u16::from(has_footer);
    items as u16 + footer
}

#[cfg(test)]
mod tests {
    use super::{matching_commands, palette_row_count, slash_args, slash_query};
    use crate::frontend::composer::slash_commands::COMMANDS;

    #[test]
    fn slash_query_requires_a_leading_slash_on_one_line() {
        assert_eq!(slash_query(""), None);
        assert_eq!(slash_query("help"), None);
        assert_eq!(slash_query(" /settings"), None);
        assert_eq!(slash_query("/"), Some(""));
        assert_eq!(slash_query("/set"), Some("set"));
        assert_eq!(slash_query("/settings extra"), Some("settings"));
        assert_eq!(slash_query("/settings\nextra"), None);
    }

    #[test]
    fn slash_args_are_the_text_after_the_command_token() {
        assert_eq!(slash_args("/"), "");
        assert_eq!(slash_args("/export"), "");
        assert_eq!(slash_args("/export ./out.html"), "./out.html");
        assert_eq!(slash_args("hello"), "");
    }

    #[test]
    fn matching_commands_are_case_insensitive_prefixes_in_registry_order() {
        let matches = matching_commands("");
        assert_eq!(matches.len(), COMMANDS.len());
        assert_eq!(matches[0].name, "settings");

        let matches = matching_commands("s");
        assert_eq!(
            matches
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            ["settings", "scoped-models", "status"]
        );

        let matches = matching_commands("SET");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "settings");

        assert!(matching_commands("zzzz").is_empty());
    }

    #[test]
    fn palette_row_count_reserves_a_footer_when_there_is_room() {
        assert_eq!(palette_row_count(12, 0), 0);
        assert_eq!(palette_row_count(12, 1), 1);
        assert_eq!(palette_row_count(12, 2), 2);
        assert_eq!(palette_row_count(12, 6), 6);
        assert_eq!(palette_row_count(12, 20), 6);
        assert_eq!(palette_row_count(0, 6), 2);
        assert_eq!(palette_row_count(3, 6), 4);
    }
}
