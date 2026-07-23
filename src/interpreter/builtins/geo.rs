//! `Geo.*` — the arithmetic behind "find what is near me".
//!
//! The browser hands you a latitude and longitude; everything after that is
//! server-side. These are the operations an app actually needs, and each is
//! easy to get subtly wrong by hand:
//!
//! ```soli
//! metres = Geo.distance(48.8566, 2.3522, 51.5074, -0.1278)   # Paris → London
//! box    = Geo.bounding_box(48.8566, 2.3522, 5000)           # 5 km around Paris
//! hash   = Geo.geohash(48.8566, 2.3522, 9)
//! ```
//!
//! # Why the bounding box matters
//!
//! Distance is a trigonometric function of every row, so a query that sorts by
//! it cannot use an index — on a large collection that is a full scan. The
//! usual shape is a cheap indexed pre-filter followed by exact distances on
//! what survives:
//!
//! ```soli
//! box = Geo.bounding_box(lat, lng, 5000)
//! nearby = Place.where("lat >= @min_lat AND lat <= @max_lat AND lng >= @min_lng AND lng <= @max_lng", box)
//! ```
//!
//! The box is deliberately a little generous — it is a square around a circle,
//! so it returns corners the radius excludes. Filter those out with
//! `Geo.distance` afterwards; that is the point of the two-step.

use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};

/// Mean Earth radius in metres (IUGG). Good to ~0.5% anywhere on the globe,
/// which is far better than consumer GPS.
const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// The standard geohash alphabet: base32 without `a`, `i`, `l`, `o`, chosen so
/// a hash is hard to mistranscribe.
const BASE32: &[u8] = b"0123456789bcdefghjkmnpqrstuvwxyz";

const MAX_PRECISION: usize = 12;

fn number(value: &Value, function: &str, name: &str) -> Result<f64, String> {
    match value {
        Value::Int(i) => Ok(*i as f64),
        Value::Float(f) => Ok(*f),
        Value::Decimal(d) => d
            .to_string()
            .parse::<f64>()
            .map_err(|_| format!("{}(): {} is not a number", function, name)),
        other => Err(format!(
            "{}(): {} must be a number, got {}",
            function,
            name,
            other.type_name()
        )),
    }
}

/// Coordinates outside these ranges are a bug in the caller, not a point on
/// Earth — and silently wrapping them produces answers that look plausible.
fn check_coordinates(lat: f64, lng: f64, function: &str) -> Result<(), String> {
    if !(-90.0..=90.0).contains(&lat) {
        return Err(format!(
            "{}(): latitude must be between -90 and 90, got {}",
            function, lat
        ));
    }
    if !(-180.0..=180.0).contains(&lng) {
        return Err(format!(
            "{}(): longitude must be between -180 and 180, got {}",
            function, lng
        ));
    }
    Ok(())
}

