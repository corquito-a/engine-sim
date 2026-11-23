//! Port of `atg_scs` — the 2D rigid-body constraint solver in
//! `dependencies/submodules/simple-2d-constraint-solver`.
//!
//! Only the subset the engine simulator uses is ported, but within that subset
//! the transliteration is exact:
//!
//! * `Matrix` / `SparseMatrix<3, 2>` — dense and block-sparse arithmetic.
//! * `RigidBody`, `SystemState` — bodies and the flat solver state.
//! * `Constraint` — trait replacing the C++ class (bodies referenced by index).
//! * `OptimizedNsvRigidBodySystem` — the NVECTOR/Baumgarte-style solver used by
//!   `PistonEngineSimulator` (`SystemType::NsvOptimized`).
//! * `GaussSeidelSleSolver` — the linear complementarity solver with limits.
//! * The concrete constraints the engine instantiates: fixed position, line,
//!   link, rotation friction, clutch.
//!
//! `GenericRigidBodySystem` / `GaussianEliminationSleSolver` / `NsvOdeSolver`
//! are *not* ported yet; the engine always constructs the optimized system with
//! the Gauss–Seidel solver, and `SystemType::Generic` is a debug path.

// ---------------------------------------------------------------- matrix

/// Dense matrix, row-major, indexed as `(column, row)` exactly like the C++
/// class (the original stores `m_matrix[row][column]`).
#[derive(Clone, Debug, Default)]
pub struct Matrix {
    data: Vec<f64>,
    width: usize,
    height: usize,
}

impl Matrix {
    pub fn new(width: usize, height: usize, value: f64) -> Self {
        Self {
            data: vec![value; width * height],
            width,
            height,
        }
    }

    pub fn initialize(&mut self, width: usize, height: usize, value: f64) {
        self.data.clear();
        self.data.resize(width * height, value);
        self.width = width;
        self.height = height;
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        if width == self.width && height == self.height {
            return;
        }
        self.initialize(width, height, 0.0);
    }

    #[inline]
    pub fn set(&mut self, column: usize, row: usize, value: f64) {
        self.data[row * self.width + column] = value;
    }

    #[inline]
    pub fn add(&mut self, column: usize, row: usize, value: f64) {
        self.data[row * self.width + column] += value;
    }

    #[inline]
    pub fn get(&self, column: usize, row: usize) -> f64 {
        self.data[row * self.width + column]
    }

    pub fn set_from(&mut self, reference: &Matrix) {
        self.data.clear();
        self.data.extend_from_slice(&reference.data);
        self.width = reference.width;
        self.height = reference.height;
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// `target = this * s`
    pub fn scale(&self, s: f64, target: &mut Matrix) {
        target.initialize(self.width, self.height, 0.0);
        for i in 0..self.data.len() {
            target.data[i] = self.data[i] * s;
        }
    }

    /// `target(column, row) = scale(0, row) * this(column, row)`
    pub fn left_scale(&self, scale: &Matrix, target: &mut Matrix) {
        target.initialize(self.width, self.height, 0.0);
        for row in 0..self.height {
            let s = scale.get(0, row);
            for column in 0..self.width {
                target.set(column, row, self.get(column, row) * s);
            }
        }
    }

    /// `target = this + b`
    pub fn add_matrix(&self, b: &Matrix, target: &mut Matrix) {
        target.initialize(self.width, self.height, 0.0);
        for i in 0..self.data.len() {
            target.data[i] = self.data[i] + b.data[i];
        }
    }

    /// `target = this * b` (row × column dot products, `(col,row)` indexing).
    pub fn multiply(&self, b: &Matrix, target: &mut Matrix) {
        target.initialize(b.width, self.height, 0.0);
        for row in 0..self.height {
            for column in 0..b.width {
                let mut sum = 0.0;
                for k in 0..self.width {
                    sum += self.get(k, row) * b.get(column, k);
                }
                target.set(column, row, sum);
            }
        }
    }

    /// `target = -this`
    pub fn negate(&self, target: &mut Matrix) {
        target.initialize(self.width, self.height, 0.0);
        for i in 0..self.data.len() {
            target.data[i] = -self.data[i];
        }
    }

    pub fn dot(&self, b: &Matrix) -> f64 {
        let mut sum = 0.0;
        for i in 0..self.data.len() {
            sum += self.data[i] * b.data[i];
        }
        sum
    }

    pub fn vector_magnitude_squared(&self) -> f64 {
        self.dot(self)
    }
}

// ---------------------------------------------------------- sparse matrix

/// Block-sparse matrix with `STRIDE`-wide blocks and `ENTRIES` blocks per row —
/// `SparseMatrix<3, 2>` from the original, which is the only instantiation the
/// solver uses.
pub const STRIDE: usize = 3;
pub const ENTRIES: usize = 2;

#[derive(Clone, Debug)]
pub struct SparseMatrix {
    data: Vec<f64>,
    blocks: Vec<u8>,
    width: usize,
    height: usize,
}

impl Default for SparseMatrix {
    fn default() -> Self {
        Self::new()
    }
}

impl SparseMatrix {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            blocks: Vec::new(),
            width: 0,
            height: 0,
        }
    }

    pub fn initialize(&mut self, width: usize, height: usize) {
        self.resize(width, height);
        self.blocks.clear();
        self.blocks.resize(height * ENTRIES, 0xFF);
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        if width == self.width && height == self.height {
            return;
        }
        self.data.clear();
        self.data.resize(height * ENTRIES * STRIDE, 0.0);
        self.blocks.clear();
        self.blocks.resize(height * ENTRIES, 0xFF);
        self.width = width;
        self.height = height;
    }

    #[inline]
    pub fn set_block(&mut self, row: usize, entry: usize, index: u8) {
        self.blocks[row * ENTRIES + entry] = index;
    }

    #[inline]
    pub fn set(&mut self, row: usize, entry: usize, slice: usize, v: f64) {
        self.data[row * ENTRIES * STRIDE + entry * STRIDE + slice] = v;
    }

    #[inline]
    pub fn get(&self, row: usize, entry: usize, slice: usize) -> f64 {
        self.data[row * ENTRIES * STRIDE + entry * STRIDE + slice]
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// `target = this * b`
    pub fn multiply(&self, b: &Matrix, target: &mut Matrix) {
        target.initialize(b.width(), self.height, 0.0);

        for i in 0..self.height {
            for j in 0..b.width() {
                let mut v = 0.0;
                for k in 0..ENTRIES {
                    let offset = k * STRIDE;
                    let block = self.blocks[i * ENTRIES + k];
                    if block == 0xFF {
                        continue;
                    }
                    let block = block as usize;
                    for l in 0..STRIDE {
                        v += self.data[i * ENTRIES * STRIDE + offset + l]
                            * b.get(j, block * STRIDE + l);
                    }
                }
                target.set(j, i, v);
            }
        }
    }

    /// `target = this * transpose(b_T)`
    pub fn multiply_transpose(&self, b_t: &SparseMatrix, target: &mut Matrix) {
        target.initialize(b_t.height, self.height, 0.0);

        for i in 0..self.height {
            for j in 0..b_t.height {
                let mut dot = 0.0;
                for k in 0..ENTRIES {
                    let block0 = self.blocks[i * ENTRIES + k];
                    if block0 == 0xFF {
                        continue;
                    }

                    for l in 0..ENTRIES {
                        let block1 = b_t.blocks[j * ENTRIES + l];
                        if block0 == block1 {
                            for m in 0..STRIDE {
                                dot += self.data[i * ENTRIES * STRIDE + k * STRIDE + m]
                                    * b_t.data[j * ENTRIES * STRIDE + l * STRIDE + m];
                            }
                        }
                    }
                }

                target.set(j, i, dot);
            }
        }
    }

    /// `target = this * diag(scale)` (row blocks scaled by `scale`)
    pub fn right_scale(&self, scale: &Matrix, target: &mut SparseMatrix) {
        target.initialize(self.width, self.height);

        for i in 0..self.height {
            for j in 0..ENTRIES {
                let index = self.blocks[i * ENTRIES + j];
                if index == 0xFF {
                    continue;
                }

                target.set_block(i, j, index);

                for k in 0..STRIDE {
                    target.set(
                        i,
                        j,
                        k,
                        scale.get(0, index as usize * STRIDE + k)
                            * self.get(i, j, k),
                    );
                }
            }
        }
    }

    /// `target = diag(scale) * this` (each row scaled by `scale(0, row)`)
    pub fn left_scale(&self, scale: &Matrix, target: &mut SparseMatrix) {
        target.initialize(self.width, self.height);

        for i in 0..self.height {
            for j in 0..ENTRIES {
                let index = self.blocks[i * ENTRIES + j];
                if index == 0xFF {
                    continue;
                }

                target.set_block(i, j, index);

                for k in 0..STRIDE {
                    target.set(i, j, k, scale.get(0, i) * self.get(i, j, k));
                }
            }
        }
    }
}

