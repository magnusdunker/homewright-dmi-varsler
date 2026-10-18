//! DMI's warnings as CAP alerts in MeteoAlarm's JSON feed for Denmark, and
//! which of them are in force for a set of areas.
//!
//! The feed keeps a few days of messages, so it is read as a history:
//!
//! - A message that a later one references (an `Update` or `Cancel`) is replaced.
//! - For each kind of warning and area, the newest message counts. DMI sends a
//!   green "no warnings" message for every area and kind each morning, which
//!   ends what was before it.
//! - DMI sets `expires` to the time it sent the message, so `expires` only
//!   ends a warning when it is later than `sent`. `ends` does, when present.
//! - A message sent after the time asked about is left out.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Duration, FixedOffset, Utc};
use serde_json::{json, Value};

pub const FEED_URL: &str = "https://feeds.meteoalarm.org/api/v1/warnings/feeds-denmark";

#[derive(Clone, Debug)]
pub struct Alert {
    pub id: String,
    pub sent: DateTime<FixedOffset>,
    pub msg_type: String,
    /// Identifiers of the messages this one replaces.
    pub references: Vec<String>,
    /// MeteoAlarm's awareness type: 1 wind, 2 snow and ice, 3 thunderstorm,
    /// 4 fog, 5 high temperature, 7 coastal event, 10 rain, 12 flooding ...
    pub kind: u32,
    /// The kind in Danish, e.g. "Forhøjet vandstand".
    pub kind_name: String,
    /// 1 green, 2 yellow, 3 orange, 4 red.
    pub level: u8,
    pub headline: String,
    pub description: String,
    pub instruction: String,
    pub onset: Option<DateTime<FixedOffset>>,
    pub ends: Option<DateTime<FixedOffset>>,
    pub expires: Option<DateTime<FixedOffset>>,
    /// EMMA_ID and the area's name in the message.
    pub areas: Vec<(String, String)>,
}

fn time(v: &Value) -> Option<DateTime<FixedOffset>> {
    v.as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok())
}

fn param<'a>(info: &'a Value, name: &str) -> Option<&'a str> {
    info["parameter"].as_array()?.iter().find(|p| p["valueName"] == name)?["value"].as_str()
}

/// The number before the first `;` in "2; yellow; Moderate".
fn leading_number(s: &str) -> Option<u32> {
    s.split(';').next()?.trim().parse().ok()
}

/// "Moderate Forhøjet vandstand" without the severity in front.
fn strip_severity(event: &str) -> String {
    for s in ["Minor ", "Moderate ", "Severe ", "Extreme ", "Unknown "] {
        if let Some(rest) = event.strip_prefix(s) {
            return rest.to_string();
        }
    }
    event.to_string()
}

pub fn level_name(level: u8) -> &'static str {
    match level {
        2 => "yellow",
        3 => "orange",
        4 => "red",
        _ => "green",
    }
}

pub fn level_name_da(level: u8) -> &'static str {
    match level {
        2 => "gul",
        3 => "orange",
        4 => "rød",
        _ => "grøn",
    }
}

/// The alerts in the feed, with the Danish text where there is one.
pub fn parse(feed: &Value) -> Vec<Alert> {
    let Some(list) = feed["warnings"].as_array() else { return vec![] };
    list.iter().filter_map(|w| parse_alert(&w["alert"])).collect()
}

fn parse_alert(a: &Value) -> Option<Alert> {
    if a["status"].as_str() != Some("Actual") {
        return None;
    }
    let infos = a["info"].as_array()?;
    let info = infos.iter().find(|i| i["language"].as_str().is_some_and(|l| l.starts_with("da"))).or_else(|| infos.first())?;
    let references = a["references"]
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|r| r.split(',').nth(1))
        .map(String::from)
        .collect();
    let kind = param(info, "awareness_type").and_then(leading_number)?;
    let level = param(info, "awareness_level").and_then(leading_number).and_then(|l| u8::try_from(l).ok())?;
    let text = |k: &str| info[k].as_str().unwrap_or_default().trim().to_string();
    let mut instruction = text("instruction");
    if instruction.is_empty() {
        // "." when there is nothing to say
        instruction = param(info, "impacts").unwrap_or_default().trim().to_string();
        if instruction == "." {
            instruction.clear();
        }
    }
    let areas = info["area"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|ar| {
            let desc = ar["areaDesc"].as_str().unwrap_or_default().to_string();
            ar["geocode"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|g| g["valueName"] == "EMMA_ID")
                .filter_map(move |g| g["value"].as_str().map(|c| (c.to_string(), desc.clone())))
        })
        .collect();
    Some(Alert {
        id: a["identifier"].as_str()?.to_string(),
        sent: time(&a["sent"])?,
        msg_type: a["msgType"].as_str().unwrap_or("Alert").to_string(),
        references,
        kind,
        kind_name: strip_severity(info["event"].as_str().unwrap_or_default()),
        level,
        headline: text("headline"),
        description: text("description"),
        instruction,
        onset: time(&info["onset"]).or_else(|| time(&info["effective"])),
        ends: time(&info["ends"]),
        expires: time(&info["expires"]),
        areas,
    })
}

