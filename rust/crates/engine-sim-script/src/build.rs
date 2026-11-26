//! `EngineNode::buildEngine`: the script's part graph, turned into a real engine.
//!
//! The header ([`crate::engine_dump`]) is what the script *said*; this is what
//! `generate` makes of it. Everything `buildEngine` does, in its order:
//!
//! * the counts, from the lists the `add_*` actions filled;
//! * the fuel and the mean-piston-speed curve, which the engine owns;
//! * the crankshafts, which index the rod journals the rods will look up;
//! * the cylinder banks, and inside them the pistons and rods -- the rods take
//!   their crankshaft from the journal they ride, and a rod with slave journals
//!   takes the rod they hand it as its master;
//! * the heads: the valvetrain (camshafts and their lobes), the two port-flow
//!   curves, and one `HeadCylinder` per cylinder pointing at the intake and the
//!   exhaust system the script gave it;
//! * the ignition module, whose posts resolve to cylinders through the bank
//!   indices the banks just wrote.
//!
//! The difference from the original is ownership, not order: the original's
//! `Engine` owns its parts by value and hangs raw pointers off them, so it can
//! stamp a rod's master after the fact. Here the parts are `Arc`s and the links
//! have to be in place when the `Arc` is shared, so the master rods are resolved
//! after every rod exists and before any piston holds one.
//!
//! Not built here, because nothing in the trace reads them: the script's own
//! `vehicle`/`transmission` (the application -- and the trace harness -- build
//! their own) and the `impulse_response` an exhaust names (the audio layer's
//! table is installed by whoever owns the synthesizer).

use std::sync::Arc;

use engine_sim_core::drivetrain::{Transmission, Vehicle};
use engine_sim_core::engine::Engine;
use engine_sim_core::function::Function;
use engine_sim_core::ignition::IgnitionModule;
use engine_sim_core::simulator::Simulator;
use engine_sim_core::units;
use engine_sim_core::parts::{
    Camshaft, CamshaftParameters, ConnectingRod, ConnectingRodParameters, Crankshaft,
    CrankshaftParameters, CylinderBank, CylinderBankParameters, CylinderHead, DirectThrottle,
    DirectThrottleLinkage, ExhaustSystem, ExhaustSystemParameters, HeadCylinder, Intake,
    IntakeParameters, Piston, PistonParameters, Valvetrain,
};

use crate::objects::{
    BankData, CamshaftData, CrankData, ExhaustData, HeadData, IntakeData, RodData, ValvetrainData,
};
use crate::program::Program;

