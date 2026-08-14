//! Versioned celestial catalog, coordinate frames, and deterministic analytic ephemerides.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sim_time::{StableId, TdbInstant, MICROS_PER_SECOND};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f64::consts::TAU;
use std::sync::Arc;

pub const CATALOG_JSON: &str = include_str!("../../../data/catalog/solar-system-v1.json");
pub const EXPECTED_BODY_IDS: &[&str] = &[
    "sun",
    "mercury",
    "venus",
    "earth",
    "mars",
    "jupiter",
    "saturn",
    "uranus",
    "neptune",
    "ceres",
    "pluto",
    "eris",
    "haumea",
    "makemake",
    "moon",
    "phobos",
    "deimos",
    "io",
    "europa",
    "ganymede",
    "callisto",
    "mimas",
    "enceladus",
    "tethys",
    "dione",
    "rhea",
    "titan",
    "iapetus",
    "ariel",
    "umbriel",
    "titania",
    "oberon",
    "miranda",
    "triton",
    "charon",
    "vesta",
    "pallas",
    "hygiea",
    "psyche",
    "chiron",
    "arrokoth",
];

#[derive(Debug, thiserror::Error)]
pub enum AstroError {
    #[error("CATALOG_PARSE at {path}: {message}")]
    CatalogParse { path: String, message: String },
    #[error("CATALOG_INVALID at {path}: {message}")]
    CatalogInvalid { path: String, message: String },
    #[error("BODY_NOT_FOUND: {0}")]
    BodyNotFound(String),
    #[error("FRAME_MISMATCH: expected {expected}, got {actual}")]
    FrameMismatch { expected: String, actual: String },
    #[error("NON_FINITE_VALUE at {0}")]
    NonFinite(&'static str),
}

macro_rules! finite_unit {
    ($name:ident, $label:literal, $allow_zero:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "f64", into = "f64")]
        pub struct $name(f64);

        impl $name {
            pub fn new(value: f64) -> Result<Self, AstroError> {
                let range_ok = if $allow_zero {
                    value >= 0.0
                } else {
                    value > 0.0
                };
                if value.is_finite() && range_ok {
                    Ok(Self(value))
                } else {
                    Err(AstroError::CatalogInvalid {
                        path: $label.into(),
                        message: "must be finite and non-negative in the declared SI unit".into(),
                    })
                }
            }

            pub const fn value(self) -> f64 {
                self.0
            }
        }

        impl TryFrom<f64> for $name {
            type Error = AstroError;
            fn try_from(value: f64) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for f64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

finite_unit!(MassKilograms, "mass_kg", false);
finite_unit!(DistanceMeters, "distance_m", true);
finite_unit!(DurationSeconds, "duration_s", false);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FrameId(StableId);

impl FrameId {
    pub fn new(value: impl Into<String>) -> Result<Self, AstroError> {
        StableId::new(value)
            .map(Self)
            .map_err(|error| AstroError::CatalogInvalid {
                path: "frame_id".into(),
                message: error.to_string(),
            })
    }

    pub fn heliocentric() -> Self {
        Self::new("frame:heliocentric-ecliptic-j2000").expect("static frame id is valid")
    }

