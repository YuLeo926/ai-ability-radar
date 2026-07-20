use unicode_general_category::{GeneralCategory, get_general_category};

pub const MAX_REPORTED_MODEL_CHARS: usize = 120;

pub fn is_forbidden_display_character(character: char) -> bool {
    if character.is_control() || get_general_category(character) == GeneralCategory::Format {
        return true;
    }

    matches!(
        u32::from(character),
        0x034f
            | 0x115f..=0x1160
            | 0x17b4..=0x17b5
            | 0x180b..=0x180f
            | 0x2065
            | 0x3164
            | 0xfe00..=0xfe0f
            | 0xffa0
            | 0xfff0..=0xfff8
            | 0xe0000..=0xe0fff
    )
}

pub fn contains_forbidden_display_character(value: &str) -> bool {
    value.chars().any(is_forbidden_display_character)
}

pub fn is_valid_reported_model(value: &str) -> bool {
    value == value.trim()
        && !value.is_empty()
        && value.chars().count() <= MAX_REPORTED_MODEL_CHARS
        && !contains_forbidden_display_character(value)
}
