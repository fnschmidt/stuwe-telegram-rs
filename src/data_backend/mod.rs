use chrono::{Datelike, NaiveDate};

cfg_if! {
    if #[cfg(feature = "mensimates")] {
        pub mod mm_parser;
    } else {
        pub mod stuwe_parser;
    }
}

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
        // workaround as '&' in html is improperly decoded
        .replace("&amp;", "&")
}
