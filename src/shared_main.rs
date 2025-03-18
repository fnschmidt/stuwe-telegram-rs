use std::{collections::BTreeMap, error::Error, time::Instant};

use chrono::Timelike;
use teloxide::{
    prelude::*,
    types::{KeyboardButton, KeyboardMarkup},
    utils::{command::BotCommands, markdown},
};
use teloxide_core::{
    Bot,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
};
use tokio::sync::broadcast;
use tokio_cron_scheduler::{Job, JobScheduler};
use uuid::Uuid;

use crate::{
    constants::{BACKEND, NO_DB_MSG, USER_REGISTRATIONS},
    data_backend::{mm_parser::mm_build_meal_msg, stuwe_parser::stuwe_build_meal_msg},
    data_types::{Backend, Command, MensaKeyboardAction, RegisterTask, UpdateRegistrationTask},
};
use crate::{
    data_types::{JobHandlerTask, RegistrationEntry},
    db_operations::update_db_row,
};

pub fn get_user_registration(chat_id: i64) -> Option<RegistrationEntry> {
    let user_data = USER_REGISTRATIONS
        .get()
        .and_then(|data| data.read().unwrap().get(&chat_id).copied());
    user_data
}

pub fn insert_user_registration(chat_id: i64, entry: RegistrationEntry) {
    USER_REGISTRATIONS
        .get()
        .unwrap()
        .write()
        .unwrap()
        .insert(chat_id, entry);
}

pub fn make_mensa_keyboard(
    mensen: BTreeMap<u32, String>,
    action: MensaKeyboardAction,
) -> InlineKeyboardMarkup {
    let mut keyboard = Vec::new();

    for mensa in mensen {
        keyboard.push([InlineKeyboardButton::callback(
            &mensa.1,
            format!(
                "m_{}:{}",
                match action {
                    MensaKeyboardAction::Register => "regist",
                    MensaKeyboardAction::Update => "upd",
                    MensaKeyboardAction::DisplayOnce => "disp",
                },
                &mensa.1
            ),
        )]);
    }
    InlineKeyboardMarkup::new(keyboard)
}

pub fn make_commands_keyrow() -> KeyboardMarkup {
    let keyboard = vec![
        vec![
            KeyboardButton::new("/heute"),
            KeyboardButton::new("/morgen"),
            KeyboardButton::new("/andere"),
            // KeyboardButton::new("/übermorgen"),
        ],
        vec![
            KeyboardButton::new("/mensa"),
            KeyboardButton::new("/allergene"),
            KeyboardButton::new("/diff"),
        ],
    ];
    KeyboardMarkup::new(keyboard).resize_keyboard()
}

pub async fn build_meal_message_dispatcher(
    chat_id: i64,
    days_forward: i64,
    mensa_location: u32,
) -> String {
    let wants_allergens = get_user_registration(chat_id)
        .map(|reg| reg.allergens)
        .unwrap_or_default();
    match BACKEND.get().unwrap() {
        Backend::MensiMates => {
            mm_build_meal_msg(days_forward, mensa_location, wants_allergens).await
        }
        Backend::StuWe => stuwe_build_meal_msg(days_forward, mensa_location, wants_allergens).await,
    }
}

pub async fn load_job(bot: Bot, sched: &JobScheduler, task: JobHandlerTask) -> Option<Uuid> {
    // return if no time is set
    task.hour?;

    // convert de time to utc
    let de_time = chrono::Local::now()
        .with_hour(task.hour.unwrap())
        .unwrap()
        .with_minute(task.minute.unwrap())
        .unwrap()
        .with_second(0)
        .unwrap();
    let utc_time = de_time.naive_utc();

    let job = Job::new_async(
        format!(
            "0 {} {} * * Mon,Tue,Wed,Thu,Fri",
            utc_time.minute(),
            utc_time.hour()
        )
        .as_str(),
        move |_uuid, mut _l| {
            let bot = bot.clone();

            Box::pin(async move {
                bot.send_message(
                    ChatId(task.chat_id.unwrap()),
                    build_meal_message_dispatcher(task.chat_id.unwrap(), 0, task.mensa_id.unwrap())
                        .await,
                )
                .parse_mode(ParseMode::MarkdownV2)
                .await
                .unwrap();
            })
        },
    )
    .unwrap();

    let uuid = job.guid();
    sched.add(job).await.unwrap();

    Some(uuid)
}