/// `EngineNode::buildEngine`. `Ok(None)` when the script built no engine, which
/// is a complete answer rather than an error.
pub fn build(program: &mut Program) -> Result<Option<Engine>, String> {
    let engine = match program.context.published_engine() {
        Some(engine) => engine.clone(),
        None => return Ok(None),
    };

    let banks = engine.cylinder_banks.clone();

    // `buildEngine` collects the exhaust systems and intakes out of the
    // cylinders; the index each one gets is its position in that collection.
    let mut intake_nodes: Vec<usize> = Vec::new();
    let mut exhaust_nodes: Vec<usize> = Vec::new();
    let mut cylinder_count = 0;

    for bank in &banks {
        if let Some(data) = program.context.bank(*bank) {
            cylinder_count += data.cylinders.len();

            for cylinder in &data.cylinders {
                if let Some(intake) = cylinder.intake {
                    if !intake_nodes.contains(&intake) {
                        intake_nodes.push(intake);
                    }
                }

                if let Some(exhaust) = cylinder.exhaust {
                    if !exhaust_nodes.contains(&exhaust) {
                        exhaust_nodes.push(exhaust);
                    }
                }
            }
        }
    }

    // The same order the header uses, and the same reason: `buildEngine` collects
    // them in a `std::set` of pointers, which follows allocation.
    intake_nodes.sort_unstable();
    exhaust_nodes.sort_unstable();

    // `Engine::Parameters` -> the engine's own fields.
    let fuel_node = engine
        .fuel
        .ok_or_else(|| "the engine has no fuel".to_string())?;

    let fuel = program
        .fuel(fuel_node)
        .cloned()
        .ok_or_else(|| "the engine's fuel curve is missing".to_string())?;

    let engine_fuel = Arc::new(fuel);

    // `buildEngine` builds this curve itself rather than taking it from the
    // script: `f(i) = i * 0.5` over 30 samples.
    let mut mean_piston_speed_to_turbulence = Function::new(30, 1.0);
    for i in 0..30 {
        let s = i as f64;
        mean_piston_speed_to_turbulence.add_sample(s, s * 0.5);
    }

    let mut built = Engine::new(
        engine.name.clone(),
        engine_fuel,
        Arc::new(mean_piston_speed_to_turbulence),
        IgnitionModule::new(
            cylinder_count,
            Arc::new(Function::new(0, 0.0)),
            engine_sim_core::units::rpm(6000.0),
            0.5 * engine_sim_core::units::SEC,
        ),
    );

    built.starter_torque = engine.starter_torque;
    built.starter_speed = engine.starter_speed;
    built.redline = engine.redline;
    built.dyno_min_speed = engine.dyno_min_speed;
    built.dyno_max_speed = engine.dyno_max_speed;
    built.dyno_hold_step = engine.dyno_hold_step;
    built.initial_simulation_frequency = engine.initial_simulation_frequency;
    built.initial_high_frequency_gain = engine.initial_high_frequency_gain;
    built.initial_jitter = engine.initial_jitter;
    built.initial_noise = engine.initial_noise;

    // The throttle the script wired. An engine with none is one the original
    // cannot run either (`Engine::initialize` stores the pointer and every
    // throttle read dereferences it).
    built.throttle = match engine.throttle {
        Some(node) => {
            let gamma = program.context.throttle(node).map(|t| t.gamma).unwrap_or(0.0);
            Box::new(DirectThrottleLinkage::new(gamma))
        }
        None => Box::new(DirectThrottle::default()),
    };

    // ---------------------------------------------------------- crankshafts
    let mut crankshafts: Vec<Arc<Crankshaft>> = Vec::new();

    for node in &engine.crankshafts {
        let data = program
            .context
            .crankshaft(*node)
            .copied()
            .ok_or_else(|| "a crankshaft has no parameters".to_string())?;

        let journals = program.context.crankshaft_journals(*node).to_vec();

        let crankshaft = Crankshaft::new(CrankParams::new(&data, journals.len()));
        let mut crankshaft = Arc::new(crankshaft);

        for (index, journal) in journals.iter().enumerate() {
            let angle = program
                .context
                .rod_journal(*journal)
                .map(|journal| journal.angle)
                .unwrap_or(0.0);

            Arc::get_mut(&mut crankshaft)
                .expect("the crankshaft is not shared yet")
                .set_rod_journal_angle(index, angle);

            // `CrankshaftNode::generate` also tells the context where each
            // journal lives.
            program.context.add_rod_journal_index(*journal, index as i32);
        }

        crankshafts.push(crankshaft);
    }

    // -------------------------------------------------------------- banks
    let mut bank_data: Vec<Option<BankData>> = Vec::new();
    let mut bank_arcs: Vec<Arc<CylinderBank>> = Vec::new();

    for (index, node) in banks.iter().enumerate() {
        let data = program
            .context
            .bank(*node)
            .cloned()
            .ok_or_else(|| "a cylinder bank has no parameters".to_string())?;

        let bank = CylinderBank::new(CylinderBankParameters {
            angle: data.angle,
            bore: data.bore,
            deck_height: data.deck_height,
            cylinder_count: data.cylinders.len() as i32,
            position_x: data.position_x,
            position_y: data.position_y,
            display_depth: data.display_depth,
            index: index as i32,
        });

        bank_arcs.push(Arc::new(bank));
        bank_data.push(Some(data));
    }

    // `CylinderBankNode::indexSlaveJournals` runs after every crankshaft has
    // generated and before any bank does: a shared crank pin's journal takes its
    // index from the rod that owns it, not from the crankshaft.
    for data in bank_data.iter().flatten() {
        for cylinder in &data.cylinders {
            let Some(rod) = cylinder.rod else {
                continue;
            };

            let journals = program.context.rod_slave_journals(rod).to_vec();
            for (index, journal) in journals.iter().enumerate() {
                program.context.add_rod_journal_index(*journal, index as i32);
            }
        }
    }

    // --------------------------------------------------------- rods, pistons
    let mut rods: Vec<Option<Arc<ConnectingRod>>> = Vec::new();
    let mut rod_nodes: Vec<Option<usize>> = Vec::new();
    let mut rod_master_nodes: Vec<Option<usize>> = Vec::new();

    // Pistons are built after the master rods are resolved: a `Piston` holds an
    // `Arc` to its rod, and the rod cannot be changed once it is shared.
    let mut pending_pistons: Vec<(usize, usize, PendingPiston)> = Vec::new();
    let mut cylinder_base = 0;

    for (bank_index, data) in bank_data.iter().enumerate() {
        let data = match data {
            Some(data) => data,
            None => continue,
        };

        for (local, cylinder) in data.cylinders.iter().enumerate() {
            // The bank tells the context where this cylinder lives in the
            // engine, which is what the ignition posts resolve through.
            program
                .context
                .set_cylinder_index(banks[bank_index], local, (cylinder_base + local) as i32);

            let rod_node = cylinder.rod;
            let journal_node = cylinder.rod_journal;


            let rod_data: RodData = rod_node
                .and_then(|node| program.context.rod(node).copied())
                .unwrap_or_default();

            // A rod rides its journal's crankshaft. A slave rod's journal is on
            // the master rod instead, and `connectRodAssemblies` gives it the
            // master's crankshaft -- which is what this resolves to, without the
            // original's null window in between.
            let journal = journal_node.and_then(|journal| program.context.rod_journal(journal));

            let crankshaft_node = journal
                .and_then(|journal| journal.crankshaft)
                .or_else(|| {
                    journal.and_then(|journal| journal.rod).and_then(|master| {
                        master_crankshaft(&program.context, &bank_data, master)
                    })
                });

            let crankshaft = crankshaft_node
                .and_then(|node| {
                    engine
                        .crankshafts
                        .iter()
                        .position(|candidate| *candidate == node)
                })
                .map(|index| crankshafts[index].clone())
                .ok_or_else(|| "a connecting rod rides a journal with no crankshaft".to_string())?;

            let journal = journal_node
                .map(|journal| program.context.rod_journal_index(journal))
                .unwrap_or(0);

            let slave_journals = rod_node
                .map(|node| program.context.rod_slave_journals(node).to_vec())
                .unwrap_or_default();

            let mut rod = ConnectingRod::new(ConnectingRodParameters {
                center_of_mass: rod_data.center_of_mass,
                length: rod_data.length,
                mass: rod_data.mass,
                moment_of_inertia: rod_data.moment_of_inertia,
                journal,
                crankshaft: Some(crankshaft),
                slave_throw: rod_data.slave_throw,
                rod_journals: slave_journals.len(),
                master: None,
                piston_bank: Some(bank_arcs[bank_index].clone()),
                piston_cylinder_index: local as i32,
            });

            for (index, slave) in slave_journals.iter().enumerate() {
                let angle = program
                    .context
                    .rod_journal(*slave)
                    .map(|journal| journal.angle + engine_sim_core::constants::PI / 2.0)
                    .unwrap_or(engine_sim_core::constants::PI / 2.0);

                rod.set_rod_journal_angle(index, angle);
            }

            // `connectRodAssemblies`: a slave rod takes the rod its journal
            // belongs to as its master. The lookup happens after every rod
            // exists (below), because the master may be a later cylinder's.
            rod_nodes.push(rod_node);
            rod_master_nodes.push(
                journal_node
                    .and_then(|journal| program.context.rod_journal(journal))
                    .and_then(|journal| journal.rod),
            );

            let rod_index = rods.len();
            rods.push(Some(Arc::new(rod)));

            let piston_data = cylinder
                .piston
                .and_then(|node| program.context.piston(node).copied())
                .ok_or_else(|| "a cylinder has no piston".to_string())?;

            pending_pistons.push((rod_index, bank_index, PendingPiston {
                local,
                data: piston_data,
            }));
        }

        cylinder_base += data.cylinders.len();
    }

    // -------------------------------------------------------- master rods
    for (index, master_node) in rod_master_nodes.iter().enumerate() {
        let Some(master_node) = master_node else {
            continue;
        };

        let master = rod_nodes
            .iter()
            .position(|node| node == &Some(*master_node))
            .ok_or_else(|| "a slave rod names a master that does not exist".to_string())?;

        let master_rod = rods[master]
            .clone()
            .ok_or_else(|| "a slave rod names a master that does not exist".to_string())?;

        let crankshaft = master_rod.crankshaft.clone();
        let rod = rods[index]
            .as_mut()
            .expect("the rod was just built");

        let rod = Arc::get_mut(rod).expect("no piston holds the rod yet");
        rod.master = Some(master_rod);
        rod.crankshaft = crankshaft;
    }

    // ------------------------------------------------------------- pistons
    let mut pistons: Vec<Piston> = Vec::new();

    for (rod_index, bank_index, piston) in pending_pistons {
        pistons.push(Piston::new(PistonParameters {
            rod: rods[rod_index].clone(),
            bank: Some(bank_arcs[bank_index].clone()),
            cylinder_index: piston.local as i32,
            compression_height: piston.data.compression_height,
            displacement: piston.data.displacement,
            wrist_pin_position: piston.data.wrist_pin_position,
            mass: piston.data.mass,
            blowby_flow_coefficient: piston.data.blowby_flow_coefficient,
        }));
    }

    // ------------------------------------------------------ intakes, exhausts
    let mut intakes: Vec<Intake> = Vec::new();
    for node in &intake_nodes {
        let data: IntakeData = *program
            .context
            .intake(*node)
            .ok_or_else(|| "an intake has no parameters".to_string())?;

        intakes.push(Intake::new(&IntakeParameters {
            volume: data.volume,
            cross_section_area: data.cross_section_area,
            input_flow_k: data.input_flow_k,
            idle_flow_k: data.idle_flow_k,
            runner_flow_rate: data.runner_flow_rate,
            molecular_afr: data.molecular_afr,
            idle_throttle_plate_position: data.idle_throttle_plate_position,
            runner_length: data.runner_length,
            velocity_decay: data.velocity_decay,
        }));
    }

    let mut exhausts: Vec<ExhaustSystem> = Vec::new();
    for (index, node) in exhaust_nodes.iter().enumerate() {
        let data: ExhaustData = *program
            .context
            .exhaust(*node)
            .ok_or_else(|| "an exhaust system has no parameters".to_string())?;

        let mut exhaust = ExhaustSystem::new(&ExhaustSystemParameters {
            length: data.length,
            collector_cross_section_area: data.collector_cross_section_area,
            outlet_flow_rate: data.outlet_flow_rate,
            primary_tube_length: data.primary_tube_length,
            primary_flow_rate: data.primary_flow_rate,
            velocity_decay: data.velocity_decay,
            audio_volume: data.audio_volume,
            impulse_response: None,
        });

        // `Engine::initialize` stamps this: the flow staging buffer is indexed
        // by it, so two exhaust systems share one only if the index is wrong.
        exhaust.index = index as i32;

        exhausts.push(exhaust);
    }

    // ---------------------------------------------------------------- heads
    let mut heads: Vec<Arc<CylinderHead>> = Vec::new();

    for (bank_index, data) in bank_data.iter().enumerate() {
        let data = match data {
            Some(data) => data,
            None => continue,
        };

        let head_node = data
            .head
            .ok_or_else(|| "a cylinder bank has no head".to_string())?;

        let head: HeadData = *program
            .context
            .head(head_node)
            .ok_or_else(|| "a cylinder head has no parameters".to_string())?;

        let valvetrain_node = head
            .valvetrain
            .ok_or_else(|| "a cylinder head has no valvetrain".to_string())?;

        let valvetrain = build_valvetrain(program, valvetrain_node)?;

        let intake_port_flow = function(program, head.intake_port_flow)?;
        let exhaust_port_flow = function(program, head.exhaust_port_flow)?;

        let mut cylinders = Vec::new();
        for cylinder in &data.cylinders {
            cylinders.push(HeadCylinder {
                exhaust_system: index_of(&exhaust_nodes, cylinder.exhaust),
                intake: index_of(&intake_nodes, cylinder.intake),
                sound_attenuation: cylinder.sound_attenuation,
                header_primary_length: cylinder.primary_length,
            });
        }

        heads.push(Arc::new(CylinderHead {
            bank: bank_arcs[bank_index].clone(),
            valvetrain,
            exhaust_port_flow,
            intake_port_flow,
            intake_runner_volume: head.intake_runner_volume,
            intake_runner_cross_section_area: head.intake_runner_cross_section_area,
            exhaust_runner_volume: head.exhaust_runner_volume,
            exhaust_runner_cross_section_area: head.exhaust_runner_cross_section_area,
            combustion_chamber_volume: head.combustion_chamber_volume,
            flip_display: head.flip_display,
            cylinders,
        }));
    }

    // -------------------------------------------------------------- ignition
    // `IgnitionModuleNode::generate`: the posts are the wires, and a wire knows
    // which cylinders it is connected to.
    if let Some(module) = engine.ignition_module {
        let data = program
            .context
            .ignition_module(module)
            .cloned()
            .ok_or_else(|| "the ignition module has no parameters".to_string())?;

        let timing_curve = function(program, data.timing_curve)?;

        let mut ignition = IgnitionModule::new(
            cylinder_count,
            timing_curve,
            data.rev_limit,
            data.limiter_duration,
        );

        for (wire, angle) in &data.posts {
            for (bank, local) in program.context.wire_connections(*wire).to_vec() {
                let index = program.context.cylinder_index(bank, local);

                if index >= 0 && (index as usize) < cylinder_count {
                    ignition.set_firing_order(index as usize, *angle);
                }
            }
        }

        built.ignition = ignition;
    }

    built.crankshafts = crankshafts;
    built.banks = bank_arcs;
    built.heads = heads;
    built.pistons = pistons;
    built.rods = rods.into_iter().flatten().collect();
    built.intakes = intakes;
    built.exhausts = exhausts;

    Ok(Some(built))
}