// ------------------------------------------------------------- rigid body

/// `atg_scs::RigidBody`
#[derive(Clone, Copy, Debug)]
pub struct RigidBody {
    pub p_x: f64,
    pub p_y: f64,
    pub v_x: f64,
    pub v_y: f64,
    pub theta: f64,
    pub v_theta: f64,
    pub m: f64,
    pub i: f64,
    pub index: i32,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            p_x: 0.0,
            p_y: 0.0,
            v_x: 0.0,
            v_y: 0.0,
            theta: 0.0,
            v_theta: 0.0,
            m: 0.0,
            i: 0.0,
            index: -1,
        }
    }
}

impl RigidBody {
    pub fn reset(&mut self) {
        self.p_x = 0.0;
        self.p_y = 0.0;
        self.v_x = 0.0;
        self.v_y = 0.0;
        self.theta = 0.0;
        self.v_theta = 0.0;
        self.m = 0.0;
        self.i = 0.0;
    }

    pub fn energy(&self) -> f64 {
        let speed_2 = self.v_x * self.v_x + self.v_y * self.v_y;
        let e_k = 0.5 * self.m * speed_2;
        let e_r = 0.5 * self.i * self.v_theta * self.v_theta;
        e_k + e_r
    }

    pub fn local_to_world(&self, x: f64, y: f64) -> (f64, f64) {
        let cos_theta = self.theta.cos();
        let sin_theta = self.theta.sin();
        (
            cos_theta * x - sin_theta * y + self.p_x,
            sin_theta * x + cos_theta * y + self.p_y,
        )
    }

    pub fn world_to_local(&self, x: f64, y: f64) -> (f64, f64) {
        let cos_theta = self.theta.cos();
        let sin_theta = self.theta.sin();
        (
            cos_theta * (x - self.p_x) + sin_theta * (y - self.p_y),
            -sin_theta * (x - self.p_x) + cos_theta * (y - self.p_y),
        )
    }
}

// ------------------------------------------------------------ system state

/// `atg_scs::SystemState` — flat arrays indexed by body id.
#[derive(Clone, Debug, Default)]
pub struct SystemState {
    pub index_map: Vec<i32>,

    pub a_theta: Vec<f64>,
    pub v_theta: Vec<f64>,
    pub theta: Vec<f64>,

    pub a_x: Vec<f64>,
    pub a_y: Vec<f64>,
    pub v_x: Vec<f64>,
    pub v_y: Vec<f64>,
    pub p_x: Vec<f64>,
    pub p_y: Vec<f64>,

    pub f_x: Vec<f64>,
    pub f_y: Vec<f64>,
    pub t: Vec<f64>,

    pub r_x: Vec<f64>,
    pub r_y: Vec<f64>,
    pub r_t: Vec<f64>,

    pub m: Vec<f64>,

    pub n: usize,
    pub n_c: usize,
    pub dt: f64,
}

