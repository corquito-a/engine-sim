//! Port of `include/ignition_module.h` / `src/ignition_module.cpp`.

use std::sync::Arc;

use crate::constants;
use crate::function::Function;
use crate::utilities::positive_mod;

/// `IgnitionModule::SparkPlug`
#[derive(Clone, Copy, Debug, Default)]
pub struct SparkPlug {
    pub angle: f64,
    pub enabled: bool,
    pub ignition_event: bool,
}

/// `IgnitionModule`
pub struct IgnitionModule {
    pub plugs: Vec<SparkPlug>,

    pub timing_curve: Arc<Function>,

    pub last_crankshaft_angle: f64,
    pub enabled: bool,
    pub rev_limit_timer: f64,
    pub rev_limit: f64,
    pub limiter_duration: f64,
}

impl IgnitionModule {
    pub fn new(cylinder_count: usize, timing_curve: Arc<Function>, rev_limit: f64, limiter_duration: f64) -> Self {
        Self {
            plugs: vec![SparkPlug::default(); cylinder_count],
            timing_curve,
            last_crankshaft_angle: 0.0,
            enabled: false,
            rev_limit_timer: 0.0,
            rev_limit,
            limiter_duration,
        }
    }

    /// `IgnitionModule::setFiringOrder`
    pub fn set_firing_order(&mut self, cylinder_index: usize, angle: f64) {
        self.plugs[cylinder_index].angle = angle;
        self.plugs[cylinder_index].enabled = true;
    }

    /// `IgnitionModule::reset`
    pub fn reset(&mut self, cycle_angle: f64) {
        self.last_crankshaft_angle = cycle_angle;
        self.reset_ignition_events();
    }

    /// `IgnitionModule::resetIgnitionEvents`
    pub fn reset_ignition_events(&mut self) {
        for plug in self.plugs.iter_mut() {
            plug.ignition_event = false;
        }
    }

    /// `IgnitionModule::getTimingAdvance`
    pub fn get_timing_advance(&self, v_theta: f64) -> f64 {
        // Note: the reference negates the signed angular velocity rather than
        // taking its magnitude, so a counter-clockwise crank samples the
        // curve at negative speeds.
        self.timing_curve.sample_triangle(-v_theta)
    }

    /// `IgnitionModule::update`
    pub fn update(&mut self, dt: f64, cycle_angle: f64, v_theta: f64) {
        if self.enabled && self.rev_limit_timer == 0.0 {
            let four_pi = 4.0 * constants::PI;
            let advance = self.get_timing_advance(v_theta);

            for i in 0..self.plugs.len() {
                let mut adjusted_angle = positive_mod(self.plugs[i].angle - advance, four_pi);
                let r0 = self.last_crankshaft_angle;
                let mut r1 = cycle_angle;

                if v_theta < 0.0 {
                    if r1 < r0 {
                        r1 += four_pi;
                        adjusted_angle += four_pi;
                    }

                    if adjusted_angle >= r0 && adjusted_angle < r1 {
                        self.plugs[i].ignition_event = self.plugs[i].enabled;
                    }
                } else {
                    if r1 > r0 {
                        r1 -= four_pi;
                        adjusted_angle -= four_pi;
                    }

                    if adjusted_angle >= r1 && adjusted_angle < r0 {
                        self.plugs[i].ignition_event = self.plugs[i].enabled;
                    }
                }
            }
        }

        self.rev_limit_timer -= dt;
        if v_theta.abs() > self.rev_limit {
            self.rev_limit_timer = self.limiter_duration;
        }

        if self.rev_limit_timer < 0.0 {
            self.rev_limit_timer = 0.0;
        }

        self.last_crankshaft_angle = cycle_angle;
    }

    /// `IgnitionModule::getIgnitionEvent`
    pub fn get_ignition_event(&self, index: usize) -> bool {
        self.plugs[index].ignition_event
    }

    /// `IgnitionModule::getPlug`
    pub fn get_plug(&mut self, i: i32) -> &mut SparkPlug {
        let n = self.plugs.len() as i32;
        let index = ((i % n) + n) % n;
        &mut self.plugs[index as usize]
    }
}