/// Great-circle distance in metres.
///
/// Haversine rather than the simpler equirectangular approximation: the error
/// of the cheap version grows with distance and with latitude, and this is not
/// the expensive part of any request that also touches a database.
pub fn haversine_metres(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    let (phi1, phi2) = (lat1.to_radians(), lat2.to_radians());
    let delta_phi = (lat2 - lat1).to_radians();
    let delta_lambda = (lng2 - lng1).to_radians();

    let a = (delta_phi / 2.0).sin().powi(2)
        + phi1.cos() * phi2.cos() * (delta_lambda / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * a.sqrt().asin()
}

/// Initial bearing from the first point to the second, in degrees clockwise
/// from north. "Initial" because a great-circle course changes as you travel.
fn bearing_degrees(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    let (phi1, phi2) = (lat1.to_radians(), lat2.to_radians());
    let delta_lambda = (lng2 - lng1).to_radians();

    let y = delta_lambda.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * delta_lambda.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// The latitude/longitude square enclosing a circle of `radius_m`.
///
/// Longitude degrees shrink towards the poles, so the longitude span is divided
/// by cos(latitude) — without that the box is far too narrow in Oslo and far
/// too wide near the equator.
fn bounding_box(lat: f64, lng: f64, radius_m: f64) -> HashPairs {
    let lat_delta = (radius_m / EARTH_RADIUS_M).to_degrees();
    // Near the poles cos() approaches zero and the span explodes; clamp to the
    // whole world rather than emit infinities.
    let cos_lat = lat.to_radians().cos().abs().max(1e-9);
    let lng_delta = ((radius_m / EARTH_RADIUS_M) / cos_lat)
        .to_degrees()
        .min(180.0);

    let mut box_pairs = HashPairs::default();
    box_pairs.insert(
        HashKey::String("min_lat".into()),
        Value::Float((lat - lat_delta).max(-90.0)),
    );
    box_pairs.insert(
        HashKey::String("max_lat".into()),
        Value::Float((lat + lat_delta).min(90.0)),
    );
    box_pairs.insert(
        HashKey::String("min_lng".into()),
        Value::Float((lng - lng_delta).max(-180.0)),
    );
    box_pairs.insert(
        HashKey::String("max_lng".into()),
        Value::Float((lng + lng_delta).min(180.0)),
    );
    box_pairs
}

/// Encode a position as a geohash of `precision` characters.
///
/// Useful as an indexable key: a shared prefix means proximity, so a `LIKE
/// 'u09tv%'` finds a neighbourhood without any trigonometry. Precision 9 is
/// roughly a 5-metre cell, 6 is roughly 1 km.
pub fn geohash(lat: f64, lng: f64, precision: usize) -> String {
    let mut lat_range = (-90.0f64, 90.0f64);
    let mut lng_range = (-180.0f64, 180.0f64);
    let mut hash = String::with_capacity(precision);
    let mut bit = 0;
    let mut index = 0usize;
    let mut even = true; // longitude first

    while hash.len() < precision {
        if even {
            let mid = (lng_range.0 + lng_range.1) / 2.0;
            if lng >= mid {
                index = index * 2 + 1;
                lng_range.0 = mid;
            } else {
                index *= 2;
                lng_range.1 = mid;
            }
        } else {
            let mid = (lat_range.0 + lat_range.1) / 2.0;
            if lat >= mid {
                index = index * 2 + 1;
                lat_range.0 = mid;
            } else {
                index *= 2;
                lat_range.1 = mid;
            }
        }
        even = !even;

        bit += 1;
        if bit == 5 {
            hash.push(BASE32[index] as char);
            bit = 0;
            index = 0;
        }
    }
    hash
}

/// The centre of a geohash cell, plus the cell's half-height and half-width in
/// degrees — the honest error bars on that centre.
fn geohash_decode(hash: &str) -> Result<HashPairs, String> {
    let mut lat_range = (-90.0f64, 90.0f64);
    let mut lng_range = (-180.0f64, 180.0f64);
    let mut even = true;

    for character in hash.chars() {
        let lowered = character.to_ascii_lowercase();
        let index = BASE32
            .iter()
            .position(|&b| b as char == lowered)
            .ok_or_else(|| {
                format!(
                    "Geo.geohash_decode(): '{}' is not a geohash character",
                    character
                )
            })?;

        for shift in (0..5).rev() {
            let bit = (index >> shift) & 1;
            if even {
                let mid = (lng_range.0 + lng_range.1) / 2.0;
                if bit == 1 {
                    lng_range.0 = mid;
                } else {
                    lng_range.1 = mid;
                }
            } else {
                let mid = (lat_range.0 + lat_range.1) / 2.0;
                if bit == 1 {
                    lat_range.0 = mid;
                } else {
                    lat_range.1 = mid;
                }
            }
            even = !even;
        }
    }

    let mut pairs = HashPairs::default();
    pairs.insert(
        HashKey::String("lat".into()),
        Value::Float((lat_range.0 + lat_range.1) / 2.0),
    );
    pairs.insert(
        HashKey::String("lng".into()),
        Value::Float((lng_range.0 + lng_range.1) / 2.0),
    );
    pairs.insert(
        HashKey::String("lat_error".into()),
        Value::Float((lat_range.1 - lat_range.0) / 2.0),
    );
    pairs.insert(
        HashKey::String("lng_error".into()),
        Value::Float((lng_range.1 - lng_range.0) / 2.0),
    );
    Ok(pairs)
}

fn hash_value(pairs: HashPairs) -> Value {
    Value::Hash(Rc::new(std::cell::RefCell::new(pairs)))
}

pub fn register_geo_builtins(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Geo.distance(lat1, lng1, lat2, lng2) -> Float (metres)
    statics.insert(
        "distance".to_string(),
        Rc::new(NativeFunction::new("Geo.distance", Some(4), |args| {
            let lat1 = number(&args[0], "Geo.distance", "lat1")?;
            let lng1 = number(&args[1], "Geo.distance", "lng1")?;
            let lat2 = number(&args[2], "Geo.distance", "lat2")?;
            let lng2 = number(&args[3], "Geo.distance", "lng2")?;
            check_coordinates(lat1, lng1, "Geo.distance")?;
            check_coordinates(lat2, lng2, "Geo.distance")?;
            Ok(Value::Float(haversine_metres(lat1, lng1, lat2, lng2)))
        })),
    );

    // Geo.bearing(lat1, lng1, lat2, lng2) -> Float (degrees from north)
    statics.insert(
        "bearing".to_string(),
        Rc::new(NativeFunction::new("Geo.bearing", Some(4), |args| {
            let lat1 = number(&args[0], "Geo.bearing", "lat1")?;
            let lng1 = number(&args[1], "Geo.bearing", "lng1")?;
            let lat2 = number(&args[2], "Geo.bearing", "lat2")?;
            let lng2 = number(&args[3], "Geo.bearing", "lng2")?;
            check_coordinates(lat1, lng1, "Geo.bearing")?;
            check_coordinates(lat2, lng2, "Geo.bearing")?;
            Ok(Value::Float(bearing_degrees(lat1, lng1, lat2, lng2)))
        })),
    );

    // Geo.bounding_box(lat, lng, radius_m) -> {min_lat, max_lat, min_lng, max_lng}
    statics.insert(
        "bounding_box".to_string(),
        Rc::new(NativeFunction::new("Geo.bounding_box", Some(3), |args| {
            let lat = number(&args[0], "Geo.bounding_box", "lat")?;
            let lng = number(&args[1], "Geo.bounding_box", "lng")?;
            let radius = number(&args[2], "Geo.bounding_box", "radius_m")?;
            check_coordinates(lat, lng, "Geo.bounding_box")?;
            if radius < 0.0 {
                return Err("Geo.bounding_box(): radius must not be negative".to_string());
            }
            Ok(hash_value(bounding_box(lat, lng, radius)))
        })),
    );

    // Geo.geohash(lat, lng, precision?) -> String
    statics.insert(
        "geohash".to_string(),
        Rc::new(NativeFunction::new("Geo.geohash", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(
                    "Geo.geohash() expects 2 or 3 arguments (lat, lng, precision?)".to_string(),
                );
            }
            let lat = number(&args[0], "Geo.geohash", "lat")?;
            let lng = number(&args[1], "Geo.geohash", "lng")?;
            check_coordinates(lat, lng, "Geo.geohash")?;
            let precision = match args.get(2) {
                None | Some(Value::Null) => 9,
                Some(value) => {
                    let n = number(value, "Geo.geohash", "precision")? as i64;
                    if n < 1 || n as usize > MAX_PRECISION {
                        return Err(format!(
                            "Geo.geohash(): precision must be between 1 and {}",
                            MAX_PRECISION
                        ));
                    }
                    n as usize
                }
            };
            Ok(Value::String(geohash(lat, lng, precision).into()))
        })),
    );

    // Geo.geohash_decode(hash) -> {lat, lng, lat_error, lng_error}
    statics.insert(
        "geohash_decode".to_string(),
        Rc::new(NativeFunction::new("Geo.geohash_decode", Some(1), |args| {
            let hash = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "Geo.geohash_decode() expects a string, got {}",
                        other.type_name()
                    ))
                }
            };
            if hash.is_empty() || hash.len() > MAX_PRECISION {
                return Err("Geo.geohash_decode(): implausible geohash".to_string());
            }
            geohash_decode(&hash).map(hash_value)
        })),
    );

    let class = Rc::new(Class {
        name: "Geo".to_string(),
        superclass: None,
        methods: Rc::new(std::cell::RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: statics,
        native_methods: HashMap::new(),
        static_fields: Rc::new(std::cell::RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(std::cell::RefCell::new(HashMap::new())),
        ..Default::default()
    });

    env.define("Geo".to_string(), Value::Class(class));
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARIS: (f64, f64) = (48.8566, 2.3522);
    const LONDON: (f64, f64) = (51.5074, -0.1278);

    #[test]
    fn distance_matches_a_known_pair() {
        let metres = haversine_metres(PARIS.0, PARIS.1, LONDON.0, LONDON.1);
        // Paris to London is ~343.5 km great-circle.
        assert!(
            (metres - 343_500.0).abs() < 2_000.0,
            "expected ~343.5 km, got {:.0} m",
            metres
        );
    }

    #[test]
    fn distance_to_itself_is_zero() {
        assert!(haversine_metres(PARIS.0, PARIS.1, PARIS.0, PARIS.1) < 0.001);
    }

    /// A degree of latitude is ~111 km everywhere, which is the one figure that
    /// makes an error in the formula obvious.
    #[test]
    fn a_degree_of_latitude_is_about_111km() {
        let metres = haversine_metres(0.0, 0.0, 1.0, 0.0);
        assert!(
            (metres - 111_195.0).abs() < 500.0,
            "expected ~111.2 km, got {:.0} m",
            metres
        );
    }

    #[test]
    fn bearing_is_clockwise_from_north() {
        assert!(
            (bearing_degrees(0.0, 0.0, 1.0, 0.0) - 0.0).abs() < 0.001,
            "north"
        );
        assert!(
            (bearing_degrees(0.0, 0.0, 0.0, 1.0) - 90.0).abs() < 0.001,
            "east"
        );
        assert!(
            (bearing_degrees(0.0, 0.0, -1.0, 0.0) - 180.0).abs() < 0.001,
            "south"
        );
        assert!(
            (bearing_degrees(0.0, 0.0, 0.0, -1.0) - 270.0).abs() < 0.001,
            "west"
        );
    }

    /// The box must contain the circle it encloses: every point at exactly the
    /// radius, in each cardinal direction, has to fall inside.
    #[test]
    fn the_bounding_box_contains_its_circle() {
        let pairs = bounding_box(PARIS.0, PARIS.1, 5_000.0);
        let get = |key: &str| match pairs.get(&HashKey::String(key.into())) {
            Some(Value::Float(f)) => *f,
            other => panic!("missing {}: {:?}", key, other),
        };

        // ~5 km north/south/east/west of Paris, in degrees.
        let lat_5km = 5_000.0 / 111_320.0;
        let lng_5km = 5_000.0 / (111_320.0 * PARIS.0.to_radians().cos());

        assert!(get("min_lat") <= PARIS.0 - lat_5km, "south edge");
        assert!(get("max_lat") >= PARIS.0 + lat_5km, "north edge");
        assert!(get("min_lng") <= PARIS.1 - lng_5km, "west edge");
        assert!(get("max_lng") >= PARIS.1 + lng_5km, "east edge");
    }

    /// Longitude degrees shrink towards the poles: forgetting the cos(latitude)
    /// term is the classic bug, and it makes northern boxes far too narrow.
    #[test]
    fn the_longitude_span_widens_with_latitude() {
        let span = |lat: f64| {
            let pairs = bounding_box(lat, 0.0, 10_000.0);
            let get = |key: &str| match pairs.get(&HashKey::String(key.into())) {
                Some(Value::Float(f)) => *f,
                _ => panic!(),
            };
            get("max_lng") - get("min_lng")
        };
        assert!(
            span(60.0) > span(0.0) * 1.9,
            "at 60° a longitude degree is half as wide, so the span must roughly double"
        );
    }

    /// The canonical geohash example, so an error in the bit interleaving shows
    /// up against something external rather than against itself.
    #[test]
    fn geohash_matches_the_reference_value() {
        assert_eq!(geohash(57.64911, 10.40744, 11), "u4pruydqqvj");
    }

    #[test]
    fn geohash_prefixes_mean_proximity() {
        let here = geohash(PARIS.0, PARIS.1, 9);
        let nearby = geohash(PARIS.0 + 0.0001, PARIS.1 + 0.0001, 9);
        let far = geohash(LONDON.0, LONDON.1, 9);
        assert_eq!(&here[..5], &nearby[..5], "neighbours share a prefix");
        assert_ne!(&here[..2], &far[..2], "distant places do not");
    }

    #[test]
    fn geohash_round_trips_within_its_error_bars() {
        let hash = geohash(PARIS.0, PARIS.1, 9);
        let pairs = geohash_decode(&hash).expect("decodes");
        let get = |key: &str| match pairs.get(&HashKey::String(key.into())) {
            Some(Value::Float(f)) => *f,
            _ => panic!("missing {}", key),
        };
        assert!((get("lat") - PARIS.0).abs() <= get("lat_error"));
        assert!((get("lng") - PARIS.1).abs() <= get("lng_error"));
        // Precision 9 is a cell of a few metres.
        assert!(haversine_metres(get("lat"), get("lng"), PARIS.0, PARIS.1) < 10.0);
    }

    #[test]
    fn a_bad_geohash_character_is_rejected() {
        // 'a', 'i', 'l' and 'o' are deliberately absent from the alphabet.
        assert!(geohash_decode("u4pra").is_err());
    }

    #[test]
    fn coordinates_off_the_globe_are_rejected() {
        assert!(check_coordinates(91.0, 0.0, "test").is_err());
        assert!(check_coordinates(0.0, 181.0, "test").is_err());
        assert!(check_coordinates(-90.0, 180.0, "test").is_ok());
    }
}
