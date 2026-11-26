//! The `E,...` header: everything the script decided about the engine, read back
//! off it the way the application reads it.
//!
//! This is the first half of M4. The values are the engine's *parameters* --
//! names, counts, curves and part geometry -- not its behaviour: nothing has been
//! simulated when the header is produced, which is why the displacement fields
//! are zero (`Engine::calculateDisplacement` is the application's call, after the
//! script). The behaviour gate is the `RUN` trace, and it needs the port's own
//! engine to step.
//!
//! One order is the original's, not the script's: `buildEngine` iterates the
//! exhaust systems and intakes it collected in a `std::set` of pointers, so with
//! more than one of either the index order follows allocation. Nodes are
//! allocated as they are generated, so this sorts them by node id -- which
//! reproduces the pointer order for every engine in `assets/`, including the
//! ones with two exhaust systems (`atg-video-2/10_lfa_v10.mr`).

use crate::objects::HeadData;
use crate::program::Program;

/// One record per line, in `script_reference --engine`'s order.
pub fn dump(program: &mut Program) -> Vec<String> {
    dump_with_displacement(program, 0.0)
}

/// The same header, with the displacement the application calculated.
///
/// `script_reference --run` dumps the header *after* `calculateDisplacement`, so
/// the one line that differs between the two dumps is this one: everything else
/// is a parameter. `frames` in [`crate::trace`] is what runs behind it.
pub fn dump_with_displacement(program: &mut Program, displacement: f64) -> Vec<String> {
    let mut lines = Vec::new();

    let engine = match program.context.published_engine() {
        Some(engine) => engine.clone(),
        // A script that never calls `set_engine` has no engine to describe, and
        // the reference prints nothing for it.
        None => return lines,
    };

    let banks = engine.cylinder_banks.clone();

    let mut intakes: Vec<usize> = Vec::new();
    let mut exhausts: Vec<usize> = Vec::new();
    let mut cylinder_count = 0;

    for bank in &banks {
        if let Some(bank) = program.context.bank(*bank) {
            cylinder_count += bank.cylinders.len();

            for cylinder in &bank.cylinders {
                if let Some(intake) = cylinder.intake {
                    if !intakes.contains(&intake) {
                        intakes.push(intake);
                    }
                }

                if let Some(exhaust) = cylinder.exhaust {
                    if !exhausts.contains(&exhaust) {
                        exhausts.push(exhaust);
                    }
                }
            }
        }
    }

    // `buildEngine` collects these in a `std::set` of *pointers*, so the index
    // order follows allocation. piranha allocates a node when it is generated,
    // which is the order of the graph's node ids, so that is the order here.
    intakes.sort_unstable();
    exhausts.sort_unstable();

    lines.push(format!("E,name,{}", engine.name));
    lines.push(format!("E,cylinder_banks,{}", banks.len()));
    lines.push(format!("E,cylinder_count,{}", cylinder_count));
    lines.push(format!("E,crankshaft_count,{}", engine.crankshafts.len()));
    lines.push(format!("E,exhaust_system_count,{}", exhausts.len()));
    lines.push(format!("E,intake_count,{}", intakes.len()));

    for (field, value) in [
        ("displacement", displacement),
        ("redline", engine.redline),
        ("starter_torque", engine.starter_torque),
        ("starter_speed", engine.starter_speed),
        ("dyno_min_speed", engine.dyno_min_speed),
        ("dyno_max_speed", engine.dyno_max_speed),
        ("dyno_hold_step", engine.dyno_hold_step),
        ("simulation_frequency", engine.initial_simulation_frequency),
        ("initial_noise", engine.initial_noise),
        ("initial_jitter", engine.initial_jitter),
        (
            "initial_high_frequency_gain",
            engine.initial_high_frequency_gain,
        ),
    ] {
        lines.push(bits(field, value));
    }

    // `EngineNode::buildEngine` fills the engine's one `Fuel` from the node the
    // `fuel` input points at.
    if let Some(node) = engine.fuel {
        if let Some(fuel) = program.fuel(node) {
            for (field, value) in [
                ("fuel_molecular_mass", fuel.molecular_mass()),
                ("fuel_energy_density", fuel.energy_density()),
                ("fuel_density", fuel.density()),
                ("fuel_molecular_afr", fuel.molecular_afr()),
                (
                    "fuel_max_burning_efficiency",
                    fuel.max_burning_efficiency(),
                ),
                (
                    "fuel_burning_efficiency_randomness",
                    fuel.burning_efficiency_randomness(),
                ),
                (
                    "fuel_low_efficiency_attenuation",
                    fuel.low_efficiency_attenuation(),
                ),
                ("fuel_max_turbulence_effect", fuel.max_turbulence_effect()),
                ("fuel_max_dilution_effect", fuel.max_dilution_effect()),
            ] {
                lines.push(bits(field, value));
            }
        }
    }

    // `IgnitionModule::getTimingAdvance` samples the timing curve at minus the
    // crankshaft's angular velocity, which has not turned yet.
    let advance = engine
        .ignition_module
        .and_then(|module| program.context.ignition_module(module))
        .and_then(|module| module.timing_curve)
        .and_then(|curve| program.function(curve).map(|curve| curve.sample_triangle(-0.0)));

    if let Some(advance) = advance {
        lines.push(bits("ignition_advance", advance));
    }

    for (index, node) in engine.crankshafts.iter().enumerate() {
        let Some(crankshaft) = program.context.crankshaft(*node).copied() else {
            continue;
        };

        for (field, value) in [
            ("throw", crankshaft.throw),
            ("moment_of_inertia", crankshaft.moment_of_inertia),
            ("flywheel_mass", crankshaft.flywheel_mass),
            ("friction_torque", crankshaft.friction_torque),
        ] {
            lines.push(bits(&format!("crank{}_{}", index, field), value));
        }

        lines.push(format!(
            "E,crank{}_rod_journals,{}",
            index,
            program.context.crankshaft_journals(*node).len()
        ));
    }

    let mut cylinder_index = 0;

    for bank in &banks {
        let Some(data) = program.context.bank(*bank) else {
            continue;
        };

        let head: Option<HeadData> = data.head.and_then(|head| program.context.head(head)).copied();

        for cylinder in &data.cylinders {
            cylinder_field(&mut lines, program, cylinder_index, cylinder, head);
            cylinder_index += 1;
        }
    }

    for (index, intake) in intakes.iter().enumerate() {
        let Some(intake) = program.context.intake(*intake) else {
            continue;
        };

        for (field, value) in [
            ("runner_length", intake.runner_length),
            ("plenum_area", intake.cross_section_area),
            ("velocity_decay", intake.velocity_decay),
        ] {
            lines.push(bits(&format!("intake{}_{}", index, field), value));
        }
    }

    for (index, exhaust) in exhausts.iter().enumerate() {
        let Some(exhaust) = program.context.exhaust(*exhaust) else {
            continue;
        };

        for (field, value) in [
            ("length", exhaust.length),
            ("primary_tube_length", exhaust.primary_tube_length),
            ("collector_area", exhaust.collector_cross_section_area),
            ("velocity_decay", exhaust.velocity_decay),
            ("audio_volume", exhaust.audio_volume),
        ] {
            lines.push(bits(&format!("exhaust{}_{}", index, field), value));
        }
    }

    lines
}

