//! DMI's weather warnings for a place in Denmark as an external integration
//! for HomewrightHQ.
//!
//! The warnings come from MeteoAlarm's feed for Denmark, where DMI publishes
//! them as CAP messages. `cap` reads the feed and works out which warnings
//! are in force, `areas` finds DMI's warning areas for a place, and this file
//! connects them to HomewrightHQ through the `thehome-plugin` SDK.

pub mod areas;
pub mod cap;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use thehome_plugin::{async_trait, json, Entity, Entry, Error, Flow, FlowReply, Host, Integration, Map, Value};

pub use cap::FEED_URL;

pub const DOMAIN: &str = "dmi_varsler";
pub const EVENT: &str = "dmi_varsler_warning";
const POLL: Duration = Duration::from_secs(600);
/// Failed fetches in a row before the entities become unavailable.
const FAILURES: u32 = 3;

/// Logging to HomewrightHQ's log through stderr.
#[doc(hidden)]
pub fn emit(level: &str, message: String) {
    thehome_plugin::log(level, message);
}

#[macro_export]
macro_rules! info {
    ($($t:tt)*) => { $crate::emit("INFO", format!($($t)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($t:tt)*) => { $crate::emit("WARN", format!($($t)*)) };
}

fn location_schema(lat: Option<f64>, lon: Option<f64>) -> Value {
    json!({
        "type": "object",
        "required": ["latitude", "longitude"],
        "properties": {
            "latitude": { "type": "number", "title": "Breddegrad", "default": lat },
            "longitude": { "type": "number", "title": "Længdegrad", "default": lon },
        }
    })
}

fn areas_schema(selected: &[String]) -> Value {
    let options: Vec<Value> = areas::all().iter().map(|a| json!({ "const": a.code, "title": a.title() })).collect();
    json!({
        "type": "object",
        "required": ["areas"],
        "properties": {
            "areas": {
                "type": "array", "title": "Varslingsområder", "items": { "type": "string", "oneOf": options }, "default": selected,
                "description": "DMI varsler for 8 områder på land og 20 langs kysten. Kystområderne har varsler om vandstand."
            }
        }
    })
}

fn str_list(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
        Value::String(s) => s.split([',', ' ']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect(),
        _ => vec![],
    }
}

/// The codes from a form, known ones only, in the areas' order.
fn chosen(input: &Value) -> Vec<String> {
    let picked = str_list(&input["areas"]);
    areas::all().iter().filter(|a| picked.contains(&a.code)).map(|a| a.code.clone()).collect()
}

fn title(codes: &[String]) -> String {
    let land: Vec<&str> = codes.iter().filter_map(|c| areas::get(c)).filter(|a| !a.is_coast()).map(|a| a.name.as_str()).collect();
    if land.is_empty() {
        "DMI-varsler".into()
    } else {
        format!("DMI-varsler {}", land.join(", "))
    }
}

fn form_with(step: &str, schema: Value, description: &str, errors: Option<Value>) -> FlowReply {
    let mut r = match errors {
        Some(e) => FlowReply::form_with_errors(step, schema, e),
        None => FlowReply::form(step, schema),
    };
    r.result["description"] = json!(description);
    r
}

/// The home's coordinates from `zone.home`.
async fn home(host: &Host) -> (Option<f64>, Option<f64>) {
    let s = host.get_state("zone.home").await.unwrap_or(Value::Null);
    (s["attributes"]["latitude"].as_f64(), s["attributes"]["longitude"].as_f64())
}

#[derive(Default)]
pub struct DmiVarsler {
    tasks: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}

#[async_trait]
impl Integration for DmiVarsler {
    async fn config_flow(&self, host: &Host, flow: Flow, step: &str, input: Option<Value>) -> Result<FlowReply, Error> {
        match (step, input) {
            ("user", None) => {
                let (lat, lon) = home(host).await;
                Ok(form_with("user", location_schema(lat, lon), "Stedet, varslerne skal gælde for. Udfyldt med hjemmets position.", None))
            }
            ("user", Some(v)) => {
                let (Some(lat), Some(lon)) = (v["latitude"].as_f64(), v["longitude"].as_f64()) else {
                    return Ok(FlowReply::form_with_errors("user", location_schema(None, None), json!({ "base": "invalid_location" })));
                };
                let found = areas::for_point(lat, lon);
                if found.is_empty() {
                    let r = form_with("user", location_schema(Some(lat), Some(lon)), "Stedet ligger ikke i Danmark.", Some(json!({ "base": "outside_denmark" })));
                    return Ok(r);
                }
                let selected: Vec<String> = found.iter().map(|s| s.to_string()).collect();
                let mut data = flow.data.clone();
                data.insert("latitude".into(), json!(lat));
                data.insert("longitude".into(), json!(lon));
                let names: Vec<String> = selected.iter().filter_map(|c| areas::get(c)).map(|a| a.title()).collect();
                let text = format!("Stedet ligger i {}. Tilføj eller fjern områder, hvis du vil.", names.join(", "));
                Ok(form_with("areas", areas_schema(&selected), &text, None).with_data(data))
            }
            ("areas", Some(v)) => {
                let codes = chosen(&v);
                if codes.is_empty() {
                    return Ok(form_with("areas", areas_schema(&[]), "Vælg mindst ét område.", Some(json!({ "areas": "no_areas" }))));
                }
                let mut data = flow.data.clone();
                data.insert("areas".into(), json!(codes));
                Ok(FlowReply::create(&title(&codes), Value::Object(data)).with_unique_id(codes.join(",")))
            }
            (s, _) => Err(Error::invalid(format!("ukendt trin {s}"))),
        }
    }

    async fn options_flow(&self, _host: &Host, flow: Flow, _step: &str, input: Option<Value>) -> Result<FlowReply, Error> {
        let current = flow.entry.as_ref().and_then(|e| e.get("areas")).map(str_list).unwrap_or_default();
        match input {
            None => Ok(FlowReply::form("options", areas_schema(&current))),
            Some(v) => {
                let codes = chosen(&v);
                if codes.is_empty() {
                    return Ok(FlowReply::form_with_errors("options", areas_schema(&current), json!({ "areas": "no_areas" })));
                }
                Ok(FlowReply::create("", json!({ "areas": codes })))
            }
        }
    }

    async fn setup_entry(&self, host: &Host, entry: Entry) -> Result<(), Error> {
        let entry_id = entry.entry_id.clone();
        let codes = entry.get("areas").map(str_list).unwrap_or_default();
        if codes.is_empty() {
            return Err(Error::failed("ingen varslingsområder"));
        }
        let url = entry.get_str("feed_url").unwrap_or(FEED_URL).to_string();
        let poll = entry.get_f64("interval").map(Duration::from_secs_f64).unwrap_or(POLL);
        let client = reqwest::Client::builder()
            .user_agent(concat!("homewright-dmi-varsler/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(Error::failed)?;
        // the first fetch here, so a setup without a connection is retried
        let feed = fetch(&client, &url).await.map_err(Error::not_ready)?;

        let device = json!({
            "identifiers": [[DOMAIN, entry_id]],
            "name": entry.title.clone(),
            "manufacturer": "DMI",
            "model": "Vejrvarsler",
            "entry_type": "service",
            "configuration_url": "https://www.dmi.dk/varsler/",
        });
        let dev = host.request("device/register", json!({ "entry_id": entry_id, "device": device })).await?;
        let dev_id = dev["id"].as_str().ok_or_else(|| Error::failed("intet enheds-id"))?.to_string();
        let count = host
            .add_entity(&entry_id, Entity::new("sensor", format!("{entry_id}-warnings")).name("Varsler").icon("mdi:alert").device(&dev_id), false)
            .await?;
        let active = host
            .add_entity(&entry_id, Entity::new("binary_sensor", format!("{entry_id}-active")).name("Varsel nu").device_class("safety").device(&dev_id), false)
            .await?;
        let mut poller = Poller {
            host: host.clone(),
            entry_id: entry_id.clone(),
            codes,
            count,
            active,
            store: host.data_dir().join(format!("seen-{entry_id}.json")),
        };
        poller.update(&feed, Utc::now()).await;

        let task = tokio::spawn(async move {
            let mut failures = 0;
            loop {
                tokio::time::sleep(poll).await;
                match fetch(&client, &url).await {
                    Ok(feed) => {
                        failures = 0;
                        poller.update(&feed, Utc::now()).await;
                    }
                    Err(e) => {
                        failures += 1;
                        warn!("{url}: {e}");
                        if failures == FAILURES {
                            poller.unavailable().await;
                        }
                    }
                }
            }
        });
        if let Some(old) = self.tasks.lock().unwrap().insert(entry_id, task) {
            old.abort();
        }
        Ok(())
    }

    async fn unload_entry(&self, _host: &Host, entry_id: &str) -> Result<(), Error> {
        if let Some(t) = self.tasks.lock().unwrap().remove(entry_id) {
            t.abort();
        }
        Ok(())
    }

    async fn diagnostics(&self, _host: &Host, entry: Entry) -> Value {
        json!({ "data": entry.data, "options": entry.options })
    }
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let r = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("HTTP {}", r.status()));
    }
    r.json().await.map_err(|e| e.to_string())
}

struct Poller {
    host: Host,
    entry_id: String,
    codes: Vec<String>,
    count: Option<String>,
    active: Option<String>,
    /// The messages already announced and their level, kept across restarts.
    store: PathBuf,
}

impl Poller {
    async fn update(&mut self, feed: &Value, now: DateTime<Utc>) {
        let alerts = cap::parse(feed);
        let warnings = cap::current(&alerts, &self.codes, now);
        let list: Vec<Value> = warnings.iter().map(|w| w.to_json(now)).collect();
        let highest = warnings.iter().map(|w| w.level).max().unwrap_or(1);
        let now_highest = warnings.iter().filter(|w| w.in_effect(now)).map(|w| w.level).max().unwrap_or(1);
        let in_effect = warnings.iter().filter(|w| w.in_effect(now)).count();
        let area_names: Vec<String> = self.codes.iter().filter_map(|c| areas::get(c)).map(|a| a.title()).collect();

        if let Some(id) = &self.count {
            let mut a = Map::new();
            a.insert("highest_level".into(), json!(cap::level_name(highest)));
            a.insert("highest_level_da".into(), json!(cap::level_name_da(highest)));
            a.insert("in_effect".into(), json!(in_effect));
            a.insert("warnings".into(), Value::Array(list.clone()));
            a.insert("areas".into(), json!(area_names));
            a.insert("attribution".into(), json!("Varsler fra DMI via MeteoAlarm"));
            if let Err(e) = self.host.set_state(id, warnings.len().to_string(), a).await {
                warn!("kunne ikke opdatere {id}: {e}");
            }
        }
        if let Some(id) = &self.active {
            let mut a = Map::new();
            a.insert("highest_level".into(), json!(cap::level_name(now_highest)));
            a.insert("highest_level_da".into(), json!(cap::level_name_da(now_highest)));
            a.insert("warnings".into(), Value::Array(warnings.iter().filter(|w| w.in_effect(now)).map(|w| w.to_json(now)).collect()));
            let state = if in_effect > 0 { "on" } else { "off" };
            if let Err(e) = self.host.set_state(id, state, a).await {
                warn!("kunne ikke opdatere {id}: {e}");
            }
        }

        // announce warnings not seen before, and warnings raised to a higher level
        let seen = load(&self.store);
        let mut keep: HashMap<String, u8> = HashMap::new();
        for (w, j) in warnings.iter().zip(&list) {
            let lineage = cap::lineage(&alerts, w);
            let before = lineage.iter().filter_map(|id| seen.get(id)).max().copied();
            let kind = match before {
                None => Some("new"),
                Some(l) if l < w.level => Some("raised"),
                _ => None,
            };
            if let Some(kind) = kind {
                info!("{kind}: {} ({}) for {}", w.kind_name, cap::level_name(w.level), w.areas.join(", "));
                let data = json!({ "entry_id": self.entry_id, "kind": kind, "warning": j });
                if let Err(e) = self.host.fire_event(EVENT, data).await {
                    warn!("kunne ikke sende {EVENT}: {e}");
                }
            }
            let level = before.unwrap_or(0).max(w.level);
            keep.extend(lineage.into_iter().map(|id| (id, level)));
        }
        if keep != seen {
            save(&self.store, &keep);
        }
    }

    async fn unavailable(&self) {
        for id in self.count.iter().chain(&self.active) {
            let _ = self.host.set_unavailable(id).await;
        }
    }
}

fn load(path: &PathBuf) -> HashMap<String, u8> {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(path: &PathBuf, seen: &HashMap<String, u8>) {
    let write = || -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_vec(seen).unwrap_or_default())
    };
    if let Err(e) = write() {
        warn!("kunne ikke gemme {}: {e}", path.display());
    }
}
