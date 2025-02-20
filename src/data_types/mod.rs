pub mod mm_data_types;
pub mod stuwe_data_types;

use serde::{Deserialize, Serialize};
use stuwe_data_types::CanteenMealDiff;
use teloxide::{
    dispatching::dialogue::InMemStorage, prelude::Dialogue, types::MessageId,
    utils::command::BotCommands,
};
use thiserror::Error;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Backend {
    MensiMates,
    StuWe,
}

#[derive(BotCommands, Clone, Debug, PartialEq)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    #[command(description = "Gerichte für heute")]
    Heute,
    #[command(description = "Gerichte für morgen\n")]
    Morgen,
    #[command(hide)]
    Übermorgen,
    #[command(description = "Andere Mensa anzeigen")]
    Mensa,
    #[command(description = "Mensa wechseln\n")]
    Andere,
    #[command(description = "autom. Nachrichten aktivieren")]
    Subscribe,
    #[command(description = "autom. Nachrichten deaktivieren")]
    Unsubscribe,
    #[command(description = "Sendezeitpunkt ändern")]
    Uhrzeit,
    #[command(hide)]
    Allergene,
    #[command(description = "Bei Planänderungen nur Unterschied schicken")]
    Diff,
    #[command(hide)]
    Start,
}

#[derive(Clone, Default)]
pub enum DialogueState {
    #[default]
    Default,
    AwaitTimeReply,
}

pub type DialogueType = Dialogue<DialogueState, InMemStorage<DialogueState>>;
pub type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub type JobHandlerTaskType = (
    broadcast::Sender<JobHandlerTask>,
    broadcast::Receiver<JobHandlerTask>,
);

pub enum MensaKeyboardAction {
    Register,
    Update,
    DisplayOnce,
}

// used internally for teloxide/Telegram bot
#[derive(Debug, Clone)]
pub enum JobType {
    Register,
    DeleteRegistration,
    UpdateRegistration,
    BroadcastUpdate,
}

#[derive(Debug, Clone)]
pub struct JobHandlerTask {
    pub job_type: JobType,
    pub chat_id: Option<i64>,
    pub mensa_id: Option<u32>,
    pub hour: Option<u32>,
    pub minute: Option<u32>,
    pub meals_diff: Option<CanteenMealDiff>,
}

pub struct RegisterTask {
    pub chat_id: i64,
    pub mensa_id: u32,
    pub hour: u32,
    pub minute: u32,
}
impl From<RegisterTask> for JobHandlerTask {
    fn from(job: RegisterTask) -> Self {
        JobHandlerTask {
            job_type: JobType::Register,
            chat_id: Some(job.chat_id),
            mensa_id: Some(job.mensa_id),
            hour: Some(job.hour),
            minute: Some(job.minute),
            meals_diff: None,
        }
    }
}

pub struct UnregisterTask {
    pub chat_id: i64,
}
impl From<UnregisterTask> for JobHandlerTask {
    fn from(job: UnregisterTask) -> Self {
        JobHandlerTask {
            job_type: JobType::DeleteRegistration,
            chat_id: Some(job.chat_id),
            mensa_id: None,
            hour: None,
            minute: None,
            meals_diff: None,
        }
    }
}

#[derive(Clone)]
pub struct UpdateRegistrationTask {
    pub chat_id: i64,
    pub mensa_id: Option<u32>,
    pub hour: Option<u32>,
    pub minute: Option<u32>,
}
impl From<UpdateRegistrationTask> for JobHandlerTask {
    fn from(job: UpdateRegistrationTask) -> Self {
        JobHandlerTask {
            job_type: JobType::UpdateRegistration,
            chat_id: Some(job.chat_id),
            mensa_id: job.mensa_id,
            hour: job.hour,
            minute: job.minute,
            meals_diff: None,
        }
    }
}

pub struct BroadcastUpdateTask {
    pub meals_diff: CanteenMealDiff,
}
impl From<BroadcastUpdateTask> for JobHandlerTask {
    fn from(job: BroadcastUpdateTask) -> Self {
        JobHandlerTask {
            job_type: JobType::BroadcastUpdate,
            chat_id: None,
            mensa_id: None,
            hour: None,
            minute: None,
            meals_diff: Some(job.meals_diff),
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct RegistrationEntry {
    pub job_uuid: Option<Uuid>,
    pub mensa_id: u32,
    pub hour: Option<u32>,
    pub minute: Option<u32>,
    pub allergens: bool,
    pub senddiff: bool,
}

#[derive(Error, Debug, Clone)]
pub enum TimeParseError {
    #[error("Zeit konnte nicht gelesen werden")]
    InvalidTimePassed,
    #[error("Ollama API ist nicht erreichbar")]
    OllamaUnavailable,
    #[error("Ollama API ist nicht konfiguriert")]
    OllamaUnconfigured,
    #[error("Kein optionaler Zeitparam. bei Dialogstart")]
    NoTimeArgPassed,
}
#[derive(Debug, Clone)]
pub struct ParsedTimeAndLastMsgFromDialleougueue {
    pub hour: u32,
    pub minute: u32,
    pub msgid: Option<MessageId>,
}

#[derive(Debug, Clone)]
pub struct HtwkData {
    pub username: String,
    pub password: String,
    pub chat_id: i64,
    pub htwk_api_url: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct GradeTable {
    pub stammdaten: Stammdaten,
    pub title: String,
    pub semester: String,
    pub grades: Vec<Grade>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Stammdaten {
    pub name: String,
    pub matnr: String,
    pub semester_group: String,
    pub abschluss: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Grade {
    pub pruefungsnr: String,
    pub pruefungstext: String,
    pub ects: Option<String>,
    pub grade: Option<String>,
}