fn cylinder_field(
    lines: &mut Vec<String>,
    program: &Program,
    index: usize,
    cylinder: &crate::objects::CylinderRefs,
    head: Option<HeadData>,
) {
    let prefix = |field: &str| format!("cyl{}_{}", index, field);

    if let Some(piston) = cylinder.piston.and_then(|node| program.context.piston(node)) {
        for (field, value) in [
            ("piston_displacement", piston.displacement),
            ("piston_mass", piston.mass),
            ("piston_compression_height", piston.compression_height),
            ("piston_wrist_pin", piston.wrist_pin_position),
            ("piston_blowby_k", piston.blowby_flow_coefficient),
        ] {
            lines.push(bits(&prefix(field), value));
        }
    }

    if let Some(rod) = cylinder.rod.and_then(|node| program.context.rod(node)) {
        for (field, value) in [
            ("rod_length", rod.length),
            ("rod_mass", rod.mass),
            ("rod_moment_of_inertia", rod.moment_of_inertia),
            ("rod_center_of_mass", rod.center_of_mass),
        ] {
            lines.push(bits(&prefix(field), value));
        }
    }

    // `EngineNode::buildEngine` initializes each chamber with its bank's head.
    if let Some(head) = head {
        for (field, value) in [
            ("chamber_volume", head.combustion_chamber_volume),
            ("head_intake_runner_volume", head.intake_runner_volume),
            (
                "head_intake_runner_area",
                head.intake_runner_cross_section_area,
            ),
            ("head_exhaust_runner_volume", head.exhaust_runner_volume),
            (
                "head_exhaust_runner_area",
                head.exhaust_runner_cross_section_area,
            ),
        ] {
            lines.push(bits(&prefix(field), value));
        }
    }
}

fn bits(field: &str, value: f64) -> String {
    format!("E,{},{:016x}", field, value.to_bits())
}