    pub fn parent_local(parent: &StableId) -> Self {
        Self::new(format!("frame:parent-{}", parent.as_str())).expect("body id forms valid frame")
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl TryFrom<String> for FrameId {
    type Error = AstroError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<FrameId> for String {
    fn from(value: FrameId) -> Self {
        value.0.into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyClass {
    Star,
    Planet,
    DwarfPlanet,
    Moon,
    Asteroid,
    Centaur,
    KuiperBeltObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataQuality {
    Approximate,
    Reference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    PublicReference,
    Approximation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSource {
    pub name: String,
    pub url: String,
    pub kind: SourceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryStatus {
    Observed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentStatus {
    Observed,
    Surveyable,
    TransitOnly,
    CommercialOpen,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitalElements {
    pub semi_major_axis_m: DistanceMeters,
    pub eccentricity: f64,
    pub inclination_deg: f64,
    pub longitude_ascending_node_deg: f64,
    pub argument_periapsis_deg: f64,
    pub mean_anomaly_at_epoch_deg: f64,
    pub orbital_period_s: DurationSeconds,
    pub epoch_tdb_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CelestialBody {
    pub id: StableId,
    pub canonical_name: String,
    pub localized_name_zh: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub parent_id: Option<StableId>,
    pub body_class: BodyClass,
    pub mass_kg: MassKilograms,
    pub mean_radius_m: DistanceMeters,
    pub rotation_period_s: Option<DurationSeconds>,
    pub ephemeris: Option<OrbitalElements>,
    pub ephemeris_source: DataSource,
    pub data_quality: DataQuality,
    pub discovery_status: DiscoveryStatus,
    pub development_status: DevelopmentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    pub id: StableId,
    pub canonical_name: String,
    pub localized_name_zh: String,
    pub inner_radius_m: DistanceMeters,
    pub outer_radius_m: DistanceMeters,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogDocument {
    pub schema_version: u32,
    pub content_version: String,
    pub epoch: String,
    pub bodies: Vec<CelestialBody>,
    pub regions: Vec<Region>,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    document: CatalogDocument,
    by_id: BTreeMap<StableId, usize>,
    children: BTreeMap<Option<StableId>, Vec<StableId>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogAudit {
    pub schema_version: u32,
    pub content_version: String,
    pub body_count: usize,
    pub region_count: usize,
    pub checksum_blake3: String,
}

impl Catalog {
    pub fn bundled() -> Result<Self, AstroError> {
        Self::from_json(CATALOG_JSON)
    }

    pub fn from_json(json: &str) -> Result<Self, AstroError> {
        let mut deserializer = serde_json::Deserializer::from_str(json);
        let document: CatalogDocument = serde_path_to_error::deserialize(&mut deserializer)
            .map_err(|error| AstroError::CatalogParse {
                path: error.path().to_string(),
                message: error.inner().to_string(),
            })?;
        Self::new(document)
    }

    pub fn new(document: CatalogDocument) -> Result<Self, AstroError> {
        if document.schema_version != 1 {
            return Err(invalid(
                "schema_version",
                "only catalog schema 1 is supported",
            ));
        }
        if document.content_version.trim().is_empty() {
            return Err(invalid("content_version", "must not be empty"));
        }
        let mut by_id = BTreeMap::new();
        for (index, body) in document.bodies.iter().enumerate() {
            if by_id.insert(body.id.clone(), index).is_some() {
                return Err(invalid(
                    format!("bodies[{index}].id"),
                    "duplicate stable id",
                ));
            }
            validate_body(body, index)?;
        }
        let root_count = document
            .bodies
            .iter()
            .filter(|body| body.parent_id.is_none())
            .count();
        if root_count != 1 || !by_id.contains_key(&StableId::new("sun").expect("valid")) {
            return Err(invalid(
                "bodies",
                "catalog must have the Sun as its single root",
            ));
        }
        for (index, body) in document.bodies.iter().enumerate() {
            if let Some(parent) = &body.parent_id {
                if !by_id.contains_key(parent) {
                    return Err(invalid(
                        format!("bodies[{index}].parent_id"),
                        "parent does not exist",
                    ));
                }
            }
        }
        for body in &document.bodies {
            let mut seen = BTreeSet::new();
            let mut cursor = Some(body.id.clone());
            while let Some(id) = cursor {
                if !seen.insert(id.clone()) {
                    return Err(invalid(
                        format!("bodies[{}].parent_id", by_id[&body.id]),
                        "hierarchy contains a cycle",
                    ));
                }
                cursor = document.bodies[by_id[&id]].parent_id.clone();
            }
        }
        let mut region_ids = BTreeSet::new();
        for (index, region) in document.regions.iter().enumerate() {
            if by_id.contains_key(&region.id) || !region_ids.insert(region.id.clone()) {
                return Err(invalid(
                    format!("regions[{index}].id"),
                    "region ID collides with catalog",
                ));
            }
            if region.inner_radius_m.value() >= region.outer_radius_m.value() {
                return Err(invalid(
                    format!("regions[{index}]"),
                    "inner radius must be below outer radius",
                ));
            }
        }
        let mut children: BTreeMap<Option<StableId>, Vec<StableId>> = BTreeMap::new();
        for body in &document.bodies {
            children
                .entry(body.parent_id.clone())
                .or_default()
                .push(body.id.clone());
        }
        for entries in children.values_mut() {
            entries.sort();
        }
        Ok(Self {
            document,
            by_id,
            children,
        })
    }

    pub fn audit(&self) -> Result<CatalogAudit, AstroError> {
        let present: BTreeSet<_> = self
            .document
            .bodies
            .iter()
            .map(|body| body.id.as_str())
            .collect();
        let missing: Vec<_> = EXPECTED_BODY_IDS
            .iter()
            .filter(|id| !present.contains(**id))
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(invalid(
                "bodies",
                format!("missing required bodies: {}", missing.join(", ")),
            ));
        }
        if self.document.bodies.iter().any(|body| {
            body.discovery_status != DiscoveryStatus::Observed
                || body.development_status != DevelopmentStatus::Observed
        }) {
            return Err(invalid(
                "bodies",
                "all alpha-v0.1 bodies must start Observed",
            ));
        }
        let canonical = serde_json::to_vec(&self.document)
            .map_err(|error| invalid("catalog", error.to_string()))?;
        Ok(CatalogAudit {
            schema_version: self.document.schema_version,
            content_version: self.document.content_version.clone(),
            body_count: self.document.bodies.len(),
            region_count: self.document.regions.len(),
            checksum_blake3: blake3::hash(&canonical).to_hex().to_string(),
        })
    }

    pub fn body(&self, id: &StableId) -> Result<&CelestialBody, AstroError> {
        self.by_id
            .get(id)
            .map(|index| &self.document.bodies[*index])
            .ok_or_else(|| AstroError::BodyNotFound(id.to_string()))
    }

    pub fn bodies(&self) -> &[CelestialBody] {
        &self.document.bodies
    }

    pub fn regions(&self) -> &[Region] {
        &self.document.regions
    }

    pub fn content_version(&self) -> &str {
        &self.document.content_version
    }

    pub fn children_of(&self, parent: Option<&StableId>) -> Vec<&CelestialBody> {
        let key = parent.cloned();
        self.children
            .get(&key)
            .into_iter()
            .flatten()
            .filter_map(|id| self.body(id).ok())
            .collect()
    }

    pub fn search(&self, query: &str) -> Vec<&CelestialBody> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return self.document.bodies.iter().collect();
        }
        self.document
            .bodies
            .iter()
            .filter(|body| {
                body.id.as_str().contains(&query)
                    || body.canonical_name.to_lowercase().contains(&query)
                    || body.localized_name_zh.contains(&query)
                    || body
                        .aliases
                        .iter()
                        .any(|alias| alias.to_lowercase().contains(&query))
            })
            .collect()
    }
}

fn invalid(path: impl Into<String>, message: impl Into<String>) -> AstroError {
    AstroError::CatalogInvalid {
        path: path.into(),
        message: message.into(),
    }
}

fn validate_body(body: &CelestialBody, index: usize) -> Result<(), AstroError> {
    let path = |field: &str| format!("bodies[{index}].{field}");
    if body.canonical_name.trim().is_empty() || body.localized_name_zh.trim().is_empty() {
        return Err(invalid(
            path("canonical_name"),
            "display names must not be empty",
        ));
    }
    if body.parent_id.is_none() != (body.body_class == BodyClass::Star) {
        return Err(invalid(
            path("parent_id"),
            "only the root star may omit a parent",
        ));
    }
    if body.parent_id.is_some() != body.ephemeris.is_some() {
        return Err(invalid(
            path("ephemeris"),
            "non-root bodies require orbital elements",
        ));
    }
    if let Some(elements) = &body.ephemeris {
        let values = [
            elements.eccentricity,
            elements.inclination_deg,
            elements.longitude_ascending_node_deg,
            elements.argument_periapsis_deg,
            elements.mean_anomaly_at_epoch_deg,
        ];
        if values.iter().any(|value| !value.is_finite())
            || !(0.0..1.0).contains(&elements.eccentricity)
        {
            return Err(invalid(
                path("ephemeris"),
                "orbital elements must be finite and 0 <= eccentricity < 1",
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateVector {
    pub frame_id: FrameId,
    pub epoch_tdb: TdbInstant,
    pub position_m: [f64; 3],
    pub velocity_mps: [f64; 3],
}

impl StateVector {
    pub fn new(
        frame_id: FrameId,
        epoch_tdb: TdbInstant,
        position_m: [f64; 3],
        velocity_mps: [f64; 3],
    ) -> Result<Self, AstroError> {
        if position_m
            .iter()
            .chain(velocity_mps.iter())
            .any(|value| !value.is_finite())
        {
            return Err(AstroError::NonFinite("state_vector"));
        }
        Ok(Self {
            frame_id,
            epoch_tdb,
            position_m,
            velocity_mps,
        })
    }

    pub fn relative_to(&self, origin: &Self, result_frame: FrameId) -> Result<Self, AstroError> {
        if self.frame_id != origin.frame_id || self.epoch_tdb != origin.epoch_tdb {
            return Err(AstroError::FrameMismatch {
                expected: format!("{} at {:?}", self.frame_id.as_str(), self.epoch_tdb),
                actual: format!("{} at {:?}", origin.frame_id.as_str(), origin.epoch_tdb),
            });
        }
        StateVector::new(
            result_frame,
            self.epoch_tdb,
            subtract(self.position_m, origin.position_m),
            subtract(self.velocity_mps, origin.velocity_mps),
        )
    }

    pub fn translated_by(&self, origin: &Self, result_frame: FrameId) -> Result<Self, AstroError> {
        if self.epoch_tdb != origin.epoch_tdb {
            return Err(AstroError::FrameMismatch {
                expected: format!("epoch {:?}", self.epoch_tdb),
                actual: format!("epoch {:?}", origin.epoch_tdb),
            });
        }
        StateVector::new(
            result_frame,
            self.epoch_tdb,
            add(self.position_m, origin.position_m),
            add(self.velocity_mps, origin.velocity_mps),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BodyState {
    pub body_id: StableId,
    #[serde(flatten)]
    pub state: StateVector,
    pub quality: DataQuality,
    pub source: DataSource,
}

#[derive(Clone)]
pub struct EphemerisService {
    catalog: Arc<Catalog>,
    cache: Arc<RwLock<HashMap<(StableId, i64), BodyState>>>,
}

impl EphemerisService {
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self {
            catalog,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn state(&self, body_id: &StableId, time: TdbInstant) -> Result<BodyState, AstroError> {
        let key = (body_id.clone(), time.micros_since_j2000());
        if let Some(cached) = self.cache.read().get(&key) {
            return Ok(cached.clone());
        }
        let state = self.compute_heliocentric(body_id, time)?;
        self.cache.write().insert(key, state.clone());
        Ok(state)
    }

    pub fn local_state(
        &self,
        body_id: &StableId,
        time: TdbInstant,
    ) -> Result<BodyState, AstroError> {
        let body = self.catalog.body(body_id)?;
        let parent = body.parent_id.as_ref();
        let vector = if let (Some(elements), Some(parent_id)) = (&body.ephemeris, parent) {
            orbital_state(elements, time, FrameId::parent_local(parent_id))?
        } else {
            StateVector::new(FrameId::heliocentric(), time, [0.0; 3], [0.0; 3])?
        };
        Ok(BodyState {
            body_id: body.id.clone(),
            state: vector,
            quality: body.data_quality,
            source: body.ephemeris_source.clone(),
        })
    }

    fn compute_heliocentric(
        &self,
        body_id: &StableId,
        time: TdbInstant,
    ) -> Result<BodyState, AstroError> {
        let body = self.catalog.body(body_id)?;
        let state = if let (Some(elements), Some(parent_id)) = (&body.ephemeris, &body.parent_id) {
            let local = orbital_state(elements, time, FrameId::parent_local(parent_id))?;
            let parent = self.state(parent_id, time)?;
            local.translated_by(&parent.state, FrameId::heliocentric())?
        } else {
            StateVector::new(FrameId::heliocentric(), time, [0.0; 3], [0.0; 3])?
        };
        Ok(BodyState {
            body_id: body.id.clone(),
            state,
            quality: body.data_quality,
            source: body.ephemeris_source.clone(),
        })
    }

    pub fn map_sample(&self, time: TdbInstant) -> Result<Vec<BodyState>, AstroError> {
        self.catalog
            .bodies()
            .iter()
            .map(|body| self.state(&body.id, time))
            .collect()
    }

    pub fn cache_len(&self) -> usize {
        self.cache.read().len()
    }
}

fn orbital_state(
    elements: &OrbitalElements,
    time: TdbInstant,
    frame: FrameId,
) -> Result<StateVector, AstroError> {
    let delta_seconds =
        (time.micros_since_j2000() - elements.epoch_tdb_micros) as f64 / MICROS_PER_SECOND as f64;
    let period = elements.orbital_period_s.value();
    let mean_motion = TAU / period;
    let mean_anomaly = (elements.mean_anomaly_at_epoch_deg.to_radians()
        + mean_motion * delta_seconds)
        .rem_euclid(TAU);
    let eccentricity = elements.eccentricity;
    let mut eccentric_anomaly = mean_anomaly;
    for _ in 0..12 {
        eccentric_anomaly -=
            (eccentric_anomaly - eccentricity * eccentric_anomaly.sin() - mean_anomaly)
                / (1.0 - eccentricity * eccentric_anomaly.cos());
    }
    let axis = elements.semi_major_axis_m.value();
    let x = axis * (eccentric_anomaly.cos() - eccentricity);
    let y = axis * (1.0 - eccentricity * eccentricity).sqrt() * eccentric_anomaly.sin();
    let denominator = 1.0 - eccentricity * eccentric_anomaly.cos();
    let vx = -axis * mean_motion * eccentric_anomaly.sin() / denominator;
    let vy =
        axis * mean_motion * (1.0 - eccentricity * eccentricity).sqrt() * eccentric_anomaly.cos()
            / denominator;
    let rotation = orbital_rotation(elements);
    StateVector::new(
        frame,
        time,
        apply(rotation, [x, y, 0.0]),
        apply(rotation, [vx, vy, 0.0]),
    )
}

fn orbital_rotation(elements: &OrbitalElements) -> [[f64; 3]; 3] {
    let node = elements.longitude_ascending_node_deg.to_radians();
    let inclination = elements.inclination_deg.to_radians();
    let periapsis = elements.argument_periapsis_deg.to_radians();
    let (sn, cn) = node.sin_cos();
    let (si, ci) = inclination.sin_cos();
    let (sp, cp) = periapsis.sin_cos();
    [
        [cn * cp - sn * sp * ci, -cn * sp - sn * cp * ci, sn * si],
        [sn * cp + cn * sp * ci, -sn * sp + cn * cp * ci, -cn * si],
        [sp * si, cp * si, ci],
    ]
}

fn apply(matrix: [[f64; 3]; 3], vector: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| {
        row.iter()
            .zip(vector)
            .map(|(left, right)| left * right)
            .sum()
    })
}

fn add(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_complete_and_regions_are_not_bodies() {
        let catalog = Catalog::bundled().unwrap();
        let audit = catalog.audit().unwrap();
        assert_eq!(audit.body_count, EXPECTED_BODY_IDS.len());
        assert_eq!(audit.region_count, 3);
        assert!(catalog
            .body(&StableId::new("asteroid-belt").unwrap())
            .is_err());
    }

    #[test]
    fn development_permission_does_not_control_catalog_loading() {
        let commercially_open = CATALOG_JSON.replacen(
            "\"development_status\": \"observed\"",
            "\"development_status\": \"commercial_open\"",
            1,
        );
        let catalog = Catalog::from_json(&commercially_open).unwrap();
        let sun = catalog.body(&StableId::new("sun").unwrap()).unwrap();
        assert_eq!(sun.development_status, DevelopmentStatus::CommercialOpen);
    }

    #[test]
    fn catalog_errors_include_a_field_path() {
        let malformed =
            CATALOG_JSON.replacen("\"mass_kg\": 1.9885e30", "\"mass_kg\": \"heavy\"", 1);
        let error = Catalog::from_json(&malformed).unwrap_err().to_string();
        assert!(error.contains("bodies[0].mass_kg"), "{error}");
    }

    #[test]
    fn unit_types_reject_nan_infinity_and_negative_values() {
        for invalid in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(MassKilograms::new(invalid).is_err());
        }
        assert_eq!(DistanceMeters::new(1_000.0).unwrap().value(), 1_000.0);
    }

    #[test]
    fn local_heliocentric_conversion_round_trips() {
        let catalog = Arc::new(Catalog::bundled().unwrap());
        let ephemeris = EphemerisService::new(catalog);
        let earth_id = StableId::new("earth").unwrap();
        let moon_id = StableId::new("moon").unwrap();
        let earth = ephemeris.state(&earth_id, TdbInstant::J2000).unwrap();
        let moon = ephemeris.state(&moon_id, TdbInstant::J2000).unwrap();
        let local = moon
            .state
            .relative_to(&earth.state, FrameId::parent_local(&earth_id))
            .unwrap();
        let restored = local
            .translated_by(&earth.state, FrameId::heliocentric())
            .unwrap();
        for (actual, expected) in restored.position_m.iter().zip(moon.state.position_m) {
            assert!((actual - expected).abs() < 1e-4);
        }
    }

    #[test]
    fn public_reference_samples_are_plausible() {
        let catalog = Arc::new(Catalog::bundled().unwrap());
        let ephemeris = EphemerisService::new(catalog);
        for (id, expected_radius, tolerance) in [
            ("earth", 149.6e9, 3.0e9),
            ("moon", 384.4e6, 25.0e6),
            ("ganymede", 1.070e9, 30.0e6),
            ("titan", 1.222e9, 40.0e6),
            ("titania", 435.9e6, 15.0e6),
            ("triton", 354.8e6, 15.0e6),
        ] {
            let body_id = StableId::new(id).unwrap();
            let local = ephemeris.local_state(&body_id, TdbInstant::J2000).unwrap();
            let radius = local
                .state
                .position_m
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .sqrt();
            assert!(
                (radius - expected_radius).abs() < tolerance,
                "{id}: {radius}"
            );
        }
        let sun = ephemeris
            .state(&StableId::new("sun").unwrap(), TdbInstant::J2000)
            .unwrap();
        assert_eq!(sun.state.position_m, [0.0; 3]);
    }

    #[test]
    fn ephemeris_queries_are_cached() {
        let catalog = Arc::new(Catalog::bundled().unwrap());
        let ephemeris = EphemerisService::new(catalog);
        ephemeris
            .state(&StableId::new("earth").unwrap(), TdbInstant::J2000)
            .unwrap();
        ephemeris
            .state(&StableId::new("earth").unwrap(), TdbInstant::J2000)
            .unwrap();
        assert_eq!(ephemeris.cache_len(), 2); // Earth plus its parent Sun.
    }
}
