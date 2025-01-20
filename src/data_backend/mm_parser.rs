use crate::data_backend::{escape_markdown_v2, german_date_fmt, EMOJIS};
use crate::data_types::mm_data_types::{GetMensasMensa, MensiMeal};

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Local, Weekday};
use rand::Rng;
use std::{collections::BTreeMap, time::Instant};
use teloxide::utils::markdown;

pub async fn get_mensen() -> Result<BTreeMap<u32, String>> {
    let mut mensen = BTreeMap::new();
    let client = reqwest::Client::new();
    let res = loop {
        match client
            .get("https://api.cyber-biene.de/mensaHub/mensa/getMensas")
            .send()
            .await
        {
            Ok(res) => break res,
            Err(_) => {
                log::error!("MensaHub unreachable, retry in 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await
            }
        }
    };
    for mensa in res.json::<Vec<GetMensasMensa>>().await? {
        mensen.insert(mensa.id, mensa.name);
    }

    Ok(mensen)
}

async fn mm_get_meals_at_mensa_at_day(
    date: &DateTime<Local>,
    mensa_id: u32,
) -> Result<Vec<MensiMeal>> {
    let client = reqwest::Client::new();
    let date_str = format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day());

    let now = Instant::now();
    let resp = client
        .get(format!(
            "https://api.cyber-biene.de/mensaHub/meal/servingDate/{}/fromMensa/{}",
            date_str, mensa_id
        ))
        .send()
        .await?
        .error_for_status()?;

    log::info!("MensiMates response: {:.2?}", now.elapsed());

    Ok(resp.json::<Vec<MensiMeal>>().await?)
}

pub async fn mm_build_meal_msg(
    days_forward: i64,
    mensa_location: u32,
    wants_allergens: bool,
) -> String {
    let mut msg: String = String::new();

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

    // retrieve meals
    let day_meals = mm_get_meals_at_mensa_at_day(&requested_date, mensa_location).await;

    let now = Instant::now();

    // start message formatting
    let rand_emoji = EMOJIS[rand::thread_rng().gen_range(0..EMOJIS.len())];
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

    match day_meals {
        Ok(meals) => {
            if meals.is_empty() {
                msg += &markdown::bold("\nkeine Daten vorhanden.\n");
            } else {
                let mut structured_day_meals: BTreeMap<String, Vec<MensiMeal>> = BTreeMap::new();

                // loop over meal groups
                for meal in meals {
                    if let Some(cat) = structured_day_meals.get_mut(&meal.category) {
                        cat.push(meal);
                    } else {
                        structured_day_meals.insert(meal.category.clone(), vec![meal]);
                    }
                }

                for meal_group in structured_day_meals {
                    // Bold type of meal (-group)
                    let title = meal_group.0;
                    msg += &format!(
                        "\n{} {}\n",
                        get_mealgroup_icon(&title),
                        markdown::bold(&title)
                    );

                    let meals_in_group = meal_group.1;
                    if meals_in_group.is_empty() {
                        msg += &format!(" • {}\n", markdown::underline("Keine Gerichte..."));
                        continue;
                    }

                    let price_first_meal = meals_in_group.first().unwrap().price.clone();
                    let price_is_shared = meals_in_group
                        .iter()
                        .all(|item| item.price == price_first_meal);

                    for meal in meals_in_group {
                        msg += &format!(" • {}\n", markdown::underline(&meal.name));

                        let sub_ingredients = &meal
                            .description
                            .split('·')
                            .map(|x| x.trim())
                            .collect::<Vec<&str>>();
                        for ingr in sub_ingredients {
                            if *ingr != "N/A" && !ingr.is_empty() {
                                msg += &format!("     + {}\n", markdown::italic(ingr));
                            }
                        }

                        if wants_allergens && meal.allergens != "N/A" {
                            msg += &format!("    ⓘ {}\n", meal.allergens);
                        };

                        if !price_is_shared {
                            msg += &format!("   {}\n", &meal.price);
                        }

                        if meal.votes != 0 {
                            msg += &format!(
                                "    Bewertung: {} ({})\n",
                                float_rating_to_stars(meal.rating),
                                meal.rating
                            );
                        }
                    }

                    if price_is_shared {
                        msg += &format!("  {}\n", price_first_meal);
                    }
                }
            }
        }
        Err(e) => {
            msg += &markdown::bold("\nMensiMates-Abfrage fehlgeschlagen:\n");
            msg += &e.to_string().replace('_', r"\_");
        }
    }

    let msg = escape_markdown_v2(&msg);
    log::info!("MensiMates build msg: {:.2?}", now.elapsed());

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

pub fn float_rating_to_stars(rating: f32) -> String {
    let floor = rating.floor();
    let partial_star = match rating - floor {
        r if r >= 0.875 => '🌕',
        r if r >= 0.625 => '🌖',
        r if r >= 0.375 => '🌗',
        r if r >= 0.125 => '🌘',
        _ => '🌑',
    };

    let mut stars: String = "🌕".repeat(floor as usize);
    if (floor as usize) < 5 {
        stars.push(partial_star);
        stars.push_str(&"🌑".repeat(5 - stars.chars().count()));
    }

    stars
}
