use anyhow::{Context, Result};
use scraper::{Html, Selector};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
};

use crate::data_types::stuwe_data_types::{
    CampusDualError, CampusDualGrade, CampusDualSignupOption,
};

async fn extract_grades(html_text: String) -> Result<Vec<CampusDualGrade>> {
    let mut grades = Vec::new();

    let document = Html::parse_document(&html_text);
    let table = document
        .select(&Selector::parse("#acwork tbody").unwrap())
        .next()
        .context("CD grades page: #acwork tbody missing")?;
    let top_level_line_selector = Selector::parse(".child-of-node-0").unwrap();
    let top_level_lines = table.select(&top_level_line_selector);
    for line in top_level_lines {
        let l_id = line
            .value()
            .attr("id")
            .context("CD: grades table line has no ID")?;
        let content_selector = &Selector::parse("td").unwrap();
        let mut content = line.select(content_selector);
        let name = content.next().unwrap().text().next().unwrap();
        let grade = content.next().unwrap().text().next().unwrap();

        let subline_selector = &Selector::parse(&format!(".child-of-{}", l_id)).unwrap();
        let sub_count = table.select(subline_selector).count();

        grades.push(CampusDualGrade {
            name: name.to_string(),
            grade: grade.to_string(),
            subgrades: sub_count,
        });
    }

    Ok(grades)
}

