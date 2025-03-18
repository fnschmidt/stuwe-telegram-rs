use anyhow::Result;
use chrono::Timelike;
use futures_util::TryStreamExt;
use reqwest::Client;
use reqwest_websocket::{Message, RequestBuilderExt};
use std::{collections::BTreeMap, env, time::Duration};
use teloxide::{
    Bot,
    payloads::SendMessageSetters,
    requests::Requester,
    types::{ChatId, ParseMode},
};
use tokio::{sync::broadcast::Sender, time::sleep};
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    campusdual_fetcher::{
        compare_campusdual_grades, compare_campusdual_signup_options, get_campusdual_data,
        save_campusdual_grades, save_campusdual_signup_options,
    },
    constants::{API_URL, BACKEND, CD_DATA, NO_DB_MSG, USER_REGISTRATIONS},
    data_backend::stuwe_parser::{stuwe_build_diff_msg, stuwe_build_meal_msg},
    data_types::{
        Backend, BroadcastUpdateTask, JobHandlerTask, RegistrationEntry, UpdateRegistrationTask,
    },
    db_operations::{
        get_all_user_registrations_db, get_user_allergen_state, get_user_senddiff_state,
        init_db_record, task_db_kill_auto, update_db_row,
    },
    shared_main::{get_user_registration, insert_user_registration, load_job},
};

pub async fn handle_add_registration_task(
    bot: &Bot,
    job_handler_task: JobHandlerTask,
    sched: &JobScheduler,
) {
    log::info!(
        "Register: {} for Mensa {}",
        &job_handler_task.chat_id.unwrap(),
        &job_handler_task.mensa_id.unwrap()
    );
    // create or update row in db
    init_db_record(&job_handler_task).unwrap();
    let registration = get_user_registration(job_handler_task.chat_id.unwrap());
    if let Some(uuid) = registration.and_then(|reg| reg.job_uuid) {
        sched.context.job_delete_tx.send(uuid).unwrap();
    }

    // get uuid (here guaranteed to be Some() since default is registration with job)
    let new_uuid = load_job(bot.clone(), sched, job_handler_task.clone()).await;

    // insert new job uuid
    insert_user_registration(
        job_handler_task.chat_id.unwrap(),
        RegistrationEntry {
            job_uuid: new_uuid,
            mensa_id: job_handler_task.mensa_id.unwrap(),
            hour: job_handler_task.hour,
            minute: job_handler_task.minute,
            allergens: registration.map(|reg| reg.allergens).unwrap_or(true),
            senddiff: registration.map(|reg| reg.senddiff).unwrap_or(true),
        },
    );
}

pub async fn handle_update_registration_task(
    bot: &Bot,
    job_handler_task: JobHandlerTask,
    sched: &JobScheduler,
) {
    if let Some(mensa) = job_handler_task.mensa_id {
        log::info!("{} 📌 to {}", job_handler_task.chat_id.unwrap(), mensa);
    }
    if job_handler_task.hour.is_some() {
        log::info!(
            "{} changed 🕘: {:02}:{:02}",
            job_handler_task.chat_id.unwrap(),
            job_handler_task.hour.unwrap(),
            job_handler_task.minute.unwrap()
        );
    }

    // reuse old data for unset fields, as the entire row is replaced
    if let Some(registration) = get_user_registration(job_handler_task.chat_id.unwrap()) {
        let mensa_id = job_handler_task.mensa_id.unwrap_or(registration.mensa_id);
        let hour = job_handler_task.hour.or(registration.hour);
        let minute = job_handler_task.minute.or(registration.minute);

        let new_job_task = UpdateRegistrationTask {
            chat_id: job_handler_task.chat_id.unwrap(),
            mensa_id: Some(mensa_id),
            hour,
            minute,
        };

        let new_uuid =
            // new time was passed -> unload old job, load new
            if job_handler_task.hour.is_some() || job_handler_task.minute.is_some() || job_handler_task.mensa_id.is_some() {
                if let Some(uuid) = registration.job_uuid {
                    // unload old job if exists
                    sched.context.job_delete_tx.send(uuid).unwrap();
                }
                // load new job, return uuid
                load_job(
                    bot.clone(),
                    sched,
                    new_job_task.into(),
                ).await
            } else {
                // no new time was set -> return old job uuid
                registration.job_uuid
            };

        insert_user_registration(
            job_handler_task.chat_id.unwrap(),
            RegistrationEntry {
                job_uuid: new_uuid,
                mensa_id,
                hour,
                minute,
                allergens: registration.allergens,
                senddiff: registration.senddiff,
            },
        );

        // update any values that are to be changed, aka are Some()
        update_db_row(&job_handler_task).unwrap();
    } else {
        log::error!("Tried to update non-existent job");
        bot.send_message(ChatId(job_handler_task.chat_id.unwrap()), NO_DB_MSG)
            .await
            .unwrap();
    }
}

