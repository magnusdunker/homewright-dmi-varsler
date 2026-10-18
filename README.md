# DMI-varsler til Homewright

En ekstern integration til [Homewright](https://github.com/magnusdunker/homewright), der henter DMI's vejrvarsler for et sted i Danmark: kraftig vind, regn, torden, sne og is, tåge, høje temperaturer og forhøjet vandstand. Den kræver ingen nøgle eller konto.

## Installation

I Homewright: **Indstillinger → Integrationer → Fra GitHub**. Indsæt

```
https://github.com/magnusdunker/homewright-dmi-varsler
```

og tryk **Installér**. Tilføj derefter **DMI-varsler** under **Tilføj integration**.

Formularen er udfyldt med hjemmets position fra `zone.home`. Næste trin viser de varslingsområder, stedet hører til: det område på land, det ligger i, og de kystområder, der ligger inden for 8 km. Du kan vælge flere eller færre. Kystområderne har varsler om forhøjet vandstand, så bor du ved vandet, så tjek, at det rigtige kystområde er med. Områderne kan ændres senere under integrationens indstillinger. Vil du følge flere steder, så tilføj integrationen en gang til.

## Hvad du får

En enhed med to entities:

- **Varsler** (`sensor`): antallet af varsler, der gælder nu eller er varslet til senere. Attributterne er:
  - `highest_level` og `highest_level_da`: det højeste niveau, `green`, `yellow`, `orange` eller `red` (grøn, gul, orange, rød)
  - `in_effect`: hvor mange af varslerne, der gælder nu
  - `warnings`: varslerne, hver med `type` (f.eks. "Forhøjet vandstand"), `type_id`, `level`, `level_da`, `headline`, `description`, `instruction`, `onset`, `ends`, `in_effect`, `areas`, `sent` og `id`
  - `areas`: de valgte områder
- **Varsel nu** (`binary_sensor`, device class `safety`): tændt, når et varsel gælder nu. `highest_level` og `warnings` dækker kun dem.

Varslerne hentes hvert 10. minut. Fejler det tre gange i træk, bliver begge entities utilgængelige, indtil det lykkes igen.

### Hændelsen `dmi_varsler_warning`

Når et nyt varsel dukker op, eller et varsel bliver hævet til et højere niveau, sender integrationen hændelsen `dmi_varsler_warning`. `data` har `entry_id`, `kind` (`new` eller `raised`) og `warning` med de samme felter som i attributten `warnings`. Integrationen husker, hvilke varsler den har sendt, så en genstart sender dem ikke igen.

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

## Hvor varslerne kommer fra

Integrationen læser MeteoAlarms feed for Danmark, `https://feeds.meteoalarm.org/api/v1/warnings/feeds-denmark`. DMI lægger selv sine varsler dér som CAP-beskeder (Common Alerting Protocol) på dansk og engelsk, og MeteoAlarm er det europæiske netværk af vejrtjenester, som DMI er med i. Valget faldt på det feed, fordi:

- Formatet er CAP, en fast standard, og feedet er beskrevet offentligt. dmi.dk har også et JSON-endpoint til sit varselskort, men det er ikke dokumenteret, og det kan ændre sig, når hjemmesiden gør.
- Feedet gemmer beskeder et par dage tilbage, så der findes rigtige varsler at teste mod. Testene bruger feedet fra 2026-10-18 med DMI's varsel om forhøjet vandstand ved Sydfyn og Lolland-Falster.
- Det kræver ingen nøgle. Indholdet er under CC BY 4.0.

DMI's varslingsområder er MeteoAlarms EMMA_ID-områder, 8 på land og 20 langs kysten. Deres omrids er taget fra Python-pakken [meteoalarm](https://pypi.org/project/meteoalarm/) (MIT) og ligger i `assets/areas.json`.

Sådan læses feedet:

- En besked, som en nyere besked henviser til (en opdatering eller en annullering), er erstattet.
- For hver slags varsel og hvert område gælder den nyeste besked. Hver morgen sender DMI en grøn besked, "Der er ingen aktive DMI varsler/risiko", for alle områder og slags, og den afslutter det, der var før.
- DMI sætter `expires` til det tidspunkt, beskeden blev sendt, så `expires` afslutter kun et varsel, når den ligger efter afsendelsen. `ends` gør det altid.
- Beskeder med samme tekst for flere områder bliver ét varsel med alle områderne.

## Udvikling

Bygget i Rust på SDK'en `thehome-plugin` fra [homewright-example-integration-rust](https://github.com/magnusdunker/homewright-example-integration-rust).

- `src/cap.rs`: CAP-beskederne i feedet, og hvilke varsler der gælder
- `src/areas.rs`: varslingsområderne og hvilke af dem et sted hører til
- `src/lib.rs`: opsætningen, opdateringen og forbindelsen til Homewright
- `tests/`: tests mod det rigtige feed i `tests/fixtures`, mod et falsk feed og med en falsk Homewright i `tests/host`

```
cargo test
cargo test -- --ignored live_feed --nocapture   # mod det rigtige feed
cargo build --release
```

En ny version udgives med et tag, f.eks. `git tag v0.2.0 && git push origin v0.2.0`. Workflowet bygger til Windows, Linux (x86_64 og arm64, statisk musl) og macOS og lægger binærerne på releasen.

## Licens

MIT. Varslerne er fra DMI via MeteoAlarm under CC BY 4.0.