/// A warning in force or coming, for the chosen areas. Messages that say the
/// same for several areas are one warning.
#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    /// The identifiers of the messages behind it.
    pub ids: Vec<String>,
    pub kind: u32,
    pub kind_name: String,
    pub level: u8,
    pub headline: String,
    pub description: String,
    pub instruction: String,
    pub onset: Option<DateTime<FixedOffset>>,
    pub ends: Option<DateTime<FixedOffset>>,
    pub sent: DateTime<FixedOffset>,
    /// The areas' names as the messages give them.
    pub areas: Vec<String>,
}

impl Warning {
    pub fn in_effect(&self, now: DateTime<Utc>) -> bool {
        self.onset.is_none_or(|o| o <= now)
    }

    pub fn to_json(&self, now: DateTime<Utc>) -> Value {
        json!({
            "type": self.kind_name,
            "type_id": self.kind,
            "level": level_name(self.level),
            "level_da": level_name_da(self.level),
            "headline": self.headline,
            "description": self.description,
            "instruction": self.instruction,
            "onset": self.onset.map(|t| t.to_rfc3339()),
            "ends": self.ends.map(|t| t.to_rfc3339()),
            "in_effect": self.in_effect(now),
            "areas": self.areas,
            "sent": self.sent.to_rfc3339(),
            "id": self.ids.first(),
        })
    }
}

/// Highest level first, then kind, onset and text.
type MergeKey = (u8, u32, Option<DateTime<FixedOffset>>, String, String);

fn ended(a: &Alert, now: DateTime<Utc>) -> bool {
    a.ends.is_some_and(|e| e <= now) || a.expires.is_some_and(|e| e > a.sent && e <= now)
}

/// The warnings in force or coming for `areas`, the highest level first.
pub fn current(alerts: &[Alert], areas: &[String], now: DateTime<Utc>) -> Vec<Warning> {
    // messages sent later than now (with a little slack for clocks) do not
    // exist yet, which matters when the feed is read for a time gone by
    let alerts: Vec<&Alert> = alerts.iter().filter(|a| a.sent <= now + Duration::minutes(5)).collect();
    let replaced: HashSet<&str> = alerts.iter().flat_map(|a| a.references.iter().map(String::as_str)).collect();
    // newest message per (kind, area)
    let mut newest: HashMap<(u32, &str), (&Alert, &str)> = HashMap::new();
    for a in alerts.iter().copied().filter(|a| !replaced.contains(a.id.as_str())) {
        for (code, desc) in &a.areas {
            if !areas.iter().any(|c| c == code) {
                continue;
            }
            let slot = newest.entry((a.kind, code.as_str())).or_insert((a, desc));
            if a.sent > slot.0.sent {
                *slot = (a, desc);
            }
        }
    }
    // one warning per text, with the areas it covers
    let mut merged: BTreeMap<MergeKey, Warning> = BTreeMap::new();
    for (a, desc) in newest.into_values() {
        if a.msg_type == "Cancel" || a.level < 2 || ended(a, now) {
            continue;
        }
        let key = (u8::MAX - a.level, a.kind, a.onset, a.headline.clone(), a.description.clone());
        let w = merged.entry(key).or_insert_with(|| Warning {
            ids: vec![],
            kind: a.kind,
            kind_name: a.kind_name.clone(),
            level: a.level,
            headline: a.headline.clone(),
            description: a.description.clone(),
            instruction: a.instruction.clone(),
            onset: a.onset,
            ends: a.ends,
            sent: a.sent,
            areas: vec![],
        });
        if !w.ids.contains(&a.id) {
            w.ids.push(a.id.clone());
        }
        if !w.areas.iter().any(|x| x == desc) {
            w.areas.push(desc.to_string());
        }
        w.sent = w.sent.max(a.sent);
    }
    let mut out: Vec<Warning> = merged.into_values().collect();
    for w in &mut out {
        w.ids.sort();
        w.areas.sort();
    }
    out
}

/// Every message a warning's messages replaced, directly or further back, and
/// the messages themselves.
pub fn lineage(alerts: &[Alert], w: &Warning) -> HashSet<String> {
    let refs: HashMap<&str, &[String]> = alerts.iter().map(|a| (a.id.as_str(), a.references.as_slice())).collect();
    let mut out: HashSet<String> = HashSet::new();
    let mut todo: Vec<String> = w.ids.clone();
    while let Some(id) = todo.pop() {
        if let Some(r) = refs.get(id.as_str()) {
            todo.extend(r.iter().filter(|x| !out.contains(*x)).cloned());
        }
        out.insert(id);
    }
    out
}
