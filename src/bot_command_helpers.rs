use crate::constants::{NO_DB_MSG, OLLAMA_HOST, OLLAMA_MODEL};
use crate::data_types::{
    HandlerResult, MensaKeyboardAction, ParsedTimeAndLastMsgFromDialleougueue, TimeParseError,
};
use crate::shared_main::{get_user_registration, make_mensa_keyboard};

use regex_lite::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use lazy_static::lazy_static;
use teloxide::types::InputFile;

use std::collections::BTreeMap;
use teloxide::prelude::*;

pub async fn mensa_disp_or_upd(
    bot: Bot,
    msg: Message,
    mensen: BTreeMap<u32, String>,
    disp_or_update: MensaKeyboardAction,
) -> HandlerResult {
    if get_user_registration(msg.chat.id.0).is_some() {
        let keyboard = make_mensa_keyboard(mensen, disp_or_update);
        bot.send_message(msg.chat.id, "Mensa auswählen:")
            .reply_markup(keyboard)
            .await?;
    } else {
        bot.send_message(msg.chat.id, NO_DB_MSG).await?;
    }
    Ok(())
}

pub async fn parse_time_send_status_msgs(
    bot: &Bot,
    chatid: ChatId,
    txt: Option<&str>,
) -> Result<ParsedTimeAndLastMsgFromDialleougueue, TimeParseError> {
    if txt.is_none() {
        bot.send_message(chatid, "Bitte mit Uhrzeit antworten:")
            .await
            .unwrap();
        return Err(TimeParseError::NoTimeArgPassed);
    }

    if let Ok((hour, minute)) = rex_parse_time(txt.unwrap()) {
        let msgtxt = format!(
            "Plan wird ab jetzt automatisch an Wochentagen {:02}:{:02} Uhr \
        gesendet.\n\n/unsubscribe zum Deaktivieren",
            hour, minute
        );

        bot.send_message(chatid, msgtxt).await.unwrap();

        Ok(ParsedTimeAndLastMsgFromDialleougueue {
            hour,
            minute,
            msgid: None,
        })
    } else {
        // try ai
        let oracle_msgid = bot
            .send_message(chatid, "Das Orakel wird befragt...")
            .await
            .unwrap()
            .id;

        let parse_result = ai_parse_time(txt.unwrap()).await;

        match parse_result {
            Ok(parsed) => {
                let msgtxt = format!(
                    "Plan wird ab jetzt automatisch an Wochentagen {:02}:{:02} Uhr \
                gesendet.\n\n/unsubscribe zum Deaktivieren",
                    parsed.0, parsed.1
                );

                bot.edit_message_text(chatid, oracle_msgid, msgtxt)
                    .await
                    .unwrap();

                Ok(ParsedTimeAndLastMsgFromDialleougueue {
                    hour: parsed.0,
                    minute: parsed.1,
                    msgid: Some(oracle_msgid),
                })
            }
            Err(TimeParseError::InvalidTimePassed) => {
                bot.edit_message_text(
                    chatid,
                    oracle_msgid,
                    "Das Orakel verharrt in Ungewissheit.\nBitte erneut:",
                )
                .await
                .unwrap();

                Err(TimeParseError::InvalidTimePassed)
            }
            Err(TimeParseError::OllamaUnavailable) => {
                bot.edit_message_text(
                    chatid,
                    oracle_msgid,
                    "Das Orakel erhört unsere Bitten nicht.\nBitte im Format HH:mm antworten:",
                )
                .await
                .unwrap();

                Err(TimeParseError::OllamaUnavailable)
            }
            Err(TimeParseError::OllamaUnconfigured) => {
                bot.edit_message_text(
                    chatid,
                    oracle_msgid,
                    "Eingegebene Zeit ist ungültig.\nBitte mit Uhrzeit antworten:",
                )
                .await
                .unwrap();

                Err(TimeParseError::OllamaUnconfigured)
            }
            Err(TimeParseError::NoTimeArgPassed) => {
                unreachable!()
            }
        }
    }
}
async fn ai_parse_time(txt: &str) -> Result<(u32, u32), TimeParseError> {
    let prompt = format!(
        "I am looking for a time. If you can guess which one I mean, reply in 24 hour format with a colon like \"HH:mm\", \
         else 'null'. The user uses 24 hour format. NO EXPLANATION. Input: {}",
        txt
    );

    let ollama_host = OLLAMA_HOST.get().unwrap();
    let ollama_model = OLLAMA_MODEL.get().unwrap();

    if ollama_host.is_none() || ollama_model.is_none() {
        log::warn!("Ollama API is unconfigured, cannot fancy-parse time");
        return Err(TimeParseError::OllamaUnconfigured);
    }

    let client = reqwest::Client::new();
    let params = json!(
        {
            "model": ollama_model.as_ref().unwrap(),
            "prompt": &prompt,
            "stream": false,
            "keep_alive": -1
        }
    );

    log::info!("AI Query: '{}'", prompt);

    let res = client
        .post(format!("{}/generate", ollama_host.as_ref().unwrap()))
        .body(params.to_string())
        .send()
        .await
        .and_then(|r| r.error_for_status());

    if let Ok(res) = res {
        let txt = res.text().await.unwrap();

        #[derive(Serialize, Deserialize, Debug)]
        struct LlamaResponse {
            response: String,
            done: bool,
            context: Vec<i64>,
        }

        let struct_ai_resp: LlamaResponse = serde_json::from_str(&txt).unwrap();

        log::info!("AI Response: '{}'", struct_ai_resp.response);
        rex_parse_time(&struct_ai_resp.response)
    } else {
        log::warn!("Ollama API unavailable: {}", res.err().unwrap());
        Err(TimeParseError::OllamaUnavailable)
    }
}

fn rex_parse_time(txt: &str) -> Result<(u32, u32), TimeParseError> {
    lazy_static! {
        static ref RE: Regex = Regex::new("^([01]?[0-9]|2[0-3]):([0-5][0-9])").unwrap();
    }
    if !RE.is_match(txt) {
        Err(TimeParseError::InvalidTimePassed)
    } else {
        let caps = RE.captures(txt).unwrap();

        let hour = caps.get(1).unwrap().as_str().parse::<u32>().unwrap();
        let minute = caps.get(2).unwrap().as_str().parse::<u32>().unwrap();

        Ok((hour, minute))
    }
}

pub async fn send_bloat_image(bot: &Bot, chatid: ChatId) {
    let img = InputFile::file("documentation.jpeg");
    bot.send_photo(chatid, img).await.unwrap();
}
