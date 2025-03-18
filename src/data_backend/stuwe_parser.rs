use std::collections::BTreeMap;

use crate::constants::API_URL;
use crate::data_backend::{EMOJIS, escape_markdown_v2, german_date_fmt};
use crate::data_types::stuwe_data_types::{CanteenMealDiff, MealGroup};

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Local, Weekday};
use teloxide::utils::markdown;

use tokio::time::Instant;
pub async fn stuwe_build_meal_msg(
    days_forward: i64,
    mensa_location: u32,
    wants_allergens: bool,
) -> String {
    // get requested date
    let mut requested_date = chrono::Local::now() + Duration::days(days_forward);
    let mut date_raised_by_days = 0;

    match requested_date.weekday() {
        // sat -> change req_date to mon
        Weekday::Sat => {
            requested_date += Duration::days(2);
            date_raised_by_days = 2;
        }
        Weekday::Sun => {
            requested_date += Duration::days(1);
            date_raised_by_days = 1;
        }
        _ => {
            // Any other weekday is fine, nothing to do
        }
    }

    let mut msg: String = String::new();
    // start message formatting
    let rand_emoji = EMOJIS[rand::random_range(0..EMOJIS.len())];
    msg += &format!(
        "{} {} {}\n",
        rand_emoji,
        german_date_fmt(requested_date.date_naive()),
        rand_emoji,
    );

    // warn if requested "today" was raised to next monday (requested on sat/sun)
    if days_forward == 0 && date_raised_by_days == 1 {
        msg += &markdown::italic("      (Morgen)\n");
    } else if days_forward == 0 && date_raised_by_days == 2 {
        msg += &markdown::italic("      (Übermorgen)\n")
    }

    // retrieve meals
    let now = Instant::now();
    match get_meals_from_api(requested_date, mensa_location).await {
        Err(_) => msg = "Ein Fehler ist aufgetreten.".to_string(),
        Ok(meal_groups) => {
            log::debug!("API data: {:.2?}", now.elapsed());

            if meal_groups.is_empty() {
                msg += &markdown::bold("\nkeine Daten vorhanden.\n");
            } else {
                msg += &mealgroups_to_msg(&meal_groups, wants_allergens);
            }
        }
    };

    escape_markdown_v2(&msg)
}

pub async fn stuwe_build_diff_msg(diff: &CanteenMealDiff, wants_allergens: bool) -> String {
    let mut msg = markdown::bold(&markdown::underline("Planänderung")).to_string();
    if let Some(new_meals) = diff.new_meals.as_ref() {
        msg += &markdown::bold(&markdown::underline(if new_meals.len() == 1 {
            "\nNeues Gericht:"
        } else {
            "\nNeue Gerichte:"
        }));
        msg += &mealgroups_to_msg(new_meals, wants_allergens);
    }

    let use_modified = match wants_allergens {
        true => &diff.modified_meals,
        false => &diff.modified_meals_ignoring_allergens,
    };

    if let Some(modified_meals) = use_modified {
        msg += &markdown::bold(&markdown::underline(if modified_meals.len() == 1 {
            "\nGeändertes Gericht:"
        } else {
            "\nGeänderte Gerichte:"
        }));
        msg += &mealgroups_to_msg(modified_meals, wants_allergens);
    }

    if let Some(removed_meals) = diff.removed_meals.as_ref() {
        msg += &markdown::bold(&markdown::underline(if removed_meals.len() == 1 {
            "\nEntferntes Gericht:"
        } else {
            "\nEntfernte Gerichte:"
        }));
        for rem_meal in removed_meals {
            for sub_meal in &rem_meal.sub_meals {
                msg += &format!("\n • {}", markdown::underline(&sub_meal.name));
            }
        }
    }

    escape_markdown_v2(msg.trim_end())
}