pub async fn handle_delete_registration_task(
    job_handler_task: JobHandlerTask,
    sched: &JobScheduler,
) {
    log::info!("Unregister: {}", &job_handler_task.chat_id.unwrap());

    // unregister is only invoked if existence of job is guaranteed
    let registration = get_user_registration(job_handler_task.chat_id.unwrap()).unwrap();

    // unload old job
    sched
        .context
        .job_delete_tx
        .send(registration.job_uuid.unwrap())
        .unwrap();

    // kill uuid from this thing
    insert_user_registration(
        job_handler_task.chat_id.unwrap(),
        RegistrationEntry {
            job_uuid: None,
            mensa_id: registration.mensa_id,
            hour: None,
            minute: None,
            allergens: registration.allergens,
            senddiff: registration.senddiff,
        },
    );

    // delete from db
    task_db_kill_auto(job_handler_task.chat_id.unwrap()).unwrap();
}

pub async fn handle_broadcast_update_task(bot: &Bot, job_handler_task: JobHandlerTask) {
    log::info!(
        "TodayMeals changed @Mensa {}",
        &job_handler_task.meals_diff.as_ref().unwrap().canteen_id
    );

    let diff = job_handler_task.meals_diff.unwrap();

    let workaround = USER_REGISTRATIONS.get().unwrap().read().unwrap().clone();
    for (chat_id, registration_data) in workaround {
        let canteen_id = registration_data.mensa_id;

        let now = chrono::Local::now();

        if let (Some(job_hour), Some(job_minute)) =
            (registration_data.hour, registration_data.minute)
        {
            // send update to all subscribers of this mensa id
            if canteen_id == diff.canteen_id
                        // only send updates after job message has been sent: job hour has to be earlier OR same hour, earlier minute
                        && (job_hour < now.hour() || job_hour == now.hour() && job_minute <= now.minute())
            {
                if env::var_os("ALLERGEN_DBG").is_some() && canteen_id == 118 {
                    let len_all = diff.modified_meals.as_ref().map(|t| t.len()).unwrap_or(0);
                    let len_non_alg = diff
                        .modified_meals_ignoring_allergens
                        .as_ref()
                        .map(|t| t.len())
                        .unwrap_or(0);
                    println!("ALL upd len: {}\nnon-ALG upd len: {}", len_all, len_non_alg);
                    if let Some(all) = &diff.modified_meals {
                        println!("ALL: {:#?}", all);
                    }
                    if let Some(non_alg) = &diff.modified_meals_ignoring_allergens {
                        println!("non-ALG: {:#?}", non_alg);
                    }
                }

                // user doesnt display allergens, but only only diff is allergens -> skip
                if !registration_data.allergens
                    && diff.new_meals.is_none()
                    && diff.removed_meals.is_none()
                    && diff.modified_meals_ignoring_allergens.is_none()
                {
                    continue;
                }

                log::info!("Sent update to {}", chat_id);

                let text = match registration_data.senddiff {
                    true => stuwe_build_diff_msg(&diff, registration_data.allergens).await,
                    false => {
                        stuwe_build_meal_msg(0, diff.canteen_id, registration_data.allergens).await
                    }
                };

                bot.send_message(ChatId(chat_id), text)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await
                    .unwrap();
            }
        }
    }
}