pub async fn get_campusdual_data(
    uname: String,
    password: String,
) -> Result<(Vec<CampusDualGrade>, Vec<CampusDualSignupOption>)> {
    let client = reqwest::Client::builder().cookie_store(true).build()?;

    let resp = client
        .get("https://erp.campus-dual.de/sap/bc/webdynpro/sap/zba_initss?sap-client=100&sap-language=de&uri=https://selfservice.campus-dual.de/index/login")
        .send()
        .await?
        .error_for_status()?;

    let xsrf = {
        let document = Html::parse_document(&resp.text().await?);
        document
            .select(&Selector::parse(r#"input[name="sap-login-XSRF"]"#).unwrap())
            .next()
            .context("CD login stage 1: XSRF token missing")?
            .value()
            .attr("value")
            .unwrap()
            .to_string()
    };

    let form = [
        ("sap-user", uname),
        ("sap-password", password),
        ("sap-login-XSRF", xsrf),
    ];

    let resp = client
        .post("https://erp.campus-dual.de/sap/bc/webdynpro/sap/zba_initss?uri=https%3a%2f%2fselfservice.campus-dual.de%2findex%2flogin&sap-client=100&sap-language=DE")
        .form(&form)
        .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/117.0.0.0 Safari/537.36")
        .send()
        .await?
        .error_for_status()?;

    // check if title of redirect page implicates successful login
    {
        let zba_init_doc = Html::parse_document(&resp.text().await.unwrap());
        match zba_init_doc
            .select(&Selector::parse("title").unwrap())
            .next()
            .unwrap()
            .inner_html()
            .as_str()
        {
            "Anmeldung" => return Err(CampusDualError::CdBadCredentials.into()),
            "Initialisierung Selfservices" => {}
            _ => log::warn!("Unexpected redirect, treating as success"),
        }
    }

    let grade_resp = client
        .get("https://selfservice.campus-dual.de/acwork/index")
        .send()
        .await?;
    log::debug!("get exams: {}", grade_resp.status());

    let grades = extract_grades(grade_resp.text().await.unwrap()).await?;

    let exam_signup_resp = client
        .get("https://selfservice.campus-dual.de/acwork/expproc")
        .send()
        .await
        .unwrap();

    println!("get exam reg options: {}", exam_signup_resp.status());

    let signup_options =
        extract_exam_registr_options(exam_signup_resp.text().await.unwrap()).await?;

    Ok((grades, signup_options))
}

pub async fn compare_campusdual_grades(
    recv_grades: &Vec<CampusDualGrade>,
) -> Option<Vec<CampusDualGrade>> {
    let mut f = match File::open("grades.json").await {
        Ok(f) => f,
        Err(_) => {
            let mut f = File::create("grades.json").await.unwrap();
            let json_str = serde_json::to_string(recv_grades).unwrap();
            f.write_all(json_str.as_bytes()).await.unwrap();
            return Some(recv_grades.to_vec());
        }
    };
    let mut new_grades = vec![];

    let mut old_grades_str = String::new();
    f.read_to_string(&mut old_grades_str).await.unwrap();
    let old_grades: Vec<CampusDualGrade> = serde_json::from_str(&old_grades_str).unwrap();

    for grade in recv_grades {
        if !old_grades.contains(grade) {
            new_grades.push(grade.clone());
        }
    }

    if new_grades.is_empty() {
        None
    } else {
        Some(new_grades)
    }
}

pub async fn compare_campusdual_signup_options(
    recv_options: &Vec<CampusDualSignupOption>,
) -> Option<Vec<CampusDualSignupOption>> {
    let mut f = match File::open("signup_options.json").await {
        Ok(f) => f,
        Err(_) => {
            let mut f = File::create("signup_options.json").await.unwrap();
            let json_str = serde_json::to_string(recv_options).unwrap();
            f.write_all(json_str.as_bytes()).await.unwrap();
            return Some(recv_options.to_vec());
        }
    };
    let mut new_options = vec![];

    let mut old_options_str = String::new();
    f.read_to_string(&mut old_options_str).await.unwrap();
    let old_options: Vec<CampusDualSignupOption> = serde_json::from_str(&old_options_str).unwrap();

    for option in recv_options {
        if !old_options.contains(option) {
            new_options.push(option.clone());
        }
    }

    if new_options.is_empty() {
        None
    } else {
        Some(new_options)
    }
}

async fn extract_exam_registr_options(html_text: String) -> Result<Vec<CampusDualSignupOption>> {
    let mut signup_options = Vec::new();

    let document = Html::parse_document(&html_text);
    let table = document
        .select(&Selector::parse("#expproc tbody").unwrap())
        .next()
        .unwrap();
    let top_level_line_selector = Selector::parse(".child-of-node-0").unwrap();
    let top_level_lines = table.select(&top_level_line_selector);
    for line in top_level_lines {
        let l_id = line.value().attr("id").unwrap();
        let content_selector = &Selector::parse("td").unwrap();
        let mut content = line.select(content_selector);
        let class = content.next().unwrap().text().next().unwrap();
        let verfahren = content.next().unwrap().text().next().unwrap(); // .inner_html();

        let subline_selector = &Selector::parse(&format!(".child-of-{}", l_id)).unwrap();
        let status_icon_url = table
            .select(subline_selector)
            .next()
            .unwrap()
            .select(&Selector::parse("img").unwrap())
            .next()
            .unwrap()
            .value()
            .attr("src")
            .unwrap();

        let status = match status_icon_url {
            "/images/missed.png" => "🚫",
            "/images/yellow.png" => "📝",
            "/images/exclamation.jpg" => "⚠️",
            _ => "???",
        };

        signup_options.push(CampusDualSignupOption {
            name: class.to_string(),
            verfahren: verfahren.to_string(),
            status: status.to_string(),
        });

        // println!("{} ({}) — {}", status, verfahren, name);
    }

    Ok(signup_options)
}

pub async fn save_campusdual_grades(recv_grades: &Vec<CampusDualGrade>) {
    let mut f = File::create("grades.json").await.unwrap();
    let json_str = serde_json::to_string(recv_grades).unwrap();
    f.write_all(json_str.as_bytes()).await.unwrap();
}

pub async fn save_campusdual_signup_options(recv_options: &Vec<CampusDualSignupOption>) {
    let mut f = File::create("signup_options.json").await.unwrap();
    let json_str = serde_json::to_string(recv_options).unwrap();
    f.write_all(json_str.as_bytes()).await.unwrap();
}
