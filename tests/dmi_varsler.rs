//! The feed parsing against a real MeteoAlarm feed from 2026-10-18, the
//! areas, and the plugin played over a pipe against a fake feed.

mod host;

use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use homewright_dmi_varsler::{areas, cap, DmiVarsler, EVENT};
use host::FakeHost;
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn real_feed() -> Value {
    let p = format!("{}/tests/fixtures/meteoalarm-2026-10-18.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn codes(c: &[&str]) -> Vec<String> {
    c.iter().map(|s| s.to_string()).collect()
}

#[test]
fn real_feed_storm_surge_in_the_night() {
    let alerts = cap::parse(&real_feed());
    assert_eq!(alerts.len(), 54);
    // 06:00 Danish time: the update from 04:39 is the one in force
    let w = cap::current(&alerts, &codes(&["DK006", "DK811"]), at("2026-10-18T04:00:00Z"));
    assert_eq!(w.len(), 1, "{w:#?}");
    let w = &w[0];
    assert_eq!(w.kind, 7);
    assert_eq!(w.kind_name, "Forhøjet vandstand");
    assert_eq!(w.level, 2);
    assert_eq!(w.headline, "DMI varsel om forhøjet vandstand");
    assert!(w.description.contains("mellem 1.0 og 1.3 m"), "{}", w.description);
    assert!(w.instruction.starts_with("Vær opmærksom på"), "{}", w.instruction);
    assert_eq!(w.areas, ["Sydfynske Øhav"]);
    assert_eq!(w.ids, ["2.49.0.1.208.0.20261018023945.7_C11_1202610170203"]);
    assert!(w.in_effect(at("2026-10-18T04:00:00Z")));
    // the messages it replaced
    let lineage = cap::lineage(&alerts, w);
    assert!(lineage.contains("2.49.0.1.208.0.20261017185401.7_C11_1202610170203"));

    // the same text for two coasts is one warning
    let w = cap::current(&alerts, &codes(&["DK810", "DK811"]), at("2026-10-18T04:00:00Z"));
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].areas, ["Sydfynske Øhav", "Sydlige Lillebælt"]);
    assert_eq!(w[0].ids.len(), 2);

    // inland, nothing
    assert!(cap::current(&alerts, &codes(&["DK006"]), at("2026-10-18T04:00:00Z")).is_empty());
    // DMI's green message at 08:00 ends it
    assert!(cap::current(&alerts, &codes(&["DK811"]), at("2026-10-18T07:00:00Z")).is_empty());
}

#[test]
fn areas_for_a_point() {
    // Odense centre: Fyn, far from the coast areas
    assert_eq!(areas::for_point(55.3959, 10.3883), ["DK006"]);
    // Svendborg harbour: Fyn and the South Funen archipelago
    let s = areas::for_point(55.0598, 10.6138);
    assert_eq!(s[0], "DK006");
    assert!(s.contains(&"DK811"), "{s:?}");
    // Aarhus, Aalborg, Rønne, Copenhagen
    assert_eq!(areas::for_point(56.1572, 10.2107)[0], "DK004");
    assert_eq!(areas::for_point(57.0488, 9.9217)[0], "DK001");
    assert_eq!(areas::for_point(55.1037, 14.7065)[0], "DK003");
    assert_eq!(areas::for_point(55.6761, 12.5683)[0], "DK008");
    // Berlin
    assert!(areas::for_point(52.52, 13.405).is_empty());
    assert_eq!(areas::all().len(), 28);
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[derive(Clone, Copy)]
struct A<'a> {
    id: &'a str,
    sent: DateTime<Utc>,
    msg_type: &'a str,
    refs: &'a [&'a str],
    kind: u32,
    level: u8,
    areas: &'a [(&'a str, &'a str)],
    onset: DateTime<Utc>,
    expires: DateTime<Utc>,
    text: &'a str,
}

fn alert(a: A) -> Value {
    let color = ["", "green", "yellow", "orange", "red"][a.level as usize];
    let refs: Vec<String> = a.refs.iter().map(|r| format!("vagtchef@dmi.dk,{r},{}", ts(a.sent))).collect();
    let area: Vec<Value> = a
        .areas
        .iter()
        .map(|(c, d)| json!({ "areaDesc": d, "geocode": [{ "value": c, "valueName": "EMMA_ID" }] }))
        .collect();
    let mut alert = json!({
        "identifier": a.id, "msgType": a.msg_type, "status": "Actual", "sent": ts(a.sent),
        "info": [
            { "language": "en-GB", "event": "Moderate Wind", "headline": "DMI warning", "area": area,
              "parameter": [{ "valueName": "awareness_level", "value": format!("{}; {color}; x", a.level) },
                            { "valueName": "awareness_type", "value": format!("{}; Wind", a.kind) }] },
            { "language": "da-DK", "event": "Moderate Kraftig vind", "headline": "DMI varsel om kraftig vind", "description": a.text,
              "onset": ts(a.onset), "expires": ts(a.expires), "area": area,
              "parameter": [{ "valueName": "awareness_level", "value": format!("{}; {color}; x", a.level) },
                            { "valueName": "awareness_type", "value": format!("{}; Wind", a.kind) },
                            { "valueName": "impacts", "value": "Pas på løse genstande." }] }
        ]
    });
    if !refs.is_empty() {
        alert["references"] = json!(refs.join(" "));
    }
    json!({ "alert": alert })
}