pub async fn callback_handler(
    bot: Bot,
    q: CallbackQuery,
    mensen: BTreeMap<u32, String>,
    jobhandler_task_tx: broadcast::Sender<JobHandlerTask>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(q_data) = q.data {
        // acknowledge callback query to remove the loading alert
        bot.answer_callback_query(q.id).await?;

        if let Some(message) = q.message {
            let id = message.id();
            let chat = message.chat();

            let (cmd, arg) = q_data.split_once(':').unwrap();
            match cmd {
                "m_upd" => {
                    // replace mensa selection message with selected mensa
                    bot.edit_message_text(
                        chat.id,
                        id,
                        format!("Gewählte Mensa: {}", markdown::bold(arg)),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await
                    .unwrap();

                    let text = build_meal_message_dispatcher(
                        chat.id.0,
                        0,
                        *mensen.iter().find(|(_, v)| v.as_str() == arg).unwrap().0,
                    )
                    .await;

                    bot.send_message(chat.id, text)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;

                    let task = UpdateRegistrationTask {
                        chat_id: chat.id.0,
                        mensa_id: Some(*mensen.iter().find(|(_, v)| v.as_str() == arg).unwrap().0),
                        hour: None,
                        minute: None,
                    }
                    .into();

                    update_db_row(&task).unwrap();
                    jobhandler_task_tx.send(task).unwrap();
                }
                "m_disp" => {
                    // replace mensa selection message with selected mensa
                    bot.edit_message_text(chat.id, id, format!("{}:", markdown::bold(arg)))
                        .parse_mode(ParseMode::MarkdownV2)
                        .await
                        .unwrap();

                    bot.send_message(
                        chat.id,
                        build_meal_message_dispatcher(
                            chat.id.0,
                            0,
                            *mensen.iter().find(|(_, v)| v.as_str() == arg).unwrap().0,
                        )
                        .await,
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
                "m_regist" => {
                    // replace mensa selection message with list of commands
                    bot.edit_message_text(chat.id, id, Command::descriptions().to_string())
                        .await?;

                    let task = RegisterTask {
                        chat_id: chat.id.0,
                        mensa_id: *mensen.iter().find(|(_, v)| v.as_str() == arg).unwrap().0,
                        hour: 6,
                        minute: 0,
                    }
                    .into();

                    update_db_row(&task).unwrap();
                    jobhandler_task_tx.send(task).unwrap();

                    bot
                        .send_message(chat.id, format!("Plan der {} wird ab jetzt automatisch an Wochentagen *06:00 Uhr* gesendet\\.\n\nÄndern mit\n/mensa oder /uhrzeit", markdown::bold(arg)))
                        .parse_mode(ParseMode::MarkdownV2)
                        .reply_markup(make_commands_keyrow()).await?;

                    let text = build_meal_message_dispatcher(
                        chat.id.0,
                        0,
                        *mensen.iter().find(|(_, v)| v.as_str() == arg).unwrap().0,
                    )
                    .await;

                    bot.send_message(chat.id, text)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
                "day" => {
                    if let Some(registration) = get_user_registration(chat.id.0) {
                        // start building message
                        let now = Instant::now();

                        let days_forward = arg.parse::<i64>().unwrap();
                        let day_str = ["Heute", "Morgen", "Übermorgen"]
                            [usize::try_from(days_forward).unwrap()];

                        let text = build_meal_message_dispatcher(
                            chat.id.0,
                            days_forward,
                            registration.mensa_id,
                        )
                        .await;
                        log::debug!("Build {} msg: {:.2?}", day_str, now.elapsed());
                        let now = Instant::now();

                        bot.send_message(chat.id, text)
                            .parse_mode(ParseMode::MarkdownV2)
                            .await?;
                        log::debug!("Send {} msg: {:.2?}", day_str, now.elapsed());
                    } else {
                        bot.send_message(chat.id, NO_DB_MSG).await?;
                    }
                }
                _ => panic!("Unknown callback query command: {}", cmd),
            }
        }
    }

    Ok(())
}
