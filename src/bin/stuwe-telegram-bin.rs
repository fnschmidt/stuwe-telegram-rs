// GEANT_OV_RSA_CA_4_tcs-cert3.pem has to be properly set up, eg. in /etc/ssl/certs for Debian
// (the container image already has it)
use stuwe_telegram_rs::data_types::CampusDualData;

use stuwe_telegram_rs::bot_command_handlers::{
    change_mensa, day_cmd, invalid_cmd, reply_time_dialogue, show_different_mensa, start,
    start_time_dialogue, subscribe, unsubscribe,
};
use stuwe_telegram_rs::constants::{
    BACKEND, CD_DATA, DB_FILENAME, OLLAMA_HOST, OLLAMA_MODEL, STUWE_DB,
};
use stuwe_telegram_rs::data_backend::stuwe_parser::{get_mensen, update_cache};
use stuwe_telegram_rs::data_types::{
    Backend, Command, DialogueState, JobHandlerTask, JobHandlerTaskType, JobType,
    QueryRegistrationType, RegistrationEntry,
};
use stuwe_telegram_rs::db_operations::{check_or_create_db_tables, init_mensa_id_db};
use stuwe_telegram_rs::shared_main::callback_handler;
use stuwe_telegram_rs::task_scheduler_funcs::{
    handle_add_registration_task, handle_broadcast_update_task, handle_delete_registration_task,
    handle_query_registration_task, handle_update_registration_task, load_jobs_from_db,
    start_mensacache_and_campusdual_job,
};

use clap::Parser;
use std::collections::BTreeMap;
use std::env;
use teloxide::{
    dispatching::{
        dialogue::{self, InMemStorage},
        UpdateHandler,
    },
    prelude::*,
};
use tokio::sync::broadcast;
use tokio_cron_scheduler::JobScheduler;

/// Telegram bot to receive daily meal plans from any Studentenwerk Leipzig mensa.
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// The telegram bot token to be used
    #[arg(short, long, env)]
    token: String,
    #[arg(short, long, env = "CD_USER", id = "CAMPUSDUAL-USER")]
    user: Option<String>,
    #[arg(short, long, env = "CD_PASSWORD", id = "CD-PASSWORD")]
    password: Option<String>,
    /// The Chat-ID which will receive CampusDual exam scores
    #[arg(short, long, env)]
    chatid: Option<i64>,
    /// Enable debug logging (very bloated amount){n}[SETS env: RUST_LOG=debug]
    #[arg(long)]
    debug: bool,
    /// Ollama API host for AI time parsing bloatware{n}Example: <http://127.0.0.1:11434/api>
    #[arg(long, env = "OLLAMA_HOST")]
    ollama_host: Option<String>,
    /// Ollama model for inference{n}Example: 'llama3:latest'
    #[arg(long, env = "OLLAMA_MODEL")]
    ollama_model: Option<String>,
}

