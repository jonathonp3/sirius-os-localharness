//! Persistent scheduled automation tasks.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{Datelike, Duration, Local, LocalResult, NaiveTime, TimeZone, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationTask {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub frequency: String,
    pub time: String,
    #[serde(default)]
    pub weekday: Option<u32>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub ended: bool,
    pub created_at: i64,
    #[serde(default)]
    pub last_run: Option<i64>,
    #[serde(default)]
    pub next_run: Option<i64>,
    #[serde(default)]
    pub last_result: Option<String>,
}

pub async fn load(path: &Path) -> Result<Vec<AutomationTask>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = tokio::fs::read_to_string(path).await?;
    let tasks: Vec<AutomationTask> =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(tasks)
}

pub async fn save(path: &Path, tasks: &[AutomationTask]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temp = path.with_extension("json.tmp");
    tokio::fs::write(&temp, serde_json::to_string_pretty(tasks)? + "\n").await?;
    tokio::fs::rename(&temp, path).await?;
    Ok(())
}

fn parse_time(value: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(value, "%H:%M").with_context(|| format!("invalid time `{value}`"))
}

fn weekday_from_number(value: u32) -> Option<Weekday> {
    match value {
        1 => Some(Weekday::Mon),
        2 => Some(Weekday::Tue),
        3 => Some(Weekday::Wed),
        4 => Some(Weekday::Thu),
        5 => Some(Weekday::Fri),
        6 => Some(Weekday::Sat),
        7 => Some(Weekday::Sun),
        _ => None,
    }
}

pub fn next_run_at(frequency: &str, time: &str, weekday: Option<u32>, after: chrono::DateTime<Local>) -> Result<i64> {
    let time = parse_time(time)?;
    let candidate = |date: chrono::NaiveDate| -> Option<chrono::DateTime<Local>> {
        match Local.from_local_datetime(&date.and_time(time)) {
            LocalResult::Single(value) => Some(value),
            LocalResult::Ambiguous(first, _) => Some(first),
            LocalResult::None => None,
        }
    };
    let mut date = after.date_naive();
    let mut next = candidate(date).context("local time does not exist")?;
    if next <= after {
        date += Duration::days(1);
        next = candidate(date).context("local time does not exist")?;
    }
    if frequency == "weekly" {
        let target = weekday_from_number(weekday.unwrap_or(1)).context("invalid weekday")?;
        let mut guard = 0;
        while next.weekday() != target {
            date += Duration::days(1);
            next = candidate(date).context("local time does not exist")?;
            guard += 1;
            if guard > 8 {
                bail!("could not find weekly slot");
            }
        }
    }
    Ok(next.timestamp())
}

pub fn initial_next_run(task: &AutomationTask) -> Result<i64> {
    next_run_at(&task.frequency, &task.time, task.weekday, Local::now())
}

pub fn validate(frequency: &str, time: &str, weekday: Option<u32>) -> Result<()> {
    if frequency != "daily" && frequency != "weekly" {
        bail!("frequency must be daily or weekly");
    }
    parse_time(time)?;
    if frequency == "weekly" && weekday_from_number(weekday.unwrap_or(0)).is_none() {
        bail!("weekly tasks need a weekday 1-7");
    }
    Ok(())
}

pub fn create(title: String, prompt: String, frequency: String, time: String, weekday: Option<u32>) -> Result<AutomationTask> {
    let title = title.trim().to_string();
    let prompt = prompt.trim().to_string();
    if title.is_empty() || prompt.is_empty() {
        bail!("title and prompt are required");
    }
    validate(&frequency, &time, weekday)?;
    let now = Local::now();
    let mut task = AutomationTask {
        id: format!("task-{}", now.timestamp_millis()),
        title,
        prompt,
        frequency,
        time,
        weekday,
        paused: false,
        ended: false,
        created_at: now.timestamp(),
        last_run: None,
        next_run: None,
        last_result: None,
    };
    task.next_run = Some(initial_next_run(&task)?);
    Ok(task)
}
