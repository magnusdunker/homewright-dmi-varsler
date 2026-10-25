# DMI-varsler for HomewrightHQ

An external integration for [HomewrightHQ](https://github.com/magnusdunker/homewrighthq) that fetches DMI's weather warnings for a place in Denmark: strong wind, rain, thunder, snow and ice, fog, high temperatures and raised water levels. It needs no key or account.

## Installation

In HomewrightHQ: **Settings → Integrations → From GitHub**. Paste

```
https://github.com/magnusdunker/homewrighthq-dmi-varsler
```

and press **Install**. Then add **DMI-varsler** under **Add integration**.

The form is filled in with the home's position from `zone.home`. The next step shows the warning areas the place belongs to: the land area it is in, and the coastal areas within 8 km. You can select more or fewer. The coastal areas have warnings about raised water levels, so if you live by the water, check that the right coastal area is included. The areas can be changed later under the integration's options. To follow more places, add the integration again.

## What you get

A device with two entities:

- **Varsler** (warnings, `sensor`): the number of warnings that apply now or are forecast for later. The attributes are:
  - `highest_level` and `highest_level_da`: the highest level, `green`, `yellow`, `orange` or `red` (grøn, gul, orange, rød)
  - `in_effect`: how many of the warnings apply now
  - `warnings`: the warnings, each with `type` (e.g. "Forhøjet vandstand"), `type_id`, `level`, `level_da`, `headline`, `description`, `instruction`, `onset`, `ends`, `in_effect`, `areas`, `sent` and `id`
  - `areas`: the selected areas
- **Varsel nu** (warning now, `binary_sensor`, device class `safety`): on when a warning applies now. `highest_level` and `warnings` cover only those.

The warnings are fetched every 10 minutes. If that fails three times in a row, both entities become unavailable until it succeeds again.

### The `dmi_varsler_warning` event

When a new warning appears, or a warning is raised to a higher level, the integration fires the event `dmi_varsler_warning`. `data` has `entry_id`, `kind` (`new` or `raised`) and `warning` with the same fields as in the `warnings` attribute. The integration remembers which warnings it has sent, so a restart does not send them again.

```yaml
triggers:
  - trigger: event
    event_type: dmi_varsler_warning
actions:
  - action: notify.notify
    data:
      title: "{{ trigger.event.data.warning.headline }}"
      message: >
        {{ trigger.event.data.warning.level_da | capitalize }} varsel:
        {{ trigger.event.data.warning.description }}
```

## Where the warnings come from

The integration reads MeteoAlarm's feed for Denmark, `https://feeds.meteoalarm.org/api/v1/warnings/feeds-denmark`. DMI itself puts its warnings there as CAP messages (Common Alerting Protocol) in Danish and English, and MeteoAlarm is the European network of weather services that DMI is part of. That feed was chosen because:

- The format is CAP, a fixed standard, and the feed is publicly described. dmi.dk also has a JSON endpoint for its warning map, but it is not documented, and it can change when the website does.
- The feed keeps messages a few days back, so there are real warnings to test against. The tests use the feed from 2026-10-18 with DMI's warning of raised water levels at Sydfyn and Lolland-Falster.
- It needs no key. The content is under CC BY 4.0.

DMI's warning areas are MeteoAlarm's EMMA_ID areas, 8 on land and 20 along the coast. Their outlines are taken from the Python package [meteoalarm](https://pypi.org/project/meteoalarm/) (MIT) and are in `assets/areas.json`.

This is how the feed is read:

- A message that a newer message refers to (an update or a cancellation) is replaced.
- For each kind of warning and each area, the newest message applies. Every morning DMI sends a green message, "Der er ingen aktive DMI varsler/risiko" (there are no active DMI warnings/risk), for all areas and kinds, and it ends whatever was there before.
- DMI sets `expires` to the time the message was sent, so `expires` only ends a warning when it is later than the sending time. `ends` always does.
- Messages with the same text for several areas become one warning with all the areas.

## Development

Built in Rust on the `thehome-plugin` SDK from [homewright-example-integration-rust](https://github.com/magnusdunker/homewrighthq-example-integration-rust).

- `src/cap.rs`: the CAP messages in the feed, and which warnings apply
- `src/areas.rs`: the warning areas and which of them a place belongs to
- `src/lib.rs`: the setup, the update and the connection to HomewrightHQ
- `tests/`: tests against the real feed in `tests/fixtures`, against a fake feed and with a fake HomewrightHQ in `tests/host`

```
cargo test
cargo test -- --ignored live_feed --nocapture   # against the real feed
cargo build --release
```

A new version is released with a tag, e.g. `git tag v0.2.0 && git push origin v0.2.0`. The workflow builds for Windows, Linux (x86_64 and arm64, static musl) and macOS and puts the binaries on the release.

## License

MIT. The warnings are from DMI via MeteoAlarm under CC BY 4.0.
