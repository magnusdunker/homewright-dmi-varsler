//! DMI's warning areas: 8 on land (`DK001`-`DK008`) and 20 along the coast
//! (`DK801`-`DK820`), with their outlines. The outlines are MeteoAlarm's
//! EMMA_ID areas, taken from the `meteoalarm` Python package and rounded to
//! 4 decimals (about 10 m).

use std::sync::OnceLock;

use serde::Deserialize;

/// How close a coast area must be for its water level warnings to be chosen
/// by default. The coast areas are drawn on the water and start a few
/// kilometres out: Svendborg harbour is 3.9 km from "Sydfynske Øhav" and the
/// centre of Copenhagen 3.8 km from "Øresund".
pub const COAST_KM: f64 = 8.0;
/// A point outside every land area may be this far from the nearest one.
const LAND_KM: f64 = 25.0;

#[derive(Deserialize, Debug)]
pub struct Area {
    pub code: String,
    pub name: String,
    /// GeoJSON MultiPolygon coordinates: polygons of rings of [lon, lat].
    polygons: Vec<Vec<Vec<[f64; 2]>>>,
}

impl Area {
    pub fn is_coast(&self) -> bool {
        self.code.starts_with("DK8")
    }

    /// The name shown in forms; the coast areas have a land area's name too.
    pub fn title(&self) -> String {
        if self.is_coast() {
            format!("Kyst: {}", self.name)
        } else {
            self.name.clone()
        }
    }

    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        self.polygons.iter().any(|rings| {
            // inside the outer ring and not in a hole
            let mut rings = rings.iter();
            rings.next().is_some_and(|outer| in_ring(outer, lat, lon)) && !rings.any(|hole| in_ring(hole, lat, lon))
        })
    }

    /// Kilometres from the point to the area, 0 inside it.
    pub fn distance_km(&self, lat: f64, lon: f64) -> f64 {
        if self.contains(lat, lon) {
            return 0.0;
        }
        let kx = 111.32 * lat.to_radians().cos();
        let ky = 110.57;
        let mut best = f64::MAX;
        for ring in self.polygons.iter().flatten() {
            for w in ring.windows(2) {
                let (ax, ay) = ((w[0][0] - lon) * kx, (w[0][1] - lat) * ky);
                let (bx, by) = ((w[1][0] - lon) * kx, (w[1][1] - lat) * ky);
                best = best.min(segment_distance(ax, ay, bx, by));
            }
        }
        best
    }
}

/// Distance from the origin to the segment a-b.
fn segment_distance(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (-(ax * dx + ay * dy) / len2).clamp(0.0, 1.0) };
    let (px, py) = (ax + t * dx, ay + t * dy);
    (px * px + py * py).sqrt()
}

fn in_ring(ring: &[[f64; 2]], lat: f64, lon: f64) -> bool {
    let mut inside = false;
    let mut j = ring.len().wrapping_sub(1);
    for i in 0..ring.len() {
        let ([xi, yi], [xj, yj]) = (ring[i], ring[j]);
        if (yi > lat) != (yj > lat) && lon < (xj - xi) * (lat - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn all() -> &'static [Area] {
    static AREAS: OnceLock<Vec<Area>> = OnceLock::new();
    AREAS.get_or_init(|| serde_json::from_str(include_str!("../assets/areas.json")).expect("assets/areas.json"))
}

pub fn get(code: &str) -> Option<&'static Area> {
    all().iter().find(|a| a.code == code)
}

/// The areas whose warnings concern a point: the land area it is in (or the
/// nearest, for a point just off the coast) and the coast areas within
/// [`COAST_KM`]. Empty when the point is not in Denmark.
pub fn for_point(lat: f64, lon: f64) -> Vec<&'static str> {
    let land = all()
        .iter()
        .filter(|a| !a.is_coast())
        .map(|a| (a, a.distance_km(lat, lon)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, d)| *d <= LAND_KM);
    let Some((land, _)) = land else { return vec![] };
    let mut out = vec![land.code.as_str()];
    out.extend(all().iter().filter(|a| a.is_coast() && a.distance_km(lat, lon) <= COAST_KM).map(|a| a.code.as_str()));
    out
}