fn feed(alerts: Vec<Value>) -> Value {
    json!({ "warnings": alerts })
}

#[test]
fn cancel_expiry_and_coming_warnings() {
    let now = at("2026-11-02T12:00:00Z");
    let h = ChronoDuration::hours;
    let fyn = &[("DK006", "Fyn")][..];
    let base = |id, kind, level, onset, expires| A { id, sent: now - h(3), msg_type: "Alert", refs: &[], kind, level, areas: fyn, onset, expires, text: id };
    let alerts = cap::parse(&feed(vec![
        // in force, expires = sent as DMI does it
        alert(base("wind", 1, 3, now - h(1), now - h(3))),
        // coming tomorrow
        alert(base("rain", 10, 2, now + h(20), now - h(3))),
        // expired properly
        alert(base("fog", 4, 2, now - h(2), now - h(1))),
        // cancelled
        alert(base("snow", 2, 2, now - h(2), now - h(3))),
        alert(A { id: "snow-cancel", sent: now - h(1), msg_type: "Cancel", refs: &["snow"], ..base("x", 2, 2, now - h(2), now - h(1)) }),
        // another area
        alert(A { areas: &[("DK001", "Nordjylland")], ..base("north", 3, 4, now - h(1), now - h(3)) }),
    ]));
    let w = cap::current(&alerts, &codes(&["DK006"]), now);
    let got: Vec<(&str, u8, bool)> = w.iter().map(|w| (w.ids[0].as_str(), w.level, w.in_effect(now))).collect();
    assert_eq!(got, [("wind", 3, true), ("rain", 2, false)]);
    let j = w[0].to_json(now);
    assert_eq!(j["level"], "orange");
    assert_eq!(j["level_da"], "orange");
    assert_eq!(j["type"], "Kraftig vind");
    assert_eq!(j["instruction"], "Pas på løse genstande.");
}

async fn start() -> FakeHost {
    let host = FakeHost::start(DmiVarsler::default());
    let dir = std::env::temp_dir().join(format!("dmi-varsler-test-{}-{}", std::process::id(), rand_suffix()));
    host.call("initialize", json!({ "api_version": 1, "data_dir": dir })).await.unwrap();
    host
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

fn flow(data: Value) -> Value {
    json!({ "flow_id": "f1", "source": "user", "data": data })
}

#[tokio::test]
async fn config_flow_starts_at_home() {
    let host = start().await;
    let r = host.call("config_flow", json!({ "flow": flow(json!({})), "step": "user" })).await.unwrap();
    assert_eq!(r["result"]["step_id"], "user");
    assert_eq!(r["result"]["schema"]["properties"]["latitude"]["default"], 55.3959);

    let berlin = json!({ "flow": flow(json!({})), "step": "user", "input": { "latitude": 52.52, "longitude": 13.405 } });
    let r = host.call("config_flow", berlin).await.unwrap();
    assert_eq!(r["result"]["errors"]["base"], "outside_denmark");

    let svendborg = json!({ "flow": flow(json!({})), "step": "user", "input": { "latitude": 55.0598, "longitude": 10.6138 } });
    let r = host.call("config_flow", svendborg).await.unwrap();
    assert_eq!(r["result"]["step_id"], "areas");
    let default = r["result"]["schema"]["properties"]["areas"]["default"].clone();
    assert!(default.as_array().unwrap().contains(&json!("DK811")), "{default}");
    assert!(r["result"]["description"].as_str().unwrap().contains("Kyst: Sydfynske"));
    let data = r["data"].clone();

    let step = |areas: Value| json!({ "flow": flow(data.clone()), "step": "areas", "input": { "areas": areas } });
    let r = host.call("config_flow", step(json!([]))).await.unwrap();
    assert_eq!(r["result"]["errors"]["areas"], "no_areas");
    let r = host.call("config_flow", step(json!(["DK811", "DK006", "DK999"]))).await.unwrap();
    assert_eq!(r["result"]["type"], "create_entry");
    assert_eq!(r["result"]["title"], "DMI-varsler Fyn");
    assert_eq!(r["result"]["data"], json!({ "latitude": 55.0598, "longitude": 10.6138, "areas": ["DK006", "DK811"] }));
    assert_eq!(r["unique_id"], "DK006,DK811");
}

async fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..200 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("ventede forgæves på {what}");
}

async fn serve(server: &MockServer, body: Value) {
    server.reset().await;
    Mock::given(method("GET")).and(path("/feed")).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(server).await;
}