/// A piston waiting for the rods to settle.
struct PendingPiston {
    local: usize,
    data: crate::objects::PistonData,
}

/// The crankshaft of the cylinder a rod drives: the journal that rod rides is
/// on the crankshaft, while a slave rod's journal is on its master.
fn master_crankshaft(
    context: &crate::objects::EngineContext,
    bank_data: &[Option<BankData>],
    master: usize,
) -> Option<usize> {
    bank_data
        .iter()
        .flatten()
        .flat_map(|bank| bank.cylinders.iter())
        .find(|cylinder| cylinder.rod == Some(master))
        .and_then(|cylinder| cylinder.rod_journal)
        .and_then(|journal| context.rod_journal(journal))
        .and_then(|journal| journal.crankshaft)
}

fn build_valvetrain(program: &mut Program, node: usize) -> Result<Arc<Valvetrain>, String> {
    let data = program
        .context
        .valvetrain(node)
        .cloned()
        .ok_or_else(|| "a valvetrain has no parameters".to_string())?;

    match data {
        ValvetrainData::Standard {
            intake_camshaft,
            exhaust_camshaft,
        } => Ok(Arc::new(Valvetrain::Standard {
            intake_camshaft: build_camshaft(program, intake_camshaft)?,
            exhaust_camshaft: build_camshaft(program, exhaust_camshaft)?,
        })),
        ValvetrainData::Vtec {
            intake_camshaft,
            exhaust_camshaft,
            vtec_intake_camshaft,
            vtec_exhaust_camshaft,
            min_rpm,
            min_speed,
            min_throttle_position,
            manifold_vacuum,
        } => Ok(Arc::new(Valvetrain::Vtec {
            intake_camshaft: build_camshaft(program, intake_camshaft)?,
            exhaust_camshaft: build_camshaft(program, exhaust_camshaft)?,
            vtec_intake_camshaft: build_camshaft(program, vtec_intake_camshaft)?,
            vtec_exhaust_camshaft: build_camshaft(program, vtec_exhaust_camshaft)?,
            min_rpm,
            min_speed,
            min_throttle_position,
            manifold_vacuum,
        })),
    }
}

