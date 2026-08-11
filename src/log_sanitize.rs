#[derive(Clone, Copy)]
enum State {
    Text,
    Escape,
    Csi,
    Osc,
    OscEscape,
    ControlString,
    ControlStringEscape,
}

pub(crate) fn sanitize_log_line(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut state = State::Text;

    for character in input.chars() {
        state = match state {
            State::Text => match character {
                '\u{1b}' => State::Escape,
                '\u{0090}' | '\u{0098}' | '\u{009e}' | '\u{009f}' => State::ControlString,
                '\u{009b}' => State::Csi,
                '\u{009d}' => State::Osc,
                character if is_unwanted_control(character) => State::Text,
                character => {
                    output.push(character);
                    State::Text
                }
            },
            State::Escape => match character {
                '\u{1b}' => State::Escape,
                '[' => State::Csi,
                ']' => State::Osc,
                'P' | 'X' | '^' | '_' => State::ControlString,
                '\u{20}'..='\u{2f}' => State::Escape,
                _ => State::Text,
            },
            State::Csi => match character {
                '\u{1b}' => State::Escape,
                '\u{40}'..='\u{7e}' => State::Text,
                _ => State::Csi,
            },
            State::Osc => match character {
                '\u{0007}' | '\u{009c}' => State::Text,
                '\u{1b}' => State::OscEscape,
                _ => State::Osc,
            },
            State::OscEscape => match character {
                '\\' => State::Text,
                '\u{1b}' => State::OscEscape,
                _ => State::Osc,
            },
            State::ControlString => match character {
                '\u{009c}' => State::Text,
                '\u{1b}' => State::ControlStringEscape,
                _ => State::ControlString,
            },
            State::ControlStringEscape => match character {
                '\\' => State::Text,
                '\u{1b}' => State::ControlStringEscape,
                _ => State::ControlString,
            },
        };
    }

    output
}

fn is_unwanted_control(character: char) -> bool {
    character.is_control() && character != '\t'
}

#[cfg(test)]
mod tests {
    use super::sanitize_log_line;

    #[test]
    fn strips_sgr_color_and_style_sequences() {
        let line = "\u{1b}[2m2026-08-11T07:08:07Z\u{1b}[0m \u{1b}[32mINFO\u{1b}[0m bridge ready";

        assert_eq!(
            sanitize_log_line(line),
            "2026-08-11T07:08:07Z INFO bridge ready"
        );
    }

    #[test]
    fn strips_osc_links_and_window_titles() {
        let line = "\u{1b}]0;secret title\u{7}before \u{1b}]8;;https://example.com\u{1b}\\link\u{1b}]8;;\u{1b}\\ after";

        assert_eq!(sanitize_log_line(line), "before link after");
    }

    #[test]
    fn strips_cursor_commands_and_incomplete_sequences() {
        assert_eq!(sanitize_log_line("before\u{1b}[2Jafter"), "beforeafter");
        assert_eq!(sanitize_log_line("safe\u{1b}[31"), "safe");
    }

    #[test]
    fn preserves_unicode_equals_signs_and_tabs() {
        let line = "adapter=飞书\t状态=正常 ✅";

        assert_eq!(sanitize_log_line(line), line);
    }

    #[test]
    fn removes_embedded_terminal_control_characters() {
        assert_eq!(
            sanitize_log_line("first\rsecond\u{8}third\u{7}"),
            "firstsecondthird"
        );
    }
}