fn mealgroups_to_msg(meal_groups: &[MealGroup], wants_allergens: bool) -> String {
    let mut msg: String = String::new();

    // loop over meal groups
    for meal_group in meal_groups {
        let price_first_meal = meal_group.sub_meals.first().unwrap().price.clone();
        let price_is_shared = meal_group
            .sub_meals
            .iter()
            .all(|item| item.price == price_first_meal);

        // Bold type of meal (-group)
        msg += &format!(
            "\n{} {}\n",
            get_mealgroup_icon(&meal_group.meal_type),
            markdown::bold(&meal_group.meal_type)
        );

        // loop over meals in meal group
        for sub_meal in &meal_group.sub_meals {
            // underlined single or multiple meal name
            if !(meal_group.sub_meals.len() == 1 && sub_meal.name == meal_group.meal_type) {
                msg += &format!(" • {}\n", markdown::underline(&sub_meal.name));
            }

            // loop over ingredients of meal
            for ingredient in &sub_meal.additional_ingredients {
                // appending ingredient to msg
                msg += &format!("     + {}\n", markdown::italic(ingredient))
            }
            if wants_allergens {
                if let Some(allergens) = sub_meal.allergens.as_ref() {
                    msg += &format!("    ⓘ {}\n", allergens)
                }
            }
            // appending price
            if !price_is_shared {
                msg += &format!("   {}\n", sub_meal.price);
            }

            if let Some(variations) = sub_meal.variations.as_ref() {
                msg += &format!("   → {}\n", markdown::bold("Variationen:"));
                for variation in variations {
                    msg += &format!("       • {}\n", markdown::italic(&variation.name));
                    if wants_allergens {
                        if let Some(allergens_and_add) = variation.allergens_and_add.as_ref() {
                            msg += &format!("         ⓘ {}\n", allergens_and_add)
                        }
                    }
                }
            }
        }
        if price_is_shared {
            msg += &format!("  {}\n", price_first_meal);
        }
    }

    msg
}

fn get_mealgroup_icon(meal_name: &str) -> &'static str {
    match meal_name.to_lowercase().as_str() {
        s if s.contains("vegan") => "🌱",
        s if s.contains("vegetarisch") => "🧀",
        s if s.contains("fleisch") => "🍗",
        s if s.contains("grill") => "🍔",
        s if s.contains("fisch") => "🐟",
        s if s.contains("pastateller") => "🍝",
        s if s.contains("gemüse") => "🥕",
        s if s.contains("sättigung") => "➕",
        s if s.contains("schneller teller") => "♿️",
        _ => "🍽️",
    }
}

async fn get_meals_from_api(requested_date: DateTime<Local>, mensa: u32) -> Result<Vec<MealGroup>> {
    let date_str = build_date_string(requested_date);
    let client = reqwest::Client::new();
    let meal_groups = client
        .get(format!(
            "{}/canteens/{}/days/{}",
            API_URL.get().unwrap(),
            mensa,
            date_str
        ))
        .send()
        .await?
        .json()
        .await?;

    Ok(meal_groups)
}

fn build_date_string(requested_date: DateTime<Local>) -> String {
    let (year, month, day) = (
        requested_date.year(),
        requested_date.month(),
        requested_date.day(),
    );

    format!("{:04}-{:02}-{:02}", year, month, day)
}

pub async fn get_mensen() -> BTreeMap<u32, String> {
    let client = reqwest::Client::new();

    #[derive(serde::Deserialize)]
    struct Mensa {
        id: u32,
        name: String,
    }

    let text = loop {
        if let Ok(res) = client
            .get(format!("{}/canteens", API_URL.get().unwrap()))
            .send()
            .await
        {
            if let Ok(text) = res.text().await {
                break text;
            }
        };

        log::error!("Mensa API unreachable, retry in 5s");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    };

    let mensa_list: Vec<Mensa> = serde_json::from_str(&text).unwrap();

    mensa_list.into_iter().map(|m| (m.id, m.name)).collect()
}