fn build_camshaft(program: &mut Program, node: Option<usize>) -> Result<Arc<Camshaft>, String> {
    let node = node.ok_or_else(|| "a valvetrain camshaft input is unconnected".to_string())?;

    let data: CamshaftData = program
        .context
        .camshaft(node)
        .cloned()
        .ok_or_else(|| "a camshaft has no parameters".to_string())?;

    let lobe_profile = function(program, data.lobe_profile)?;

    let mut camshaft = Camshaft::new(CamshaftParameters {
        lobes: data.lobes.len(),
        lobe_profile,
        advance: data.advance,
        base_radius: data.base_radius,
    });

    for (index, centerline) in data.lobes.iter().enumerate() {
        camshaft.set_lobe_centerline(index, *centerline);
    }

    Ok(Arc::new(camshaft))
}

fn function(program: &mut Program, node: Option<usize>) -> Result<Arc<Function>, String> {
    let node = node.ok_or_else(|| "a function input is unconnected".to_string())?;

    program
        .function(node)
        .cloned()
        .map(Arc::new)
        .ok_or_else(|| "a function input names a node that is not a function".to_string())
}

fn index_of(nodes: &[usize], node: Option<usize>) -> Option<usize> {
    node.and_then(|node| nodes.iter().position(|candidate| *candidate == node))
}