impl SystemState {
    pub fn resize(&mut self, body_count: usize, constraint_count: usize) {
        self.n = body_count;
        self.n_c = constraint_count;

        let zero = |v: &mut Vec<f64>, len: usize| {
            v.clear();
            v.resize(len, 0.0);
        };

        zero(&mut self.a_theta, body_count);
        zero(&mut self.v_theta, body_count);
        zero(&mut self.theta, body_count);
        zero(&mut self.a_x, body_count);
        zero(&mut self.a_y, body_count);
        zero(&mut self.v_x, body_count);
        zero(&mut self.v_y, body_count);
        zero(&mut self.p_x, body_count);
        zero(&mut self.p_y, body_count);
        zero(&mut self.f_x, body_count);
        zero(&mut self.f_y, body_count);
        zero(&mut self.t, body_count);
        zero(&mut self.m, body_count);

        zero(&mut self.r_x, 2 * constraint_count);
        zero(&mut self.r_y, 2 * constraint_count);
        zero(&mut self.r_t, 2 * constraint_count);

        // `SystemState::indexMap` maps constraint index -> first equation index,
        // so it is sized by the constraint count (not the body count).
        self.index_map.clear();
        self.index_map.resize(constraint_count.max(1), 0);
    }

    pub fn local_to_world(&self, x: f64, y: f64, body: usize) -> (f64, f64) {
        let x0 = self.p_x[body];
        let y0 = self.p_y[body];
        let theta = self.theta[body];

        let cos_theta = theta.cos();
        let sin_theta = theta.sin();

        (cos_theta * x - sin_theta * y + x0, sin_theta * x + cos_theta * y + y0)
    }

    pub fn velocity_at_point(&self, x: f64, y: f64, body: usize) -> (f64, f64) {
        let (w_x, w_y) = self.local_to_world(x, y, body);

        let v_theta = self.v_theta[body];
        let angular_to_linear_x = -v_theta * (w_y - self.p_y[body]);
        let angular_to_linear_y = v_theta * (w_x - self.p_x[body]);

        (
            self.v_x[body] + angular_to_linear_x,
            self.v_y[body] + angular_to_linear_y,
        )
    }

    pub fn apply_force(&mut self, x_l: f64, y_l: f64, f_x: f64, f_y: f64, body: usize) {
        let (w_x, w_y) = self.local_to_world(x_l, y_l, body);

        self.f_x[body] += f_x;
        self.f_y[body] += f_y;

        self.t[body] += (w_y - self.p_y[body]) * -f_x + (w_x - self.p_x[body]) * f_y;
    }
}

// --------------------------------------------------------------- constraint

pub const MAX_CONSTRAINT_COUNT: usize = 3;
pub const MAX_BODY_COUNT: usize = 2;

/// `atg_scs::Constraint::Output`
#[derive(Clone, Copy, Debug)]
pub struct Output {
    pub c: [f64; MAX_CONSTRAINT_COUNT],
    pub j: [[f64; 3 * MAX_BODY_COUNT]; MAX_CONSTRAINT_COUNT],
    pub j_dot: [[f64; 3 * MAX_BODY_COUNT]; MAX_CONSTRAINT_COUNT],
    pub v_bias: [f64; MAX_CONSTRAINT_COUNT],
    pub limits: [[f64; 2]; MAX_CONSTRAINT_COUNT],
    pub ks: [f64; MAX_CONSTRAINT_COUNT],
    pub kd: [f64; MAX_CONSTRAINT_COUNT],
}

impl Default for Output {
    fn default() -> Self {
        Self {
            c: [0.0; MAX_CONSTRAINT_COUNT],
            j: [[0.0; 3 * MAX_BODY_COUNT]; MAX_CONSTRAINT_COUNT],
            j_dot: [[0.0; 3 * MAX_BODY_COUNT]; MAX_CONSTRAINT_COUNT],
            v_bias: [0.0; MAX_CONSTRAINT_COUNT],
            limits: [[-f64::MAX, f64::MAX]; MAX_CONSTRAINT_COUNT],
            ks: [0.0; MAX_CONSTRAINT_COUNT],
            kd: [0.0; MAX_CONSTRAINT_COUNT],
        }
    }
}

impl Output {
    pub fn no_limits(&mut self) {
        for i in 0..MAX_CONSTRAINT_COUNT {
            self.limits[i][0] = -f64::MAX;
            self.limits[i][1] = f64::MAX;
        }
    }
}

/// Solver-force accumulators (`Constraint::F_x` / `F_y` / `F_t`).
#[derive(Clone, Copy, Debug, Default)]
pub struct ConstraintForces {
    pub f_x: [[f64; MAX_BODY_COUNT]; MAX_CONSTRAINT_COUNT],
    pub f_y: [[f64; MAX_BODY_COUNT]; MAX_CONSTRAINT_COUNT],
    pub f_t: [[f64; MAX_BODY_COUNT]; MAX_CONSTRAINT_COUNT],
}

/// `atg_scs::Constraint`
pub trait Constraint {
    fn constraint_count(&self) -> usize;

    /// Body indices into [`RigidBodySystem::bodies`], `-1` for "unset".
    fn bodies(&self) -> [i32; MAX_BODY_COUNT];

    fn calculate(&mut self, output: &mut Output, state: &SystemState, bodies: &[RigidBody]);

    fn forces(&self) -> &ConstraintForces;
    fn forces_mut(&mut self) -> &mut ConstraintForces;

    /// Downcasting support so the simulator can reconfigure the dynamometer or
    /// starter motor after they have been boxed into the constraint list.
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// Torque reported to the UI/telemetry (only the dynamometer implements it,
    /// mirroring `Dynamometer::getTorque`).
    fn output_torque(&self, _bodies: &[RigidBody]) -> f64 {
        0.0
    }
}

/// Implements the `as_any` plumbing for a concrete constraint type.
macro_rules! constraint_any {
    ($t:ty) => {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    };
}

/// `atg_scs::ForceGenerator`
pub trait ForceGenerator {
    fn apply(&mut self, state: &mut SystemState);
}

