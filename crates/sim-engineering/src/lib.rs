//! Deterministic spacecraft engineering facts and constraint evaluation.

use serde::{Deserialize, Serialize};
use sim_astro::StateVector;
use sim_time::StableId;
use std::collections::BTreeSet;

pub const SHIP_CATALOG_JSON: &str = include_str!("../../../data/engineering/ships-v1.json");
pub const STANDARD_GRAVITY_MPS2: f64 = 9.806_65;

#[derive(Debug, thiserror::Error)]
pub enum EngineeringError {
    #[error("ENGINEERING_CATALOG_PARSE at {path}: {message}")]
    CatalogParse { path: String, message: String },
    #[error("ENGINEERING_CATALOG_INVALID at {path}: {message}")]
    CatalogInvalid { path: String, message: String },
}

macro_rules! finite_unit {
    ($name:ident, $field:literal, $strictly_positive:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "f64", into = "f64")]
        pub struct $name(f64);

        impl $name {
            pub fn new(value: f64) -> Result<Self, EngineeringError> {
                let in_range = if $strictly_positive {
                    value > 0.0
                } else {
                    value >= 0.0
                };
                if value.is_finite() && in_range {
                    Ok(Self(value))
                } else {
                    Err(invalid(
                        $field,
                        if $strictly_positive {
                            "must be finite and greater than zero"
                        } else {
                            "must be finite and non-negative"
                        },
                    ))
                }
            }

            pub const fn value(self) -> f64 {
                self.0
            }
        }

        impl TryFrom<f64> for $name {
            type Error = EngineeringError;

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
finite_unit!(VolumeCubicMeters, "volume_m3", false);
finite_unit!(PowerWatts, "power_w", false);
finite_unit!(EnergyJoules, "energy_j", false);
finite_unit!(DurationSeconds, "duration_s", false);
finite_unit!(VelocityMetersPerSecond, "velocity_mps", true);
finite_unit!(TemperatureKelvin, "temperature_k", true);
finite_unit!(
    SpecificEnergyJoulesPerKilogram,
    "specific_energy_j_per_kg",
    true
);

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Efficiency(f64);

impl Efficiency {
    pub fn new(value: f64) -> Result<Self, EngineeringError> {
        if value.is_finite() && value > 0.0 && value <= 1.0 {
            Ok(Self(value))
        } else {
            Err(invalid("efficiency", "must be finite and in (0, 1]"))
        }
    }

    pub const fn value(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Efficiency {
    type Error = EngineeringError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Efficiency> for f64 {
    fn from(value: Efficiency) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineeringBasis {
    Fictional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleKind {
    Structure,
    CargoHold,
    FusionReactor,
    Thruster,
    PropellantTank,
    FusionFuelTank,
    Radiator,
    Avionics,
    CrewHabitat,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipModule {
    pub id: StableId,
    pub kind: ModuleKind,
    pub name: String,
    pub dry_mass_kg: MassKilograms,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CargoCapacity {
    pub mass_kg: MassKilograms,
    pub volume_m3: VolumeCubicMeters,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropulsionLimits {
    pub min_effective_exhaust_velocity_mps: VelocityMetersPerSecond,
    pub max_effective_exhaust_velocity_mps: VelocityMetersPerSecond,
    pub max_input_power_w: PowerWatts,
    pub electrical_to_jet_efficiency: Efficiency,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerLimits {
    pub reactor_continuous_output_w: PowerWatts,
    pub propulsion_bus_limit_w: PowerWatts,
    pub hotel_load_w: PowerWatts,
    pub fusion_fuel_specific_electric_energy_j_per_kg: SpecificEnergyJoulesPerKilogram,
    pub reactor_waste_heat_fraction: Efficiency,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThermalLimits {
    pub continuous_heat_rejection_w: PowerWatts,
    pub peak_buffer_capacity_j: EnergyJoules,
    pub max_operating_temperature_k: TemperatureKelvin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LifetimeLimits {
    pub max_continuous_burn_s: DurationSeconds,
    pub reactor_full_power_lifetime_s: DurationSeconds,
    pub engine_full_power_lifetime_s: DurationSeconds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipBlueprint {
    pub id: StableId,
    pub revision: u32,
    pub name: String,
    pub role: String,
    pub engineering_basis: EngineeringBasis,
    pub modules: Vec<ShipModule>,
    pub dry_mass_kg: MassKilograms,
    pub cargo_capacity: CargoCapacity,
    pub fusion_fuel_capacity_kg: MassKilograms,
    pub propellant_capacity_kg: MassKilograms,
    pub propulsion: PropulsionLimits,
    pub power: PowerLimits,
    pub thermal: ThermalLimits,
    pub lifetime: LifetimeLimits,
}

impl ShipBlueprint {
    pub fn validate(&self) -> Result<(), EngineeringError> {
        let prefix = format!("blueprints[{}]", self.id);
        if self.revision == 0 {
            return Err(invalid(format!("{prefix}.revision"), "must be at least 1"));
        }
        if self.name.trim().is_empty() || self.role.trim().is_empty() {
            return Err(invalid(
                format!("{prefix}.name"),
                "name and role must not be empty",
            ));
        }
        if self.dry_mass_kg.value() <= 0.0 {
            return Err(invalid(
                format!("{prefix}.dry_mass_kg"),
                "must be greater than zero",
            ));
        }
        if self.modules.is_empty() {
            return Err(invalid(format!("{prefix}.modules"), "must not be empty"));
        }

        let mut module_ids = BTreeSet::new();
        let mut module_kinds = BTreeSet::new();
        let mut module_mass = 0.0;
        for (index, module) in self.modules.iter().enumerate() {
            if !module_ids.insert(module.id.clone()) {
                return Err(invalid(
                    format!("{prefix}.modules[{index}].id"),
                    "duplicate module id",
                ));
            }
            if module.name.trim().is_empty() {
                return Err(invalid(
                    format!("{prefix}.modules[{index}].name"),
                    "must not be empty",
                ));
            }
            module_kinds.insert(module.kind);
            module_mass += module.dry_mass_kg.value();
        }

        let required = [
            ModuleKind::Structure,
            ModuleKind::CargoHold,
            ModuleKind::FusionReactor,
            ModuleKind::Thruster,
            ModuleKind::PropellantTank,
            ModuleKind::FusionFuelTank,
            ModuleKind::Radiator,
            ModuleKind::Avionics,
        ];
        if let Some(missing) = required.iter().find(|kind| !module_kinds.contains(kind)) {
            return Err(invalid(
                format!("{prefix}.modules"),
                format!("missing required {missing:?} module"),
            ));
        }
        let mass_tolerance = self.dry_mass_kg.value().max(1.0) * 1e-9;
        if (module_mass - self.dry_mass_kg.value()).abs() > mass_tolerance {
            return Err(invalid(
                format!("{prefix}.dry_mass_kg"),
                "must equal the sum of module dry masses",
            ));
        }
        if self.propulsion.min_effective_exhaust_velocity_mps.value()
            >= self.propulsion.max_effective_exhaust_velocity_mps.value()
        {
            return Err(invalid(
                format!("{prefix}.propulsion"),
                "minimum exhaust velocity must be below maximum",
            ));
        }
        if self.propulsion.max_input_power_w.value() <= 0.0
            || self.power.reactor_continuous_output_w.value() <= 0.0
            || self.power.propulsion_bus_limit_w.value() <= 0.0
        {
            return Err(invalid(
                format!("{prefix}.power"),
                "reactor, bus, and propulsion power limits must be greater than zero",
            ));
        }
        if self.propulsion.max_input_power_w.value() > self.power.propulsion_bus_limit_w.value() {
            return Err(invalid(
                format!("{prefix}.propulsion.max_input_power_w"),
                "must not exceed the propulsion bus limit",
            ));
        }
        if self.power.propulsion_bus_limit_w.value() + self.power.hotel_load_w.value()
            > self.power.reactor_continuous_output_w.value()
        {
            return Err(invalid(
                format!("{prefix}.power.reactor_continuous_output_w"),
                "must cover the propulsion bus limit plus hotel load",
            ));
        }
        if self.thermal.continuous_heat_rejection_w.value() <= 0.0
            || self.thermal.peak_buffer_capacity_j.value() <= 0.0
            || self.lifetime.max_continuous_burn_s.value() <= 0.0
            || self.lifetime.reactor_full_power_lifetime_s.value() <= 0.0
            || self.lifetime.engine_full_power_lifetime_s.value() <= 0.0
        {
            return Err(invalid(
                prefix,
                "thermal and lifetime limits must be greater than zero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VesselState {
    pub id: StableId,
    pub blueprint_id: StableId,
    pub blueprint_revision: u32,
    #[serde(default)]
    pub state_vector: Option<StateVector>,
    pub payload_mass_kg: MassKilograms,
    pub payload_volume_m3: VolumeCubicMeters,
    pub fusion_fuel_kg: MassKilograms,
    pub propellant_kg: MassKilograms,
    pub thermal_buffer_j: EnergyJoules,
    pub current_continuous_burn_s: DurationSeconds,
    pub reactor_full_power_used_s: DurationSeconds,
    pub engine_full_power_used_s: DurationSeconds,
    #[serde(default)]
    pub active_plan_id: Option<StableId>,
}

impl VesselState {
    pub fn total_mass_kg(&self, blueprint: &ShipBlueprint) -> f64 {
        blueprint.dry_mass_kg.value()
            + self.payload_mass_kg.value()
            + self.fusion_fuel_kg.value()
            + self.propellant_kg.value()
    }

    pub fn validate(&self, blueprint: &ShipBlueprint) -> Vec<ConstraintViolation> {
        let mut violations = Vec::new();
        if self.blueprint_id != blueprint.id || self.blueprint_revision != blueprint.revision {
            violations.push(ConstraintViolation::new(
                ConstraintCode::BlueprintMismatch,
                "blueprint_revision",
                f64::from(blueprint.revision),
                f64::from(self.blueprint_revision),
                "revision",
            ));
        }
        check_max(
            &mut violations,
            ConstraintCode::CargoMassCapacity,
            "payload_mass_kg",
            self.payload_mass_kg.value(),
            blueprint.cargo_capacity.mass_kg.value(),
            "kg",
        );
        check_max(
            &mut violations,
            ConstraintCode::CargoVolumeCapacity,
            "payload_volume_m3",
            self.payload_volume_m3.value(),
            blueprint.cargo_capacity.volume_m3.value(),
            "m3",
        );
        check_max(
            &mut violations,
            ConstraintCode::PropellantCapacity,
            "propellant_kg",
            self.propellant_kg.value(),
            blueprint.propellant_capacity_kg.value(),
            "kg",
        );
        check_max(
            &mut violations,
            ConstraintCode::FusionFuelCapacity,
            "fusion_fuel_kg",
            self.fusion_fuel_kg.value(),
            blueprint.fusion_fuel_capacity_kg.value(),
            "kg",
        );
        check_max(
            &mut violations,
            ConstraintCode::ThermalBufferCapacity,
            "thermal_buffer_j",
            self.thermal_buffer_j.value(),
            blueprint.thermal.peak_buffer_capacity_j.value(),
            "J",
        );
        check_max(
            &mut violations,
            ConstraintCode::ContinuousBurnLimit,
            "current_continuous_burn_s",
            self.current_continuous_burn_s.value(),
            blueprint.lifetime.max_continuous_burn_s.value(),
            "s",
        );
        check_max(
            &mut violations,
            ConstraintCode::ReactorLifetimeReserve,
            "reactor_full_power_used_s",
            self.reactor_full_power_used_s.value(),
            blueprint.lifetime.reactor_full_power_lifetime_s.value(),
            "s",
        );
        check_max(
            &mut violations,
            ConstraintCode::EngineLifetimeReserve,
            "engine_full_power_used_s",
            self.engine_full_power_used_s.value(),
            blueprint.lifetime.engine_full_power_lifetime_s.value(),
            "s",
        );
        violations
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReservePolicy {
    pub minimum_propellant_kg: MassKilograms,
    pub minimum_fusion_fuel_kg: MassKilograms,
    pub minimum_thermal_buffer_headroom_j: EnergyJoules,
    pub minimum_reactor_lifetime_remaining_s: DurationSeconds,
    pub minimum_engine_lifetime_remaining_s: DurationSeconds,
}

impl ReservePolicy {
    pub fn zero() -> Self {
        Self {
            minimum_propellant_kg: MassKilograms::new(0.0).expect("zero is valid"),
            minimum_fusion_fuel_kg: MassKilograms::new(0.0).expect("zero is valid"),
            minimum_thermal_buffer_headroom_j: EnergyJoules::new(0.0).expect("zero is valid"),
            minimum_reactor_lifetime_remaining_s: DurationSeconds::new(0.0).expect("zero is valid"),
            minimum_engine_lifetime_remaining_s: DurationSeconds::new(0.0).expect("zero is valid"),
        }
    }
}

impl Default for ReservePolicy {
    fn default() -> Self {
        Self::zero()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BurnRequest {
    pub duration_s: DurationSeconds,
    pub propulsion_input_power_w: PowerWatts,
    pub effective_exhaust_velocity_mps: VelocityMetersPerSecond,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BurnOutcome {
    pub initial_mass_kg: f64,
    pub final_mass_kg: f64,
    pub thrust_n: f64,
    pub specific_impulse_s: f64,
    pub mass_flow_kg_per_s: f64,
    pub propellant_consumed_kg: f64,
    pub fusion_fuel_consumed_kg: f64,
    pub reactor_output_w: f64,
    pub peak_waste_heat_w: f64,
    pub final_thermal_buffer_j: f64,
    pub reactor_full_power_equivalent_s: f64,
    pub engine_full_power_equivalent_s: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConstraintCode {
    BlueprintMismatch,
    InvalidRequest,
    CargoMassCapacity,
    CargoVolumeCapacity,
    PropellantCapacity,
    FusionFuelCapacity,
    ThermalBufferCapacity,
    PropulsionPowerLimit,
    ReactorPowerLimit,
    ExhaustVelocityRange,
    PropellantReserve,
    FusionFuelReserve,
    ThermalBufferReserve,
    ContinuousBurnLimit,
    ReactorLifetimeReserve,
    EngineLifetimeReserve,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstraintViolation {
    pub code: ConstraintCode,
    pub field: String,
    pub required: f64,
    pub available: f64,
    pub unit: String,
}

impl ConstraintViolation {
    fn new(
        code: ConstraintCode,
        field: impl Into<String>,
        required: f64,
        available: f64,
        unit: impl Into<String>,
    ) -> Self {
        Self {
            code,
            field: field.into(),
            required,
            available,
            unit: unit.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BurnAssessment {
    pub outcome: Option<BurnOutcome>,
    pub violations: Vec<ConstraintViolation>,
}

impl BurnAssessment {
    pub fn is_feasible(&self) -> bool {
        self.outcome.is_some() && self.violations.is_empty()
    }
}

/// Evaluates a finite burn without mutating vessel state.
///
/// The propulsion relationship is `jet_power = efficiency * input_power`,
/// `thrust = 2 * jet_power / exhaust_velocity`, and
/// `mass_flow = thrust / exhaust_velocity`.
pub fn evaluate_burn(
    blueprint: &ShipBlueprint,
    vessel: &VesselState,
    request: &BurnRequest,
    reserves: &ReservePolicy,
) -> BurnAssessment {
    let mut violations = vessel.validate(blueprint);
    let duration = request.duration_s.value();
    let propulsion_power = request.propulsion_input_power_w.value();
    let exhaust_velocity = request.effective_exhaust_velocity_mps.value();

    if duration <= 0.0 {
        violations.push(ConstraintViolation::new(
            ConstraintCode::InvalidRequest,
            "duration_s",
            f64::EPSILON,
            duration,
            "s",
        ));
    }
    if propulsion_power <= 0.0 {
        violations.push(ConstraintViolation::new(
            ConstraintCode::InvalidRequest,
            "propulsion_input_power_w",
            f64::EPSILON,
            propulsion_power,
            "W",
        ));
    }
    check_max(
        &mut violations,
        ConstraintCode::PropulsionPowerLimit,
        "propulsion_input_power_w",
        propulsion_power,
        blueprint
            .propulsion
            .max_input_power_w
            .value()
            .min(blueprint.power.propulsion_bus_limit_w.value()),
        "W",
    );
    let reactor_output = propulsion_power + blueprint.power.hotel_load_w.value();
    check_max(
        &mut violations,
        ConstraintCode::ReactorPowerLimit,
        "reactor_output_w",
        reactor_output,
        blueprint.power.reactor_continuous_output_w.value(),
        "W",
    );
    let minimum_exhaust = blueprint
        .propulsion
        .min_effective_exhaust_velocity_mps
        .value();
    let maximum_exhaust = blueprint
        .propulsion
        .max_effective_exhaust_velocity_mps
        .value();
    if exhaust_velocity < minimum_exhaust || exhaust_velocity > maximum_exhaust {
        violations.push(ConstraintViolation::new(
            ConstraintCode::ExhaustVelocityRange,
            "effective_exhaust_velocity_mps",
            exhaust_velocity.clamp(minimum_exhaust, maximum_exhaust),
            exhaust_velocity,
            "m/s",
        ));
    }

    let jet_power = blueprint.propulsion.electrical_to_jet_efficiency.value() * propulsion_power;
    let thrust = 2.0 * jet_power / exhaust_velocity;
    let mass_flow = thrust / exhaust_velocity;
    let propellant_consumed = mass_flow * duration;
    let fusion_fuel_consumed = reactor_output * duration
        / blueprint
            .power
            .fusion_fuel_specific_electric_energy_j_per_kg
            .value();
    let peak_waste_heat = reactor_output * blueprint.power.reactor_waste_heat_fraction.value()
        + propulsion_power * (1.0 - blueprint.propulsion.electrical_to_jet_efficiency.value())
        + blueprint.power.hotel_load_w.value();
    let thermal_delta =
        (peak_waste_heat - blueprint.thermal.continuous_heat_rejection_w.value()) * duration;
    let final_thermal_buffer = (vessel.thermal_buffer_j.value() + thermal_delta).max(0.0);
    let reactor_fpe =
        duration * reactor_output / blueprint.power.reactor_continuous_output_w.value();
    let engine_fpe = duration * propulsion_power / blueprint.propulsion.max_input_power_w.value();

    check_reserve(
        &mut violations,
        ConstraintCode::PropellantReserve,
        "propellant_kg",
        propellant_consumed + reserves.minimum_propellant_kg.value(),
        vessel.propellant_kg.value(),
        "kg",
    );
    check_reserve(
        &mut violations,
        ConstraintCode::FusionFuelReserve,
        "fusion_fuel_kg",
        fusion_fuel_consumed + reserves.minimum_fusion_fuel_kg.value(),
        vessel.fusion_fuel_kg.value(),
        "kg",
    );
    check_max(
        &mut violations,
        ConstraintCode::ThermalBufferReserve,
        "thermal_buffer_j",
        final_thermal_buffer + reserves.minimum_thermal_buffer_headroom_j.value(),
        blueprint.thermal.peak_buffer_capacity_j.value(),
        "J",
    );
    check_max(
        &mut violations,
        ConstraintCode::ContinuousBurnLimit,
        "current_continuous_burn_s",
        vessel.current_continuous_burn_s.value() + duration,
        blueprint.lifetime.max_continuous_burn_s.value(),
        "s",
    );
    check_max(
        &mut violations,
        ConstraintCode::ReactorLifetimeReserve,
        "reactor_full_power_used_s",
        vessel.reactor_full_power_used_s.value()
            + reactor_fpe
            + reserves.minimum_reactor_lifetime_remaining_s.value(),
        blueprint.lifetime.reactor_full_power_lifetime_s.value(),
        "s",
    );
    check_max(
        &mut violations,
        ConstraintCode::EngineLifetimeReserve,
        "engine_full_power_used_s",
        vessel.engine_full_power_used_s.value()
            + engine_fpe
            + reserves.minimum_engine_lifetime_remaining_s.value(),
        blueprint.lifetime.engine_full_power_lifetime_s.value(),
        "s",
    );

    if !violations.is_empty() {
        return BurnAssessment {
            outcome: None,
            violations,
        };
    }

    BurnAssessment {
        outcome: Some(BurnOutcome {
            initial_mass_kg: vessel.total_mass_kg(blueprint),
            final_mass_kg: vessel.total_mass_kg(blueprint)
                - propellant_consumed
                - fusion_fuel_consumed,
            thrust_n: thrust,
            specific_impulse_s: exhaust_velocity / STANDARD_GRAVITY_MPS2,
            mass_flow_kg_per_s: mass_flow,
            propellant_consumed_kg: propellant_consumed,
            fusion_fuel_consumed_kg: fusion_fuel_consumed,
            reactor_output_w: reactor_output,
            peak_waste_heat_w: peak_waste_heat,
            final_thermal_buffer_j: final_thermal_buffer,
            reactor_full_power_equivalent_s: reactor_fpe,
            engine_full_power_equivalent_s: engine_fpe,
        }),
        violations,
    }
}

pub fn apply_burn(
    blueprint: &ShipBlueprint,
    vessel: &mut VesselState,
    request: &BurnRequest,
    reserves: &ReservePolicy,
) -> BurnAssessment {
    let assessment = evaluate_burn(blueprint, vessel, request, reserves);
    let Some(outcome) = &assessment.outcome else {
        return assessment;
    };
    vessel.propellant_kg =
        MassKilograms::new(vessel.propellant_kg.value() - outcome.propellant_consumed_kg)
            .expect("feasible burn preserves non-negative propellant");
    vessel.fusion_fuel_kg =
        MassKilograms::new(vessel.fusion_fuel_kg.value() - outcome.fusion_fuel_consumed_kg)
            .expect("feasible burn preserves non-negative fusion fuel");
    vessel.thermal_buffer_j = EnergyJoules::new(outcome.final_thermal_buffer_j)
        .expect("feasible burn preserves non-negative heat buffer");
    vessel.current_continuous_burn_s =
        DurationSeconds::new(vessel.current_continuous_burn_s.value() + request.duration_s.value())
            .expect("feasible burn preserves non-negative duration");
    vessel.reactor_full_power_used_s = DurationSeconds::new(
        vessel.reactor_full_power_used_s.value() + outcome.reactor_full_power_equivalent_s,
    )
    .expect("feasible burn preserves non-negative lifetime");
    vessel.engine_full_power_used_s = DurationSeconds::new(
        vessel.engine_full_power_used_s.value() + outcome.engine_full_power_equivalent_s,
    )
    .expect("feasible burn preserves non-negative lifetime");
    assessment
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineeringCatalogDocument {
    pub schema_version: u32,
    pub content_version: String,
    pub notice: String,
    pub blueprints: Vec<ShipBlueprint>,
}

#[derive(Debug, Clone)]
pub struct EngineeringCatalog {
    document: EngineeringCatalogDocument,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineeringCatalogAudit {
    pub schema_version: u32,
    pub content_version: String,
    pub blueprint_count: usize,
    pub fictional_blueprint_count: usize,
    pub checksum_blake3: String,
}

impl EngineeringCatalog {
    pub fn bundled() -> Result<Self, EngineeringError> {
        Self::from_json(SHIP_CATALOG_JSON)
    }

    pub fn from_json(json: &str) -> Result<Self, EngineeringError> {
        let mut deserializer = serde_json::Deserializer::from_str(json);
        let document: EngineeringCatalogDocument =
            serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
                EngineeringError::CatalogParse {
                    path: error.path().to_string(),
                    message: error.inner().to_string(),
                }
            })?;
        Self::new(document)
    }

    pub fn new(document: EngineeringCatalogDocument) -> Result<Self, EngineeringError> {
        if document.schema_version != 1 {
            return Err(invalid("schema_version", "only schema 1 is supported"));
        }
        if document.content_version.trim().is_empty() || document.notice.trim().is_empty() {
            return Err(invalid(
                "content_version",
                "content version and notice must not be empty",
            ));
        }
        if document.blueprints.len() < 4 {
            return Err(invalid(
                "blueprints",
                "must include at least three ships and one uncrewed probe",
            ));
        }
        let mut revisions = BTreeSet::new();
        for blueprint in &document.blueprints {
            if !revisions.insert((blueprint.id.clone(), blueprint.revision)) {
                return Err(invalid("blueprints", "duplicate blueprint id and revision"));
            }
            blueprint.validate()?;
        }
        if !document
            .blueprints
            .iter()
            .any(|blueprint| blueprint.role == "uncrewed_probe")
        {
            return Err(invalid("blueprints", "must include an uncrewed_probe role"));
        }
        Ok(Self { document })
    }

    pub fn blueprints(&self) -> &[ShipBlueprint] {
        &self.document.blueprints
    }

    pub fn blueprint(&self, id: &StableId, revision: u32) -> Option<&ShipBlueprint> {
        self.document
            .blueprints
            .iter()
            .find(|blueprint| blueprint.id == *id && blueprint.revision == revision)
    }

    pub fn audit(&self) -> Result<EngineeringCatalogAudit, EngineeringError> {
        let canonical = serde_json::to_vec(&self.document)
            .map_err(|error| invalid("catalog", format!("could not serialize catalog: {error}")))?;
        Ok(EngineeringCatalogAudit {
            schema_version: self.document.schema_version,
            content_version: self.document.content_version.clone(),
            blueprint_count: self.document.blueprints.len(),
            fictional_blueprint_count: self
                .document
                .blueprints
                .iter()
                .filter(|blueprint| blueprint.engineering_basis == EngineeringBasis::Fictional)
                .count(),
            checksum_blake3: blake3::hash(&canonical).to_hex().to_string(),
        })
    }
}

fn check_max(
    violations: &mut Vec<ConstraintViolation>,
    code: ConstraintCode,
    field: &str,
    required: f64,
    available: f64,
    unit: &str,
) {
    let tolerance = available.abs().max(1.0) * 1e-12;
    if required > available + tolerance {
        violations.push(ConstraintViolation::new(
            code, field, required, available, unit,
        ));
    }
}

fn check_reserve(
    violations: &mut Vec<ConstraintViolation>,
    code: ConstraintCode,
    field: &str,
    required: f64,
    available: f64,
    unit: &str,
) {
    check_max(violations, code, field, required, available, unit);
}

fn invalid(path: impl Into<String>, message: impl Into<String>) -> EngineeringError {
    EngineeringError::CatalogInvalid {
        path: path.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn blueprint(id: &str) -> ShipBlueprint {
        let catalog = EngineeringCatalog::bundled().unwrap();
        catalog
            .blueprint(&StableId::new(id).unwrap(), 1)
            .unwrap()
            .clone()
    }

    fn loaded_vessel(blueprint: &ShipBlueprint) -> VesselState {
        VesselState {
            id: StableId::new("vessel:test").unwrap(),
            blueprint_id: blueprint.id.clone(),
            blueprint_revision: blueprint.revision,
            state_vector: None,
            payload_mass_kg: MassKilograms::new(blueprint.cargo_capacity.mass_kg.value() * 0.5)
                .unwrap(),
            payload_volume_m3: VolumeCubicMeters::new(
                blueprint.cargo_capacity.volume_m3.value() * 0.5,
            )
            .unwrap(),
            fusion_fuel_kg: blueprint.fusion_fuel_capacity_kg,
            propellant_kg: blueprint.propellant_capacity_kg,
            thermal_buffer_j: EnergyJoules::new(0.0).unwrap(),
            current_continuous_burn_s: DurationSeconds::new(0.0).unwrap(),
            reactor_full_power_used_s: DurationSeconds::new(0.0).unwrap(),
            engine_full_power_used_s: DurationSeconds::new(0.0).unwrap(),
            active_plan_id: None,
        }
    }

    #[test]
    fn bundled_catalog_contains_three_ships_and_an_uncrewed_probe() {
        let catalog = EngineeringCatalog::bundled().unwrap();
        let audit = catalog.audit().unwrap();
        assert_eq!(audit.blueprint_count, 4);
        assert_eq!(audit.fictional_blueprint_count, 4);
        assert_eq!(
            catalog
                .blueprints()
                .iter()
                .filter(|blueprint| blueprint.role != "uncrewed_probe")
                .count(),
            3
        );
    }

    #[test]
    fn fixed_power_golden_standard_captures_thrust_isp_tradeoff() {
        let blueprint = blueprint("ship:lunar-courier");
        let vessel = loaded_vessel(&blueprint);
        let request_at = |velocity| BurnRequest {
            duration_s: DurationSeconds::new(10.0).unwrap(),
            propulsion_input_power_w: PowerWatts::new(500_000_000.0).unwrap(),
            effective_exhaust_velocity_mps: VelocityMetersPerSecond::new(velocity).unwrap(),
        };
        let low = evaluate_burn(
            &blueprint,
            &vessel,
            &request_at(100_000.0),
            &ReservePolicy::zero(),
        )
        .outcome
        .unwrap();
        let high = evaluate_burn(
            &blueprint,
            &vessel,
            &request_at(200_000.0),
            &ReservePolicy::zero(),
        )
        .outcome
        .unwrap();

        assert!((low.thrust_n - 7_200.0).abs() < 1e-9);
        assert!((low.mass_flow_kg_per_s - 0.072).abs() < 1e-12);
        assert!((low.specific_impulse_s - 10_197.162_129_779).abs() < 1e-9);
        assert!((high.thrust_n - 3_600.0).abs() < 1e-9);
        assert!((high.mass_flow_kg_per_s - 0.018).abs() < 1e-12);
        assert!((high.specific_impulse_s - 20_394.324_259_558).abs() < 1e-9);
    }

    #[test]
    fn all_constraint_families_return_stable_structured_reasons() {
        let blueprint = blueprint("ship:lunar-courier");
        let mut vessel = loaded_vessel(&blueprint);
        vessel.payload_mass_kg =
            MassKilograms::new(blueprint.cargo_capacity.mass_kg.value() + 1.0).unwrap();
        vessel.reactor_full_power_used_s =
            DurationSeconds::new(blueprint.lifetime.reactor_full_power_lifetime_s.value() - 1.0)
                .unwrap();
        let request = BurnRequest {
            duration_s: DurationSeconds::new(
                blueprint.lifetime.max_continuous_burn_s.value() + 1.0,
            )
            .unwrap(),
            propulsion_input_power_w: PowerWatts::new(
                blueprint.power.reactor_continuous_output_w.value(),
            )
            .unwrap(),
            effective_exhaust_velocity_mps: VelocityMetersPerSecond::new(1.0).unwrap(),
        };
        let assessment = evaluate_burn(&blueprint, &vessel, &request, &ReservePolicy::zero());
        let codes: Vec<_> = assessment
            .violations
            .iter()
            .map(|violation| violation.code)
            .collect();
        assert!(!assessment.is_feasible());
        assert!(codes.contains(&ConstraintCode::CargoMassCapacity));
        assert!(codes.contains(&ConstraintCode::PropulsionPowerLimit));
        assert!(codes.contains(&ConstraintCode::ReactorPowerLimit));
        assert!(codes.contains(&ConstraintCode::ExhaustVelocityRange));
        assert!(codes.contains(&ConstraintCode::ContinuousBurnLimit));
        assert!(codes.contains(&ConstraintCode::ReactorLifetimeReserve));
        assert!(codes.contains(&ConstraintCode::ThermalBufferReserve));
    }

    proptest! {
        #[test]
        fn feasible_burns_never_produce_negative_or_over_capacity_state(
            duration in 0.1_f64..600.0,
            power_fraction in 0.01_f64..0.8,
            exhaust_fraction in 0.0_f64..1.0,
            payload_fraction in 0.0_f64..1.0,
            propellant_fraction in 0.25_f64..1.0,
            fuel_fraction in 0.25_f64..1.0,
        ) {
            let blueprint = blueprint("ship:interplanetary-freighter");
            let mut vessel = loaded_vessel(&blueprint);
            vessel.payload_mass_kg = MassKilograms::new(
                blueprint.cargo_capacity.mass_kg.value() * payload_fraction,
            ).unwrap();
            vessel.propellant_kg = MassKilograms::new(
                blueprint.propellant_capacity_kg.value() * propellant_fraction,
            ).unwrap();
            vessel.fusion_fuel_kg = MassKilograms::new(
                blueprint.fusion_fuel_capacity_kg.value() * fuel_fraction,
            ).unwrap();
            let minimum = blueprint.propulsion.min_effective_exhaust_velocity_mps.value();
            let maximum = blueprint.propulsion.max_effective_exhaust_velocity_mps.value();
            let request = BurnRequest {
                duration_s: DurationSeconds::new(duration).unwrap(),
                propulsion_input_power_w: PowerWatts::new(
                    blueprint.propulsion.max_input_power_w.value() * power_fraction,
                ).unwrap(),
                effective_exhaust_velocity_mps: VelocityMetersPerSecond::new(
                    minimum + (maximum - minimum) * exhaust_fraction,
                ).unwrap(),
            };
            let assessment = apply_burn(
                &blueprint,
                &mut vessel,
                &request,
                &ReservePolicy::zero(),
            );
            prop_assert!(assessment.is_feasible());
            prop_assert!(vessel.validate(&blueprint).is_empty());
            prop_assert!(vessel.total_mass_kg(&blueprint) >= blueprint.dry_mass_kg.value());
            prop_assert!(vessel.propellant_kg.value() >= 0.0);
            prop_assert!(vessel.fusion_fuel_kg.value() >= 0.0);
            prop_assert!(vessel.payload_mass_kg.value() <= blueprint.cargo_capacity.mass_kg.value());
        }
    }
}