/// `Crankshaft::Parameters` from the node's eight numbers.
struct CrankParams;

impl CrankParams {
    fn new(data: &CrankData, journals: usize) -> CrankshaftParameters {
        CrankshaftParameters {
            mass: data.mass,
            flywheel_mass: data.flywheel_mass,
            moment_of_inertia: data.moment_of_inertia,
            crank_throw: data.throw,
            rod_journals: journals,
            pos_x: data.position_x,
            pos_y: data.position_y,
            tdc: data.tdc,
            friction_torque: data.friction_torque,
        }
    }
}

/// The application's half: the engine the script built, wrapped in the vehicle
/// and transmission `EngineSimApplication::loadScript` falls back to when the
/// script brings its own, and set up the way the application sets it up.
///
/// The impulse response is the deterministic one the parity harness installs on
/// every exhaust channel, so the trace's audio is reproducible.
pub fn simulator(program: &mut Program) -> Result<Simulator, String> {
    let engine = build(program)?.ok_or_else(|| "the script builds no engine".to_string())?;

    let vehicle = Vehicle::new(
        units::mass(1597.0, units::KG),
        0.25,
        units::distance(6.0, units::FOOT) * units::distance(6.0, units::FOOT),
        3.42,
        units::distance(10.0, units::INCH),
        2000.0,
    );

    let transmission = Transmission::new(
        vec![2.97, 2.07, 1.43, 1.00, 0.84, 0.56],
        units::torque(1000.0, units::FT_LB),
    );

    let mut simulator = Simulator::new(engine, vehicle, transmission);

    simulator.engine.calculate_displacement();

    // `PistonEngineSimulator::setSimulationFrequency`
    simulator.simulation_frequency = simulator.engine.initial_simulation_frequency;

    {
        let mut audio = simulator.synthesizer.audio_parameters();
        audio.input_sample_noise = simulator.engine.initial_jitter as f32;
        audio.air_noise = simulator.engine.initial_noise as f32;
        audio.df_f_mix = simulator.engine.initial_high_frequency_gain as f32;
        simulator.synthesizer.set_audio_parameters(audio);
    }

    {
        let response = impulse_response(512);
        for i in 0..simulator.engine.get_exhaust_system_count() {
            simulator
                .synthesizer
                .initialize_impulse_response(&response, 1.0, i);
        }
    }

    // The application only spins the starter while [S] is held; the harness holds
    // it so the engine actually runs.
    simulator.set_starter_enabled(true);

    Ok(simulator)
}

/// The deterministic impulse response the reference builds: integer arithmetic
/// only, so every implementation gets the identical table.
pub fn impulse_response(samples: usize) -> Vec<i16> {
    (0..samples)
        .map(|i| {
            let value = ((i as i32 * 7919) % 4001) - 2000;
            let envelope = 255 - (i as i32 % 256);
            (value * envelope / 256) as i16
        })
        .collect()
}