// ------------------------------------------------------------------ solvers

/// `atg_scs::GaussSeidelSleSolver`
pub struct GaussSeidelSleSolver {
    pub max_iterations: usize,
    pub min_delta: f64,
}

impl Default for GaussSeidelSleSolver {
    fn default() -> Self {
        Self {
            max_iterations: 128,
            min_delta: 1e-1,
        }
    }
}

impl GaussSeidelSleSolver {
    pub fn solve_with_limits(
        &self,
        j: &SparseMatrix,
        w: &Matrix,
        right: &Matrix,
        limits: &Matrix,
        result: &mut Matrix,
    ) -> bool {
        let n = right.height();
        if result.height() != n {
            result.initialize(1, n, 0.0);
        }

        let mut reg = SparseMatrix::new();
        let mut m = Matrix::default();

        j.right_scale(w, &mut reg);
        reg.multiply_transpose(j, &mut m);

        for _ in 0..self.max_iterations {
            if self.solve_iteration(&m, right, limits, result) {
                return true;
            }
        }

        false
    }

    fn solve_iteration(
        &self,
        left: &Matrix,
        right: &Matrix,
        limits: &Matrix,
        k: &mut Matrix,
    ) -> bool {
        let mut max_difference = 0.0;
        let n = k.height();

        for i in 0..n {
            let mut s0 = 0.0;
            let mut s1 = 0.0;
            for j in 0..i {
                s0 += left.get(j, i) * k.get(0, j);
            }
            for j in (i + 1)..n {
                s1 += left.get(j, i) * k.get(0, j);
            }

            let k_next_i = (1.0 / left.get(i, i)) * (right.get(0, i) - s0 - s1);

            let limit_min = limits.get(0, i);
            let limit_max = limits.get(1, i);
            let x = limit_min.max(limit_max.min(k_next_i));

            let min_k = 1e-3_f64.max(k.get(0, i).abs());
            let delta = (x - k.get(0, i)).abs() / min_k;
            max_difference = if delta > max_difference { delta } else { max_difference };

            k.set(0, i, x);
        }

        max_difference < self.min_delta
    }
}

// ------------------------------------------------------------------- system

/// `atg_scs::RigidBodySystem` + `OptimizedNsvRigidBodySystem`.
pub struct RigidBodySystem {
    pub bodies: Vec<RigidBody>,
    pub constraints: Vec<Box<dyn Constraint>>,
    pub state: SystemState,

    sle_solver: GaussSeidelSleSolver,
    pub bias_factor: f64,
    pub t: f64,

    // intermediate values (mirrors `IntermediateValues`)
    j_sparse: SparseMatrix,
    sreg0: SparseMatrix,
    c: Matrix,
    m: Matrix,
    m_inv: Matrix,
    b_err: Matrix,
    v_bias: Matrix,
    limits: Matrix,
    q_dot: Matrix,
    q_dot_prime: Matrix,
    reg0: Matrix,
    reg1: Matrix,
    right: Matrix,
    f_ext: Matrix,
    lambda: Matrix,
}

impl Default for RigidBodySystem {
    fn default() -> Self {
        Self::new()
    }
}

impl RigidBodySystem {
    pub fn new() -> Self {
        Self {
            bodies: Vec::new(),
            constraints: Vec::new(),
            state: SystemState::default(),
            sle_solver: GaussSeidelSleSolver::default(),
            bias_factor: 1.0,
            t: 0.0,
            j_sparse: SparseMatrix::new(),
            sreg0: SparseMatrix::new(),
            c: Matrix::default(),
            m: Matrix::default(),
            m_inv: Matrix::default(),
            b_err: Matrix::default(),
            v_bias: Matrix::default(),
            limits: Matrix::default(),
            q_dot: Matrix::default(),
            q_dot_prime: Matrix::default(),
            reg0: Matrix::default(),
            reg1: Matrix::default(),
            right: Matrix::default(),
            f_ext: Matrix::default(),
            lambda: Matrix::default(),
        }
    }

    pub fn reset(&mut self) {
        self.bodies.clear();
        self.constraints.clear();
        self.state = SystemState::default();
    }

    pub fn add_rigid_body(&mut self, mut body: RigidBody) -> usize {
        let index = self.bodies.len();
        body.index = index as i32;
        self.bodies.push(body);
        index
    }

    pub fn add_constraint(&mut self, constraint: Box<dyn Constraint>) {
        self.constraints.push(constraint);
    }

    pub fn full_constraint_count(&self) -> usize {
        self.constraints.iter().map(|c| c.constraint_count()).sum()
    }

    /// Copy body state into the flat [`SystemState`].
    pub fn populate_system_state(&mut self) {
        let n = self.bodies.len();
        let n_c = self.full_constraint_count();

        self.state.resize(n, n_c);

        for i in 0..n {
            let b = &self.bodies[i];
            self.state.a_x[i] = 0.0;
            self.state.a_y[i] = 0.0;

            self.state.v_x[i] = b.v_x;
            self.state.v_y[i] = b.v_y;

            self.state.p_x[i] = b.p_x;
            self.state.p_y[i] = b.p_y;

            self.state.a_theta[i] = 0.0;
            self.state.v_theta[i] = b.v_theta;
            self.state.theta[i] = b.theta;

            self.state.m[i] = b.m;
        }

        let mut j_f = 0;
        for i in 0..self.constraints.len() {
            self.state.index_map[i] = j_f as i32;
            j_f += self.constraints[i].constraint_count();
        }
    }

    fn populate_mass_matrices(&mut self) {
        let n = self.bodies.len();
        self.m.initialize(1, 3 * n, 0.0);
        self.m_inv.initialize(1, 3 * n, 0.0);

        for i in 0..n {
            let b = &self.bodies[i];
            self.m.set(0, i * 3 + 0, b.m);
            self.m.set(0, i * 3 + 1, b.m);
            self.m.set(0, i * 3 + 2, b.i);

            self.m_inv.set(0, i * 3 + 0, 1.0 / b.m);
            self.m_inv.set(0, i * 3 + 1, 1.0 / b.m);
            self.m_inv.set(0, i * 3 + 2, 1.0 / b.i);
        }
    }