#[tokio::main]
async fn main() {
    DB_FILENAME.set(STUWE_DB).unwrap();
    BACKEND.set(Backend::StuWe).unwrap();

    //// Args setup
    let args = Args::parse();
    OLLAMA_HOST.set(args.ollama_host).unwrap();
    OLLAMA_MODEL.set(args.ollama_model).unwrap();

    if args.user.is_some() && args.password.is_some() && args.chatid.is_some() {
        CD_DATA
            .set(CampusDualData {
                username: args.user.unwrap(),
                password: args.password.unwrap(),
                chat_id: args.chatid.unwrap(),
            })
            .unwrap();
    } else {
        log::info!("CampusDual support disabled")
    }

    if args.debug {
        env::set_var("RUST_LOG", "debug");
    } else if env::var(pretty_env_logger::env_logger::DEFAULT_FILTER_ENV).is_err() {
        dbg!();
        env::set_var("RUST_LOG", "info");
    }

    pretty_env_logger::init_timed();
    log::info!("Starting bot...");

    //// DB setup
    check_or_create_db_tables().unwrap();

    let mensen = get_mensen().await.unwrap();
    init_mensa_id_db(&mensen).unwrap();

    // always update cache on startup
    // log::info!(target: "stuwe_telegram_rs::TaskSched", "Updating cache...");
    // log::info!(target: "stuwe_telegram_rs::TaskSched", "Updating cache...");
    match update_cache().await {
        // Ok(_) => log::info!(target: "stuwe_telegram_rs::TaskSched", "Cache updated!"),
        Ok(_) => log::info!("Cache updated!"),
        // Err(e) => log::error!(target: "stuwe_telegram_rs::TaskSched", "Cache update failed: {}", e),
        Err(e) => log::error!("Cache update failed: {}", e),
    }

    let bot = Bot::new(args.token);

    let (jobhandler_task_tx, jobhandler_task_rx): JobHandlerTaskType = broadcast::channel(10);
    let (user_registration_data_tx, _): QueryRegistrationType = broadcast::channel(10);

    // every user has a mensa_id, but only users with auto send have a job_uuid inside RegistrEntry
    {
        let bot = bot.clone();
        // there is effectively only one tx and rx, however since rx cant be passed as dptree dep (?!),
        // tx has to be cloned and passed to both (inside command_handler it will be resubscribed to rx)
        let jobhandler_task_tx = jobhandler_task_tx.clone();
        let user_registration_data_tx = user_registration_data_tx.clone();
        tokio::spawn(async move {
            log::info!("Starting task scheduler...");
            run_task_scheduler(
                bot,
                jobhandler_task_tx,
                jobhandler_task_rx,
                user_registration_data_tx,
            )
            .await;
        });
    }

    // passing a receiver doesnt work for some reason, so sending user_registration_data_tx and resubscribing to get rx
    let command_handler_deps = dptree::deps![
        InMemStorage::<DialogueState>::new(),
        mensen,
        jobhandler_task_tx,
        user_registration_data_tx
    ];
    Dispatcher::builder(bot, schema())
        .dependencies(command_handler_deps)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

fn schema() -> UpdateHandler<Box<dyn std::error::Error + Send + Sync + 'static>> {
    use dptree::case;

    let command_handler = teloxide::filter_command::<Command, _>()
        .branch(dptree::case![Command::Start].endpoint(start))
        .branch(dptree::case![Command::Heute].endpoint(day_cmd))
        .branch(dptree::case![Command::Morgen].endpoint(day_cmd))
        .branch(dptree::case![Command::Uebermorgen].endpoint(day_cmd))
        .branch(dptree::case![Command::Andere].endpoint(show_different_mensa))
        .branch(dptree::case![Command::Subscribe].endpoint(subscribe))
        .branch(dptree::case![Command::Unsubscribe].endpoint(unsubscribe))
        .branch(dptree::case![Command::Mensa].endpoint(change_mensa))
        .branch(dptree::case![Command::Uhrzeit].endpoint(start_time_dialogue));

    let message_handler = Update::filter_message()
        .branch(command_handler)
        .branch(case![DialogueState::AwaitTimeReply].endpoint(reply_time_dialogue))
        .branch(dptree::endpoint(invalid_cmd));

    let callback_query_handler = Update::filter_callback_query().endpoint(callback_handler);

    dialogue::enter::<Update, InMemStorage<DialogueState>, DialogueState, _>()
        .branch(message_handler)
        .branch(callback_query_handler)
}

async fn run_task_scheduler(
    bot: Bot,
    jobhandler_task_tx: broadcast::Sender<JobHandlerTask>,
    mut jobhandler_task_rx: broadcast::Receiver<JobHandlerTask>,
    user_registration_data_tx: broadcast::Sender<Option<RegistrationEntry>>,
) {
    let sched = JobScheduler::new().await.unwrap();

    start_mensacache_and_campusdual_job(bot.clone(), &sched, jobhandler_task_tx.clone()).await;

    let mut loaded_user_data: BTreeMap<i64, RegistrationEntry> = BTreeMap::new();
    load_jobs_from_db(
        &bot,
        &sched,
        &mut loaded_user_data,
        jobhandler_task_tx.clone(),
    )
    .await;

    // start scheduler (non blocking)
    sched.start().await.unwrap();

    // log::info!(target: "stuwe_telegram_rs::TaskSched", "Ready.");
    log::info!("Ready.");

    // receive job update msg (register/unregister/check existence)
    while let Ok(job_handler_task) = jobhandler_task_rx.recv().await {
        match job_handler_task.job_type {
            JobType::Register => {
                handle_add_registration_task(
                    &bot,
                    job_handler_task,
                    &sched,
                    &mut loaded_user_data,
                    jobhandler_task_tx.clone(),
                )
                .await;
            }

            JobType::UpdateRegistration => {
                handle_update_registration_task(
                    &bot,
                    job_handler_task,
                    &sched,
                    &mut loaded_user_data,
                    jobhandler_task_tx.clone(),
                )
                .await;
            }

            JobType::DeleteRegistration => {
                handle_delete_registration_task(job_handler_task, &sched, &mut loaded_user_data)
                    .await;
            }

            JobType::QueryRegistration => {
                handle_query_registration_task(
                    job_handler_task,
                    &mut loaded_user_data,
                    user_registration_data_tx.clone(),
                )
                .await;
            }

            JobType::BroadcastUpdate => {
                handle_broadcast_update_task(&bot, job_handler_task, &mut loaded_user_data).await;
            }
        }
    }
}