pub async fn start_mensaupd_hook_and_campusdual_job(
    bot: Bot,
    sched: &JobScheduler,
    job_handler_tx: Sender<JobHandlerTask>,
) {
    // listen for mensa updates
    if *BACKEND.get().unwrap() == Backend::StuWe {
        tokio::spawn(async move {
            loop {
                let tx = job_handler_tx.clone();
                let h = await_handle_mealplan_upd(tx).await;
                if h.is_err() {
                    log::error!("WebSocket connection failed");
                }
                sleep(Duration::from_secs(5)).await;
            }
        });
    }

    let cache_and_broadcast_job = Job::new_async("0 0/5 * * * *", move |_uuid, mut _l| {
        let bot = bot.clone();

        Box::pin(async move {
            check_notify_campusdual_grades_signups(bot).await;
        })
    })
    .unwrap();
    sched.add(cache_and_broadcast_job).await.unwrap();
}

async fn await_handle_mealplan_upd(job_handler_tx: Sender<JobHandlerTask>) -> Result<()> {
    let response = Client::default()
        .get(format!("{}/today_updated_diff_ws", API_URL.get().unwrap()))
        .upgrade() // Prepares the WebSocket upgrade.
        .send()
        .await?;

    // Turns the response into a WebSocket stream.
    let mut websocket = response.into_websocket().await?;
    log::info!("MensaUpdate WebSocket connected");

    while let Some(message) = websocket.try_next().await? {
        if let Message::Text(text) = message {
            job_handler_tx.send(
                BroadcastUpdateTask {
                    meals_diff: serde_json::from_str(&text)?,
                }
                .into(),
            )?;
        }
    }

    Ok(())
}

async fn check_notify_campusdual_grades_signups(bot: Bot) {
    if let Some(cd_data) = CD_DATA.get() {
        log::info!("Updating CampusDual");
        match get_campusdual_data(cd_data.username.clone(), cd_data.password.clone()).await {
            Ok((grades, signup_options)) => {
                if let Some(new_grades) = compare_campusdual_grades(&grades).await {
                    log::info!("Got new grades! Sending to {}", cd_data.chat_id);

                    let mut msg = String::from("Neue Note:");
                    for grade in new_grades {
                        msg.push_str(&format!("\n{}: {}", grade.name, grade.grade));
                    }
                    match bot.send_message(ChatId(cd_data.chat_id), msg).await {
                        Ok(_) => save_campusdual_grades(&grades).await,
                        Err(e) => {
                            log::error!("Failed to send CD grades: {}", e);
                        }
                    }
                }
                if let Some(new_signup_options) =
                    compare_campusdual_signup_options(&signup_options).await
                {
                    log::info!("Got new signup options! Sending to {}", cd_data.chat_id);

                    let mut msg = String::from("Neue Anmeldemöglichkeit:");
                    for signup_option in new_signup_options {
                        msg.push_str(&format!(
                            "\n{} ({}) — {}",
                            signup_option.status, signup_option.verfahren, signup_option.name
                        ));
                    }
                    match bot.send_message(ChatId(cd_data.chat_id), msg).await {
                        Ok(_) => save_campusdual_signup_options(&signup_options).await,
                        Err(e) => {
                            log::error!("Failed to send CD signup options: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to get CD grades: {}", e);
            }
        }
    }
}

pub async fn load_jobs_from_db(
    bot: &Bot,
    sched: &JobScheduler,
) -> BTreeMap<i64, RegistrationEntry> {
    let mut loaded_user_data: BTreeMap<i64, RegistrationEntry> = BTreeMap::new();
    let tasks_from_db = get_all_user_registrations_db().unwrap();

    for task in tasks_from_db {
        let bot = bot.clone();

        let uuid = load_job(bot, sched, task.clone()).await;
        loaded_user_data.insert(
            task.chat_id.unwrap(),
            RegistrationEntry {
                job_uuid: uuid,
                mensa_id: task.mensa_id.unwrap(),
                hour: task.hour,
                minute: task.minute,
                // hack job
                allergens: get_user_allergen_state(task.chat_id.unwrap()).unwrap(),
                senddiff: get_user_senddiff_state(task.chat_id.unwrap()).unwrap(),
            },
        );
    }

    loaded_user_data
}