    /// Advance the simulation by `steps` sub-steps of `dt / steps`.
    ///
    /// `apply_forces` plays the role of the C++ `ForceGenerator` list: the
    /// engine's combustion chambers live inside [`crate::engine::Engine`], so
    /// the caller applies them rather than the system owning them.
    pub fn process(&mut self, dt: f64, steps: usize, apply_forces: &mut dyn FnMut(&mut SystemState)) {
        self.populate_system_state();
        self.populate_mass_matrices();

        for _ in 0..steps {
            let sub_dt = dt / steps as f64;

            // NsvOdeSolver: `step()` always reports "done", so each sub-step is
            // exactly one force evaluation, one constraint solve, one integrate.
            self.state.dt = sub_dt;

            let body_count = self.state.n;
            for i in 0..body_count {
                self.state.f_x[i] = 0.0;
                self.state.f_y[i] = 0.0;
                self.state.t[i] = 0.0;
            }
            apply_forces(&mut self.state);

            self.process_constraints(sub_dt);

            // NsvOdeSolver::solve
            let n = self.state.n;
            for i in 0..n {
                self.state.v_x[i] += self.state.a_x[i] * sub_dt;
                self.state.v_y[i] += self.state.a_y[i] * sub_dt;
                self.state.v_theta[i] += self.state.a_theta[i] * sub_dt;

                self.state.p_x[i] += self.state.v_x[i] * sub_dt;
                self.state.p_y[i] += self.state.v_y[i] * sub_dt;
                self.state.theta[i] += self.state.v_theta[i] * sub_dt;
            }
        }

        self.propagate_results();
        self.t += dt;
    }

    fn propagate_results(&mut self) {
        let n = self.bodies.len();
        for i in 0..n {
            let (v_x, v_y, p_x, p_y, v_theta, theta) = (
                self.state.v_x[i],
                self.state.v_y[i],
                self.state.p_x[i],
                self.state.p_y[i],
                self.state.v_theta[i],
                self.state.theta[i],
            );
            let b = &mut self.bodies[i];
            b.v_x = v_x;
            b.v_y = v_y;
            b.p_x = p_x;
            b.p_y = p_y;
            b.v_theta = v_theta;
            b.theta = theta;
        }

        let mut i_f = 0;
        for constraint in self.constraints.iter_mut() {
            let n_f = constraint.constraint_count();
            let body_count = constraint.bodies().len().min(MAX_BODY_COUNT);
            let forces = constraint.forces_mut();

            for j in 0..n_f {
                for k in 0..body_count {
                    forces.f_x[j][k] = self.state.r_x[i_f * 2 + k];
                    forces.f_y[j][k] = self.state.r_y[i_f * 2 + k];
                    forces.f_t[j][k] = self.state.r_t[i_f * 2 + k];
                }
                i_f += 1;
            }
        }
    }

    fn process_constraints(&mut self, dt: f64) {
        let n = self.bodies.len();
        let m_f = self.full_constraint_count();
        let m = self.constraints.len();

        self.j_sparse.initialize(3 * n, m_f);
        self.v_bias.initialize(1, m_f, 0.0);
        self.c.initialize(1, m_f, 0.0);
        self.limits.initialize(2, m_f, 0.0);

        let mut output = Output::default();

        let mut j_f = 0;
        for j in 0..m {
            let bodies = self.constraints[j].bodies();
            let n_f = self.constraints[j].constraint_count();

            // Constraints read both the solver state and body state, exactly as
            // the C++ code does through its body pointers.
            {
                let state = &self.state;
                let bodies = &self.bodies;
                self.constraints[j].calculate(&mut output, state, bodies);
            }

            for k in 0..n_f {
                for i in 0..bodies.len().min(MAX_BODY_COUNT) {
                    let index = bodies[i];
                    if index == -1 {
                        continue;
                    }
                    self.j_sparse.set_block(j_f, i, index as u8);
                }

                for i in 0..(bodies.len().min(MAX_BODY_COUNT) * 3) {
                    let index = bodies[i / 3];
                    if index == -1 {
                        continue;
                    }
                    self.j_sparse.set(j_f, i / 3, i % 3, output.j[k][i]);
                }

                self.v_bias.set(0, j_f, output.v_bias[k]);
                self.c.set(0, j_f, output.c[k]);
                self.limits.set(0, j_f, output.limits[k][0] * dt);
                self.limits.set(1, j_f, output.limits[k][1] * dt);

                j_f += 1;
            }
        }

        self.q_dot.resize(1, n * 3);
        for i in 0..n {
            self.q_dot.set(0, i * 3 + 0, self.state.v_x[i]);
            self.q_dot.set(0, i * 3 + 1, self.state.v_y[i]);
            self.q_dot.set(0, i * 3 + 2, self.state.v_theta[i]);
        }

        self.f_ext.initialize(1, 3 * n, 0.0);
        for i in 0..n {
            self.f_ext.set(0, i * 3 + 0, self.state.f_x[i]);
            self.f_ext.set(0, i * 3 + 1, self.state.f_y[i]);
            self.f_ext.set(0, i * 3 + 2, self.state.t[i]);
        }

        // q_dot_prime = q_dot + M_inv * F_ext * dt
        self.f_ext.scale(dt, &mut self.reg0);
        self.reg0.left_scale(&self.m_inv, &mut self.reg1);
        self.reg1.add_matrix(&self.q_dot, &mut self.q_dot_prime);

        // b_err = (bias_factor / dt) * C
        self.c.scale(self.bias_factor / dt, &mut self.b_err);

        // right = -(J * q_dot_prime + v_bias + b_err)
        self.j_sparse.multiply(&self.q_dot_prime, &mut self.reg0);
        self.reg0.add_matrix(&self.v_bias, &mut self.reg1);
        self.reg1.add_matrix(&self.b_err, &mut self.reg0);
        self.reg0.negate(&mut self.right);

        let solvable = self.sle_solver.solve_with_limits(
            &self.j_sparse,
            &self.m_inv,
            &self.right,
            &self.limits,
            &mut self.lambda,
        );
        debug_assert!(solvable);

        // R = transpose(J * diag(lambda / dt))
        self.lambda.scale(1.0 / dt, &mut self.reg0);
        self.j_sparse.left_scale(&self.reg0, &mut self.sreg0);

        for i in 0..m_f {
            for j in 0..2 {
                self.state.r_x[i * 2 + j] = self.sreg0.get(i, j, 0);
                self.state.r_y[i * 2 + j] = self.sreg0.get(i, j, 1);
                self.state.r_t[i * 2 + j] = self.sreg0.get(i, j, 2);
            }
        }

        for i in 0..n {
            self.state.a_x[i] = self.f_ext.get(0, i * 3 + 0);
            self.state.a_y[i] = self.f_ext.get(0, i * 3 + 1);
            self.state.a_theta[i] = self.f_ext.get(0, i * 3 + 2);
        }

        let mut j_f = 0;
        for i in 0..m {
            let bodies = self.constraints[i].bodies();
            let n_f = self.constraints[i].constraint_count();

            for _j in 0..n_f {
                for k in 0..bodies.len().min(MAX_BODY_COUNT) {
                    let body = bodies[k];
                    if body == -1 {
                        continue;
                    }
                    let body = body as usize;
                    self.state.a_x[body] += self.state.r_x[j_f * 2 + k];
                    self.state.a_y[body] += self.state.r_y[j_f * 2 + k];
                    self.state.a_theta[body] += self.state.r_t[j_f * 2 + k];
                }
                j_f += 1;
            }
        }

        for i in 0..n {
            let inv_mass = self.m_inv.get(0, i * 3 + 0);
            let inv_inertia = self.m_inv.get(0, i * 3 + 2);

            self.state.a_x[i] *= inv_mass;
            self.state.a_y[i] *= inv_mass;
            self.state.a_theta[i] *= inv_inertia;
        }
    }
}

