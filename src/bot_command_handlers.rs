use crate::bot_command_helpers::{
    mensa_disp_or_upd, parse_time_send_status_msgs, send_bloat_image,
};
use crate::constants::NO_DB_MSG;
use crate::data_types::{
    Command, DialogueState, DialogueType, HandlerResult, JobHandlerTask, MensaKeyboardAction,
    UnregisterTask, UpdateRegistrationTask,
};

use crate::db_operations::set_user_allergen_state;
use crate::shared_main::{
    build_meal_message_dispatcher, get_user_registration, insert_user_registration,
    make_commands_keyrow, make_mensa_keyboard,
};
use rand::Rng;
use std::{collections::BTreeMap, time::Instant};
use teloxide::{prelude::*, types::ParseMode};
use tokio::sync::broadcast;

pub async fn start(bot: Bot, msg: Message, mensen: BTreeMap<u32, String>) -> HandlerResult {
    let keyboard = make_mensa_keyboard(mensen, MensaKeyboardAction::Register);
    bot.send_message(msg.chat.id, "Mensa auswählen:")
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

pub async fn invalid_cmd(bot: Bot, msg: Message) -> HandlerResult {
    bot.send_message(msg.chat.id, "Das ist kein Befehl.")
        .await?;
    Ok(())
}

pub async fn day_cmd(bot: Bot, msg: Message, cmd: Command) -> HandlerResult {
    let days_forward = match cmd {
        Command::Heute => 0,
        Command::Morgen => 1,
        Command::Übermorgen => 2,
        _ => unreachable!(),
    };

    if let Some(registration) = get_user_registration(msg.chat.id.0) {
        let text =
            build_meal_message_dispatcher(msg.chat.id.0, days_forward, registration.mensa_id).await;
        let now = Instant::now();

        bot.send_message(msg.chat.id, text)
            .parse_mode(ParseMode::MarkdownV2)
            .reply_markup(make_commands_keyrow())
            .await?;

        log::debug!("Send {:?} msg: {:.2?}", cmd, now.elapsed());
    } else {
        // every user has a registration (starting the chat always calls /start)
        // if this is none, it most likely means the DB was wiped)
        // forcing reregistration is better than crashing the bot, no data will be overwritten anyways
        // but copy pasting this everywhere is ugly
        bot.send_message(msg.chat.id, NO_DB_MSG).await?;
    }
    Ok(())
}

pub async fn show_different_mensa(
    bot: Bot,
    msg: Message,
    mensen: BTreeMap<u32, String>,
) -> HandlerResult {
    mensa_disp_or_upd(bot, msg, mensen, MensaKeyboardAction::DisplayOnce).await
}

pub async fn subscribe(
    bot: Bot,
    msg: Message,
    jobhandler_task_tx: broadcast::Sender<JobHandlerTask>,
) -> HandlerResult {
    if let Some(registration) = get_user_registration(msg.chat.id.0) {
        if registration.job_uuid.is_some() {
            if rand::thread_rng().gen_range(0..10) == 0 {
                send_bloat_image(&bot, msg.chat.id).await;
            }

            bot.send_message(
                msg.chat.id,
                "Automatische Nachrichten sind schon aktiviert.",
            )
            .await?;
        } else {
            bot.send_message(
                        msg.chat.id,
                        "Plan wird ab jetzt automatisch an Wochentagen *06:00 Uhr* gesendet\\.\n\nÄndern mit /uhrzeit",
                    ).parse_mode(ParseMode::MarkdownV2)
                    .await?;

            let registration_job = UpdateRegistrationTask {
                chat_id: msg.chat.id.0,
                mensa_id: None,
                hour: Some(6),
                minute: Some(0),
            }
            .into();

            jobhandler_task_tx.send(registration_job).unwrap();
        }
    } else {
        bot.send_message(msg.chat.id, NO_DB_MSG).await?;
    }

    Ok(())
}

pub async fn unsubscribe(
    bot: Bot,
    msg: Message,
    jobhandler_task_tx: broadcast::Sender<JobHandlerTask>,
) -> HandlerResult {
    if let Some(registration) = get_user_registration(msg.chat.id.0) {
        if registration.job_uuid.is_none() {
            bot.send_message(
                msg.chat.id,
                "Automatische Nachrichten waren bereits deaktiviert.",
            )
            .await?;
        } else {
            bot.send_message(msg.chat.id, "Plan wird nicht mehr automatisch gesendet.")
                .await?;

            jobhandler_task_tx
                .send(
                    UnregisterTask {
                        chat_id: msg.chat.id.0,
                    }
                    .into(),
                )
                .unwrap();
        }
    } else {
        bot.send_message(msg.chat.id, NO_DB_MSG).await?;
    }
    Ok(())
}

pub async fn change_mensa(bot: Bot, msg: Message, mensen: BTreeMap<u32, String>) -> HandlerResult {
    mensa_disp_or_upd(bot, msg, mensen, MensaKeyboardAction::Update).await
}

pub async fn allergene(bot: Bot, msg: Message) -> HandlerResult {
    if let Some(mut registration) = get_user_registration(msg.chat.id.0) {
        registration.allergens = !registration.allergens;

        set_user_allergen_state(msg.chat.id.0, registration.allergens)?;
        insert_user_registration(msg.chat.id.0, registration);

        match registration.allergens {
            true => {
                bot.send_message(msg.chat.id, "✅ Allergene werden jetzt angezeigt.")
                    .await?;
            }
            false => {
                bot.send_message(msg.chat.id, "❌ Allergene werden nicht mehr angezeigt.")
                    .await?;
            }
        }
    } else {
        bot.send_message(msg.chat.id, NO_DB_MSG).await?;
    }

    Ok(())
}

pub async fn senddiff(bot: Bot, msg: Message) -> HandlerResult {
    if let Some(mut registration) = get_user_registration(msg.chat.id.0) {
        registration.senddiff = !registration.senddiff;

        set_user_allergen_state(msg.chat.id.0, registration.senddiff)?;
        insert_user_registration(msg.chat.id.0, registration);

        match registration.senddiff {
            true => {
                bot.send_message(
                    msg.chat.id,
                    "✅ Nur Änderungen werden bei Planänderung gesendet.",
                )
                .await?;
            }
            false => {
                bot.send_message(
                    msg.chat.id,
                    "❌ Gesamter Plan wird bei Änderungen gesendet.",
                )
                .await?;
            }
        }
    } else {
        bot.send_message(msg.chat.id, NO_DB_MSG).await?;
    }

    Ok(())
}

pub async fn start_time_dialogue(
    bot: Bot,
    msg: Message,
    dialogue: DialogueType,
    jobhandler_task_tx: broadcast::Sender<JobHandlerTask>,
) -> HandlerResult {
    if get_user_registration(msg.chat.id.0).is_none() {
        bot.send_message(msg.chat.id, NO_DB_MSG).await.unwrap();
        dialogue.exit().await.unwrap();
        return Ok(());
    }
    let argument = msg
        .text()
        .unwrap_or_default()
        .split_once(' ')
        .map(|slices| slices.1.trim());

    match parse_time_send_status_msgs(&bot, msg.chat.id, argument).await {
        Ok(parsed_time) => {
            jobhandler_task_tx
                .send(
                    UpdateRegistrationTask {
                        chat_id: msg.chat.id.0,
                        mensa_id: None,
                        hour: Some(parsed_time.hour),
                        minute: Some(parsed_time.minute),
                    }
                    .into(),
                )
                .unwrap();
        }

        Err(_) => {
            dialogue
                .update(DialogueState::AwaitTimeReply)
                .await
                .unwrap();
        }
    }

    Ok(())
}

pub async fn reply_time_dialogue(
    bot: Bot,
    msg: Message,
    dialogue: DialogueType,
    jobhandler_task_tx: broadcast::Sender<JobHandlerTask>,
) -> HandlerResult {
    if msg.text().is_none() {
        bot.send_message(
            msg.chat.id,
            "Das ist kein Text.\nBitte mit Uhrzeit antworten:",
        )
        .await?;
        return Ok(());
    }

    if let Ok(parsed_thing) =
        parse_time_send_status_msgs(&bot, msg.chat.id, Some(msg.text().unwrap())).await
    {
        dialogue.exit().await.unwrap();

        let registration_job = UpdateRegistrationTask {
            chat_id: msg.chat.id.0,
            mensa_id: None,
            hour: Some(parsed_thing.hour),
            minute: Some(parsed_thing.minute),
        }
        .into();
        jobhandler_task_tx.send(registration_job).unwrap();
    }

    Ok(())
}