#[tokio::test]
async fn entities_states_and_events() {
    let server = MockServer::start().await;
    let now = Utc::now();
    let h = ChronoDuration::hours;
    let fyn = &[("DK006", "Fyn")][..];
    let wind = A { id: "w1", sent: now - h(2), msg_type: "Alert", refs: &[], kind: 1, level: 2, areas: fyn, onset: now - h(1), expires: now - h(2), text: "Vind" };
    serve(&server, feed(vec![alert(wind)])).await;

    let host = start().await;
    let entry = json!({ "entry_id": "e1", "domain": "dmi_varsler", "title": "DMI-varsler Fyn",
                        "data": { "areas": ["DK006"], "feed_url": format!("{}/feed", server.uri()), "interval": 0.3 } });
    host.call("setup_entry", json!({ "entry": entry })).await.unwrap();

    let (state, attrs) = host.state("sensor.odense_varsler").unwrap();
    assert_eq!(state, "1");
    assert_eq!(attrs["highest_level"], "yellow");
    assert_eq!(attrs["warnings"][0]["type"], "Kraftig vind");
    assert_eq!(attrs["areas"], json!(["Fyn"]));
    let (state, attrs) = host.state("binary_sensor.odense_varsel_nu").unwrap();
    assert_eq!(state, "on");
    assert_eq!(attrs["highest_level_da"], "gul");
    assert_eq!(host.rec.lock().unwrap().entities["binary_sensor.odense_varsel_nu"].0["device_class"], "safety");
    let events = host.rec.lock().unwrap().events.clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, EVENT);
    assert_eq!(events[0].1["kind"], "new");

    // the same warning raised to orange, and a red one coming
    let raised = A { id: "w2", sent: now - h(1), msg_type: "Update", refs: &["w1"], level: 3, ..wind };
    let red = A { id: "r1", kind: 10, level: 4, onset: now + h(5), text: "Regn", ..wind };
    serve(&server, feed(vec![alert(wind), alert(raised), alert(red)])).await;
    let events = wait_for("to hændelser mere", || {
        let ev = host.rec.lock().unwrap().events.clone();
        (ev.len() >= 3).then_some(ev)
    })
    .await;
    let kinds: Vec<(&str, &str)> = events[1..].iter().map(|(_, d)| (d["kind"].as_str().unwrap(), d["warning"]["id"].as_str().unwrap())).collect();
    assert!(kinds.contains(&("raised", "w2")) && kinds.contains(&("new", "r1")), "{kinds:?}");
    let (state, attrs) = host.state("sensor.odense_varsler").unwrap();
    assert_eq!(state, "2");
    assert_eq!(attrs["highest_level"], "red");
    assert_eq!(attrs["in_effect"], 1);
    let (_, attrs) = host.state("binary_sensor.odense_varsel_nu").unwrap();
    assert_eq!(attrs["highest_level"], "orange");

    // nothing new: no more events
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(host.rec.lock().unwrap().events.len(), 3);

    // DMI's all clear
    let clear = A { id: "g1", sent: now, kind: 1, level: 1, text: "Der er ingen aktive DMI varsler/risiko.", ..wind };
    let clear_rain = A { id: "g2", kind: 10, ..clear };
    serve(&server, feed(vec![alert(wind), alert(raised), alert(red), alert(clear), alert(clear_rain)])).await;
    wait_for("ingen varsler", || (host.state("sensor.odense_varsler").unwrap().0 == "0").then_some(())).await;
    assert_eq!(host.state("binary_sensor.odense_varsel_nu").unwrap().0, "off");

    // the feed is gone: unavailable after three failures
    server.reset().await;
    wait_for("utilgængelig", || (host.state("sensor.odense_varsler").unwrap().0 == "unavailable").then_some(())).await;
}

#[tokio::test]
async fn setup_without_the_feed_is_retried() {
    let host = start().await;
    let entry = json!({ "entry_id": "e2", "data": { "areas": ["DK006"], "feed_url": "http://127.0.0.1:1/feed" } });
    let e = host.call("setup_entry", json!({ "entry": entry })).await.unwrap_err();
    assert_eq!(e["data"]["kind"], "not_ready");
}

/// Against the real feed: `cargo test -- --ignored live_feed`.
#[tokio::test]
#[ignore]
async fn live_feed() {
    let feed: Value = reqwest::get(cap::FEED_URL).await.unwrap().json().await.unwrap();
    let raw = feed["warnings"].as_array().unwrap().len();
    let alerts = cap::parse(&feed);
    assert!(!alerts.is_empty());
    // every DMI message has a type and a level
    assert_eq!(alerts.len(), raw);
    let all: Vec<String> = areas::all().iter().map(|a| a.code.clone()).collect();
    for w in cap::current(&alerts, &all, Utc::now()) {
        println!("{} {} {:?} {}", w.kind_name, cap::level_name(w.level), w.areas, w.headline);
    }
    println!("{} messages", alerts.len());
}