// ------------------------------------------------------------- constraints

/// `atg_scs::FixedPositionConstraint`
#[derive(Clone, Debug)]
pub struct FixedPositionConstraint {
    pub body: i32,
    pub local_x: f64,
    pub local_y: f64,
    pub world_x: f64,
    pub world_y: f64,
    pub ks: f64,
    pub kd: f64,
    pub forces: ConstraintForces,
}

impl Default for FixedPositionConstraint {
    fn default() -> Self {
        Self {
            body: -1,
            local_x: 0.0,
            local_y: 0.0,
            world_x: 0.0,
            world_y: 0.0,
            ks: 10.0,
            kd: 1.0,
            forces: ConstraintForces::default(),
        }
    }
}

impl FixedPositionConstraint {
    pub fn set_body(&mut self, body: i32) {
        self.body = body;
    }

    pub fn set_world_position(&mut self, x: f64, y: f64) {
        self.world_x = x;
        self.world_y = y;
    }

    pub fn set_local_position(&mut self, x: f64, y: f64) {
        self.local_x = x;
        self.local_y = y;
    }
}

impl Constraint for FixedPositionConstraint {
    fn constraint_count(&self) -> usize {
        2
    }

    fn bodies(&self) -> [i32; MAX_BODY_COUNT] {
        [self.body, -1]
    }

    fn calculate(&mut self, output: &mut Output, state: &SystemState, _bodies: &[RigidBody]) {
        let body = self.body as usize;

        let q1 = state.p_x[body];
        let q2 = state.p_y[body];
        let q3 = state.theta[body];
        let q3_dot = state.v_theta[body];

        let cos_q3 = q3.cos();
        let sin_q3 = q3.sin();

        let current_x = q1 + cos_q3 * self.local_x - sin_q3 * self.local_y;
        let current_y = q2 + sin_q3 * self.local_x + cos_q3 * self.local_y;

        let dx_dq1 = 1.0;
        let dx_dq2 = 0.0;
        let dx_dq3 = -sin_q3 * self.local_x - cos_q3 * self.local_y;

        let dy_dq1 = 0.0;
        let dy_dq2 = 1.0;
        let dy_dq3 = cos_q3 * self.local_x - sin_q3 * self.local_y;

        let c1 = current_x - self.world_x;
        let c2 = current_y - self.world_y;

        output.j[0][0] = dx_dq1;
        output.j[0][1] = dx_dq2;
        output.j[0][2] = dx_dq3;

        output.j[1][0] = dy_dq1;
        output.j[1][1] = dy_dq2;
        output.j[1][2] = dy_dq3;

        output.j_dot[0][0] = 0.0;
        output.j_dot[0][1] = 0.0;
        output.j_dot[0][2] = -cos_q3 * q3_dot * self.local_x + sin_q3 * q3_dot * self.local_y;

        output.j_dot[1][0] = 0.0;
        output.j_dot[1][1] = 0.0;
        output.j_dot[1][2] = -sin_q3 * q3_dot * self.local_x - cos_q3 * q3_dot * self.local_y;

        output.ks[0] = self.ks;
        output.ks[1] = self.ks;

        output.kd[0] = self.kd;
        output.kd[1] = self.kd;

        output.c[0] = c1;
        output.c[1] = c2;

        output.v_bias[0] = 0.0;
        output.v_bias[1] = 0.0;

        output.no_limits();
    }

    fn forces(&self) -> &ConstraintForces {
        &self.forces
    }

    fn forces_mut(&mut self) -> &mut ConstraintForces {
        &mut self.forces
    }

    constraint_any!(FixedPositionConstraint);
}

/// `atg_scs::LineConstraint`
#[derive(Clone, Debug)]
pub struct LineConstraint {
    pub body: i32,
    pub local_x: f64,
    pub local_y: f64,
    pub p0_x: f64,
    pub p0_y: f64,
    pub dx: f64,
    pub dy: f64,
    pub ks: f64,
    pub kd: f64,
    pub forces: ConstraintForces,
}

