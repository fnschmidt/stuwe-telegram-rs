use chrono::{Datelike, NaiveDate};

pub mod mm_parser;
pub mod stuwe_parser;

const EMOJIS: [&str; 7] = ["☀️", "🦀", "💂🏻‍♀️", "☕️", "☝🏻", "🌤️", "🥦"];

fn german_date_fmt(date: NaiveDate) -> String {
    let week_days = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag"];

    format!(
        "{}, {}",
        week_days[date.weekday().num_days_from_monday() as usize],
        date.format("%d.%m.%Y")
    )
}

fn escape_markdown_v2(input: &str) -> String {
    // all 'special' chars have to be escaped when using telegram markdown_v2

    input
        .replace('.', r"\.")
        .replace('!', r"\!")
        .replace('+', r"\+")
        .replace('-', r"\-")
        .replace('<', r"\<")
        .replace('>', r"\>")
        .replace('(', r"\(")
        .replace(')', r"\)")
        .replace('=', r"\=")
        // workaround for html things not being properly decoded
        .replace("&amp;", "&")
        .replace("&nbsp;", r" ")
}
