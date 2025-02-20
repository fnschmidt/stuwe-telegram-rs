// GEANT_OV_RSA_CA_4_tcs-cert3.pem has to be properly set up, eg. in /etc/ssl/certs for Debian
// (the container image already has it)
use stuwe_telegram_rs::data_types::HtwkData;

use stuwe_telegram_rs::bot_command_handlers::{
    allergene, change_mensa, day_cmd, invalid_cmd, reply_time_dialogue, senddiff,
    show_different_mensa, start, start_time_dialogue, subscribe, unsubscribe,
};
use stuwe_telegram_rs::constants::{
    API_URL, BACKEND, CD_DATA, DB_FILENAME, OLLAMA_HOST, OLLAMA_MODEL, STUWE_DB, USER_REGISTRATIONS,
};
use stuwe_telegram_rs::data_backend::stuwe_parser::get_mensen;
use stuwe_telegram_rs::data_types::{
    Backend, Command, DialogueState, JobHandlerTask, JobHandlerTaskType, JobType,
};
use stuwe_telegram_rs::db_operations::check_or_create_db_tables;
use stuwe_telegram_rs::shared_main::callback_handler;
use stuwe_telegram_rs::task_scheduler_funcs::{
    handle_add_registration_task, handle_broadcast_update_task, handle_delete_registration_task,
    handle_update_registration_task, load_jobs_from_db, start_mensaupd_hook_and_campusdual_job,
};

use clap::Parser;
use std::env;
use std::sync::RwLock;
use teloxide::{
    dispatching::{
        UpdateHandler,
        dialogue::{self, InMemStorage},
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
    #[arg(short, long, env)]
    api_url: String,
    #[arg(short, long, env = "HTWK_USER", id = "HTWK-USER")]
    user: Option<String>,
    #[arg(short, long, env = "HTWK_PASSWORD", id = "HTWK-PASSWORD")]
    password: Option<String>,
    /// HTWK-API host for getting grades{n}Example: <http://htwk-api:8080>
    #[arg(long, env = "HTWK_API_HOST")]
    htwk_host: Option<String>,
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
    API_URL.set(args.api_url).unwrap();
    OLLAMA_HOST.set(args.ollama_host).unwrap();
    OLLAMA_MODEL.set(args.ollama_model).unwrap();

    if args.user.is_some()
        && args.password.is_some()
        && args.chatid.is_some()
        && args.htwk_host.is_some()
    {
        CD_DATA
            .set(HtwkData {
                username: args.user.unwrap(),
                password: args.password.unwrap(),
                chat_id: args.chatid.unwrap(),
                htwk_api_url: args.htwk_host.unwrap(),
            })
            .unwrap();
    } else {
        log::info!("CampusDual support disabled")
    }

    if args.debug {
        pretty_env_logger::formatted_timed_builder()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else if env::var(pretty_env_logger::env_logger::DEFAULT_FILTER_ENV).is_err() {
        pretty_env_logger::formatted_timed_builder()
            .filter_level(log::LevelFilter::Info)
            .init();
    } else {
        pretty_env_logger::init_timed();
    }

    log::info!("Starting bot...");

    //// DB setup
    check_or_create_db_tables().unwrap();

    let mensen = get_mensen().await;

    let bot = Bot::new(args.token);

    let (jobhandler_task_tx, jobhandler_task_rx): JobHandlerTaskType = broadcast::channel(10);

    // every user has a mensa_id, but only users with auto send have a job_uuid inside RegistrEntry
    {
        let bot = bot.clone();
        // there is effectively only one tx and rx, however since rx cant be passed as dptree dep (?!),
        // tx has to be cloned and passed to both (inside command_handler it will be resubscribed to rx)
        let jobhandler_task_tx = jobhandler_task_tx.clone();
        tokio::spawn(async move {
            log::info!("Starting task scheduler...");
            run_task_scheduler(bot, jobhandler_task_tx, jobhandler_task_rx).await;
        });
    }

    // passing a receiver doesnt work for some reason, so sending user_registration_data_tx and resubscribing to get rx
    let command_handler_deps = dptree::deps![
        InMemStorage::<DialogueState>::new(),
        mensen,
        jobhandler_task_tx
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
        .branch(dptree::case![Command::Übermorgen].endpoint(day_cmd))
        .branch(dptree::case![Command::Andere].endpoint(show_different_mensa))
        .branch(dptree::case![Command::Subscribe].endpoint(subscribe))
        .branch(dptree::case![Command::Unsubscribe].endpoint(unsubscribe))
        .branch(dptree::case![Command::Mensa].endpoint(change_mensa))
        .branch(dptree::case![Command::Allergene].endpoint(allergene))
        .branch(dptree::case![Command::Diff].endpoint(senddiff))
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
) {
    let sched = JobScheduler::new().await.unwrap();

    start_mensaupd_hook_and_campusdual_job(bot.clone(), &sched, jobhandler_task_tx.clone()).await;

    let user_registrations = load_jobs_from_db(&bot, &sched).await;
    USER_REGISTRATIONS
        .set(RwLock::new(user_registrations))
        .unwrap();

    // start scheduler (non blocking)
    sched.start().await.unwrap();

    log::info!("Ready.");

    // receive job update msg (register/unregister/check existence)
    while let Ok(job_handler_task) = jobhandler_task_rx.recv().await {
        match job_handler_task.job_type {
            JobType::Register => {
                handle_add_registration_task(&bot, job_handler_task, &sched).await;
            }

            JobType::UpdateRegistration => {
                handle_update_registration_task(&bot, job_handler_task, &sched).await;
            }

            JobType::DeleteRegistration => {
                handle_delete_registration_task(job_handler_task, &sched).await;
            }

            JobType::BroadcastUpdate => {
                handle_broadcast_update_task(&bot, job_handler_task).await;
            }
        }
    }
}