impl Default for LineConstraint {
    fn default() -> Self {
        Self {
            body: -1,
            local_x: 0.0,
            local_y: 0.0,
            p0_x: 0.0,
            p0_y: 0.0,
            dx: 0.0,
            dy: 0.0,
            ks: 10.0,
            kd: 1.0,
            forces: ConstraintForces::default(),
        }
    }
}

impl LineConstraint {
    pub fn set_body(&mut self, body: i32) {
        self.body = body;
    }
}

impl Constraint for LineConstraint {
    fn constraint_count(&self) -> usize {
        1
    }

    fn bodies(&self) -> [i32; MAX_BODY_COUNT] {
        [self.body, -1]
    }

    fn calculate(&mut self, output: &mut Output, state: &SystemState, _bodies: &[RigidBody]) {
        let body = self.body as usize;

        let q1 = state.p_x[body];
        let q2 = state.p_y[body];
        let q3 = state.theta[body];
        let q3_dot = state.v_theta[body];

        let cos_q3 = q3.cos();
        let sin_q3 = q3.sin();

        let body_x = q1 + cos_q3 * self.local_x - sin_q3 * self.local_y;
        let body_y = q2 + sin_q3 * self.local_x + cos_q3 * self.local_y;

        let perp_x = -self.dy;
        let perp_y = self.dx;

        let delta_x = body_x - self.p0_x;
        let delta_y = body_y - self.p0_y;

        let c = delta_x * perp_x + delta_y * perp_y;

        let dc_dq1 = 1.0 * perp_x;
        let dc_dq2 = 1.0 * perp_y;
        let dc_dq3 = (-sin_q3 * self.local_x - cos_q3 * self.local_y) * perp_x
            + (cos_q3 * self.local_x - sin_q3 * self.local_y) * perp_y;

        output.j[0][0] = dc_dq1;
        output.j[0][1] = dc_dq2;
        output.j[0][2] = dc_dq3;

        output.j_dot[0][0] = 0.0;
        output.j_dot[0][1] = 0.0;
        output.j_dot[0][2] = (-cos_q3 * q3_dot * self.local_x + sin_q3 * q3_dot * self.local_y)
            * perp_x
            + (-sin_q3 * q3_dot * self.local_x - cos_q3 * q3_dot * self.local_y) * perp_y;

        output.ks[0] = self.ks;
        output.kd[0] = self.kd;

        output.c[0] = c;
        output.v_bias[0] = 0.0;

        output.no_limits();
    }

    fn forces(&self) -> &ConstraintForces {
        &self.forces
    }

    fn forces_mut(&mut self) -> &mut ConstraintForces {
        &mut self.forces
    }

    constraint_any!(LineConstraint);
}

/// `atg_scs::LinkConstraint`
#[derive(Clone, Debug)]
pub struct LinkConstraint {
    pub body1: i32,
    pub body2: i32,
    pub local_x_1: f64,
    pub local_y_1: f64,
    pub local_x_2: f64,
    pub local_y_2: f64,
    pub ks: f64,
    pub kd: f64,
    pub max_force: f64,
    pub forces: ConstraintForces,
}

impl Default for LinkConstraint {
    fn default() -> Self {
        Self {
            body1: -1,
            body2: -1,
            local_x_1: 0.0,
            local_y_1: 0.0,
            local_x_2: 0.0,
            local_y_2: 0.0,
            ks: 10.0,
            kd: 1.0,
            max_force: f64::MAX,
            forces: ConstraintForces::default(),
        }
    }
}

impl LinkConstraint {
    pub fn set_body1(&mut self, body: i32) {
        self.body1 = body;
    }

    pub fn set_body2(&mut self, body: i32) {
        self.body2 = body;
    }

    pub fn set_local_position1(&mut self, x: f64, y: f64) {
        self.local_x_1 = x;
        self.local_y_1 = y;
    }

    pub fn set_local_position2(&mut self, x: f64, y: f64) {
        self.local_x_2 = x;
        self.local_y_2 = y;
    }
}

impl Constraint for LinkConstraint {
    fn constraint_count(&self) -> usize {
        2
    }

    fn bodies(&self) -> [i32; MAX_BODY_COUNT] {
        [self.body1, self.body2]
    }

    fn calculate(&mut self, output: &mut Output, state: &SystemState, _bodies: &[RigidBody]) {
        let body = self.body1 as usize;
        let linked_body = self.body2 as usize;

        let q1 = state.p_x[body];
        let q2 = state.p_y[body];
        let q3 = state.theta[body];

        let q4 = state.p_x[linked_body];
        let q5 = state.p_y[linked_body];
        let q6 = state.theta[linked_body];

        let q3_dot = state.v_theta[body];
        let q6_dot = state.v_theta[linked_body];

        let cos_q3 = q3.cos();
        let sin_q3 = q3.sin();

        let cos_q6 = q6.cos();
        let sin_q6 = q6.sin();

        let body_x = q1 + cos_q3 * self.local_x_1 - sin_q3 * self.local_y_1;
        let body_y = q2 + sin_q3 * self.local_x_1 + cos_q3 * self.local_y_1;

        let linked_body_x = q4 + cos_q6 * self.local_x_2 - sin_q6 * self.local_y_2;
        let linked_body_y = q5 + sin_q6 * self.local_x_2 + cos_q6 * self.local_y_2;

        let c1 = body_x - linked_body_x;
        let c2 = body_y - linked_body_y;

        output.j[0][0] = 1.0;
        output.j[0][1] = 0.0;
        output.j[0][2] = -sin_q3 * self.local_x_1 - cos_q3 * self.local_y_1;

        output.j[1][0] = 0.0;
        output.j[1][1] = 1.0;
        output.j[1][2] = cos_q3 * self.local_x_1 - sin_q3 * self.local_y_1;

        output.j[0][3] = -1.0;
        output.j[0][4] = 0.0;
        output.j[0][5] = sin_q6 * self.local_x_2 + cos_q6 * self.local_y_2;

        output.j[1][3] = 0.0;
        output.j[1][4] = -1.0;
        output.j[1][5] = -cos_q6 * self.local_x_2 + sin_q6 * self.local_y_2;

        output.j_dot[0][0] = 0.0;
        output.j_dot[0][1] = 0.0;
        output.j_dot[0][2] =
            -cos_q3 * q3_dot * self.local_x_1 + sin_q3 * q3_dot * self.local_y_1;

        output.j_dot[1][0] = 0.0;
        output.j_dot[1][1] = 0.0;
        output.j_dot[1][2] =
            -sin_q3 * q3_dot * self.local_x_1 - cos_q3 * q3_dot * self.local_y_1;

        output.j_dot[0][3] = 0.0;
        output.j_dot[0][4] = 0.0;
        output.j_dot[0][5] =
            cos_q6 * q6_dot * self.local_x_2 - sin_q6 * q6_dot * self.local_y_2;

        output.j_dot[1][3] = 0.0;
        output.j_dot[1][4] = 0.0;
        output.j_dot[1][5] =
            sin_q6 * q6_dot * self.local_x_2 + cos_q6 * q6_dot * self.local_y_2;

        output.kd[0] = self.kd;
        output.kd[1] = self.kd;
        output.ks[0] = self.ks;
        output.ks[1] = self.ks;

        output.c[0] = c1;
        output.c[1] = c2;

        output.v_bias[0] = 0.0;
        output.v_bias[1] = 0.0;

        output.limits[0][0] = -self.max_force;
        output.limits[0][1] = self.max_force;

        output.limits[1][0] = -self.max_force;
        output.limits[1][1] = self.max_force;
    }

    fn forces(&self) -> &ConstraintForces {
        &self.forces
    }

    fn forces_mut(&mut self) -> &mut ConstraintForces {
        &mut self.forces
    }

    constraint_any!(LinkConstraint);
}

/// `atg_scs::RotationFrictionConstraint`
#[derive(Clone, Debug)]
pub struct RotationFrictionConstraint {
    pub body: i32,
    pub ks: f64,
    pub kd: f64,
    pub max_torque: f64,
    pub min_torque: f64,
    pub forces: ConstraintForces,
}

impl Default for RotationFrictionConstraint {
    fn default() -> Self {
        Self {
            body: -1,
            ks: 10.0,
            kd: 1.0,
            max_torque: f64::MAX,
            min_torque: -f64::MAX,
            forces: ConstraintForces::default(),
        }
    }
}

impl RotationFrictionConstraint {
    pub fn set_body(&mut self, body: i32) {
        self.body = body;
    }
}

impl Constraint for RotationFrictionConstraint {
    fn constraint_count(&self) -> usize {
        1
    }

    fn bodies(&self) -> [i32; MAX_BODY_COUNT] {
        [self.body, -1]
    }

    fn calculate(&mut self, output: &mut Output, _state: &SystemState, _bodies: &[RigidBody]) {
        output.c[0] = 0.0;

        output.j[0][0] = 0.0;
        output.j[0][1] = 0.0;
        output.j[0][2] = 1.0;

        output.j_dot[0][0] = 0.0;
        output.j_dot[0][1] = 0.0;
        output.j_dot[0][2] = 0.0;

        output.kd[0] = self.kd;
        output.ks[0] = self.ks;

        output.v_bias[0] = 0.0;

        output.limits[0][0] = self.min_torque;
        output.limits[0][1] = self.max_torque;
    }

    fn forces(&self) -> &ConstraintForces {
        &self.forces
    }

    fn forces_mut(&mut self) -> &mut ConstraintForces {
        &mut self.forces
    }

    constraint_any!(RotationFrictionConstraint);
}

/// `atg_scs::ClutchConstraint`
#[derive(Clone, Debug, Default)]
pub struct ClutchConstraint {
    pub body1: i32,
    pub body2: i32,
    pub ks: f64,
    pub kd: f64,
    pub max_torque: f64,
    pub min_torque: f64,
    pub forces: ConstraintForces,
}

impl ClutchConstraint {
    pub fn new() -> Self {
        Self {
            body1: -1,
            body2: -1,
            ks: 10.0,
            kd: 1.0,
            max_torque: f64::MAX,
            min_torque: -f64::MAX,
            forces: ConstraintForces::default(),
        }
    }

    pub fn set_body1(&mut self, body: i32) {
        self.body1 = body;
    }

    pub fn set_body2(&mut self, body: i32) {
        self.body2 = body;
    }
}

impl Constraint for ClutchConstraint {
    fn constraint_count(&self) -> usize {
        1
    }

    fn bodies(&self) -> [i32; MAX_BODY_COUNT] {
        [self.body1, self.body2]
    }

    fn calculate(&mut self, output: &mut Output, _state: &SystemState, _bodies: &[RigidBody]) {
        output.c[0] = 0.0;

        output.j[0][0] = 0.0;
        output.j[0][1] = 0.0;
        output.j[0][2] = -1.0;

        output.j[0][3] = 0.0;
        output.j[0][4] = 0.0;
        output.j[0][5] = 1.0;

        output.j_dot[0][0] = 0.0;
        output.j_dot[0][1] = 0.0;
        output.j_dot[0][2] = 0.0;

        output.j_dot[0][3] = 0.0;
        output.j_dot[0][4] = 0.0;
        output.j_dot[0][5] = 0.0;

        output.kd[0] = self.kd;
        output.ks[0] = self.ks;

        output.v_bias[0] = 0.0;

        output.limits[0][0] = self.min_torque;
        output.limits[0][1] = self.max_torque;
    }

    fn forces(&self) -> &ConstraintForces {
        &self.forces
    }

    fn forces_mut(&mut self) -> &mut ConstraintForces {
        &mut self.forces
    }

    constraint_any!(ClutchConstraint);
}
