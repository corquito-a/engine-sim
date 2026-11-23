//! Port of `include/function.h`, `include/gaussian_filter.h` and
//! `src/function.cpp` / `src/gaussian_filter.cpp`.
//!
//! `Function` is the engine-sim "curve" primitive: a set of samples with a
//! radius-limited weighting kernel (`sampleTriangle`) or a Gaussian kernel
//! (`sampleGaussian`). Every table in the engine (lobe profiles, port flow
//! benches, timing curves, turbulence curves) is one of these.

use std::sync::{Arc, OnceLock};

// ------------------------------------------------------------- gaussian

/// `GaussianFilter`
#[derive(Clone, Debug)]
pub struct GaussianFilter {
    cache: Vec<f64>,
    cache_steps: i32,
    radius: f64,
    alpha: f64,
    exp_s: f64,
    inv_r: f64,
}

impl GaussianFilter {
    /// `GaussianFilter::initialize(alpha, radius, cacheSteps)`
    pub fn new(alpha: f64, radius: f64, cache_steps: i32) -> Self {
        let mut filter = Self {
            cache: Vec::new(),
            cache_steps,
            radius,
            alpha,
            exp_s: (-alpha * radius * radius).exp(),
            inv_r: 1.0 / radius,
        };
        filter.generate_cache();
        filter
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }

    pub fn alpha(&self) -> f64 {
        self.alpha
    }

    /// `GaussianFilter::evaluate`
    pub fn evaluate(&self, s: f64) -> f64 {
        let actual_steps = self.cache_steps - 32;
        let s_sample = actual_steps as f64 * s.abs() * self.inv_r;
        let s0 = s_sample.floor();
        let s1 = s_sample.ceil();
        let d = s_sample - s0;

        (1.0 - d) * self.cache[s0 as usize] + d * self.cache[s1 as usize]
    }

    /// `GaussianFilter::calculate`
    fn calculate(&self, s: f64) -> f64 {
        0.0_f64.max((-self.alpha * s * s).exp() - self.exp_s)
    }

    fn generate_cache(&mut self) {
        let actual_steps = self.cache_steps - 32;
        let step = 1.0 / actual_steps as f64;

        self.cache.clear();
        self.cache.resize(self.cache_steps as usize, 0.0);

        for i in 0..=actual_steps {
            let s = i as f64 * step * self.radius;
            self.cache[i as usize] = self.calculate(s);
        }

        // The tail beyond `actualSteps` stays zero (the C++ code writes zeros).
    }
}

static DEFAULT_GAUSSIAN_FILTER: OnceLock<Arc<GaussianFilter>> = OnceLock::new();

/// `Function::DefaultGaussianFilter` — `initialize(1.0, 3.0, 1024)`.
pub fn default_gaussian_filter() -> Arc<GaussianFilter> {
    DEFAULT_GAUSSIAN_FILTER
        .get_or_init(|| Arc::new(GaussianFilter::new(1.0, 3.0, 1024)))
        .clone()
}

// --------------------------------------------------------------- function

/// `Function`
#[derive(Clone, Debug)]
pub struct Function {
    x: Vec<f64>,
    y: Vec<f64>,

    y_min: f64,
    y_max: f64,
    input_scale: f64,
    output_scale: f64,

    filter_radius: f64,
    gaussian_filter: Arc<GaussianFilter>,
}

impl Default for Function {
    fn default() -> Self {
        Self {
            x: Vec::new(),
            y: Vec::new(),
            y_min: 0.0,
            y_max: 0.0,
            input_scale: 1.0,
            output_scale: 1.0,
            filter_radius: 0.0,
            gaussian_filter: default_gaussian_filter(),
        }
    }
}

impl Function {
    /// `Function::initialize(size, filterRadius, filter)`
    pub fn new(capacity: usize, filter_radius: f64) -> Self {
        let mut f = Function::default();
        f.x.reserve(capacity);
        f.y.reserve(capacity);
        f.filter_radius = filter_radius;
        f
    }

    pub fn with_filter(
        capacity: usize,
        filter_radius: f64,
        filter: Arc<GaussianFilter>,
    ) -> Self {
        let mut f = Self::new(capacity, filter_radius);
        f.gaussian_filter = filter;
        f
    }

    pub fn set_input_scale(&mut self, s: f64) {
        self.input_scale = s;
    }

    pub fn set_output_scale(&mut self, s: f64) {
        self.output_scale = s;
    }

    /// `Function::setFilterRadius`
    pub fn set_filter_radius(&mut self, radius: f64) {
        self.filter_radius = radius;
    }

    pub fn filter_radius(&self) -> f64 {
        self.filter_radius
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// `Function::addSample`
    pub fn add_sample(&mut self, x: f64, y: f64) {
        self.y_min = self.y_min.min(y);
        self.y_max = self.y_max.max(y);

        let closest = self.closest_sample(x);
        if closest == -1 {
            self.x.clear();
            self.y.clear();
            self.x.push(x);
            self.y.push(y);
            return;
        }

        let closest = closest as usize;
        let index = if x < self.x[closest] {
            closest
        } else {
            closest + 1
        };

        self.x.insert(index, x);
        self.y.insert(index, y);
    }

    /// `Function::sampleTriangle`
    pub fn sample_triangle(&self, x: f64) -> f64 {
        let x = x * self.input_scale;
        let closest = self.closest_sample(x);

        if self.x.is_empty() {
            return 0.0;
        } else if x >= self.x[self.x.len() - 1] {
            return self.y[self.y.len() - 1] * self.output_scale;
        } else if x <= self.x[0] {
            return self.y[0] * self.output_scale;
        }

        let mut sum = 0.0;
        let mut total_weight = 0.0;

        let mut i = closest;
        while i >= 0 {
            let iu = i as usize;
            if !(self.x[iu] > x) {
                if (x - self.x[iu]).abs() > self.filter_radius {
                    break;
                }

                let w = self.triangle(self.x[iu] - x);
                sum += w * self.y[iu];
                total_weight += w;
            }
            i -= 1;
        }

        let mut i = closest.max(0) as usize;
        while i < self.x.len() {
            if self.x[i] > x {
                if (self.x[i] - x).abs() > self.filter_radius {
                    break;
                }

                let w = self.triangle(self.x[i] - x);
                sum += w * self.y[i];
                total_weight += w;
            }
            i += 1;
        }

        if total_weight != 0.0 {
            sum * self.output_scale / total_weight
        } else {
            0.0
        }
    }

    /// `Function::sampleGaussian`
    pub fn sample_gaussian(&self, x: f64) -> f64 {
        let x = x * self.input_scale;
        let closest = self.closest_sample(x);
        let filter_radius = self.filter_radius * self.gaussian_filter.radius();

        let mut sum = 0.0;
        let mut total_weight = 0.0;

        if self.x.is_empty() {
            return 0.0;
        } else if x > self.x[self.x.len() - 1] {
            let w = self.gaussian_filter.evaluate(0.0);
            sum += w * self.y[self.y.len() - 1];
            total_weight += w;
        } else if x < self.x[0] {
            let w = self.gaussian_filter.evaluate(0.0);
            sum += w * self.y[0];
            total_weight += w;
        }

        let mut i = closest;
        while i >= 0 {
            let iu = i as usize;
            if (x - self.x[iu]).abs() > filter_radius {
                break;
            }

            let w = self
                .gaussian_filter
                .evaluate((self.x[iu] - x) / filter_radius);
            sum += w * self.y[iu];
            total_weight += w;
            i -= 1;
        }

        let mut i = closest + 1;
        while i < self.x.len() as i32 {
            let iu = i as usize;
            if (self.x[iu] - x).abs() > filter_radius {
                break;
            }

            let w = self
                .gaussian_filter
                .evaluate((self.x[iu] - x) / filter_radius);
            sum += w * self.y[iu];
            total_weight += w;
            i += 1;
        }

        if total_weight != 0.0 {
            sum * self.output_scale / total_weight
        } else {
            0.0
        }
    }

    /// `Function::isOrdered`
    pub fn is_ordered(&self) -> bool {
        self.x.windows(2).all(|w| w[0] <= w[1])
    }

    pub fn domain(&self) -> (f64, f64) {
        match (self.x.first(), self.x.last()) {
            (Some(&a), Some(&b)) => (a, b),
            _ => (0.0, 0.0),
        }
    }

    pub fn range(&self) -> (f64, f64) {
        (self.y_min, self.y_max)
    }

    /// `Function::triangle`
    pub fn triangle(&self, x: f64) -> f64 {
        (self.filter_radius - x.abs()) / self.filter_radius
    }

    /// `Function::closestSample`
    pub fn closest_sample(&self, x: f64) -> i32 {
        if x.is_nan() {
            return 0;
        }

        let mut l: i32 = 0;
        let mut r = self.x.len() as i32 - 1;

        if self.x.is_empty() {
            return -1;
        } else if x <= self.x[0] {
            return l;
        } else if x >= self.x[self.x.len() - 1] {
            return r;
        }

        while l + 1 < r {
            let m = (l + r) / 2;
            if x > self.x[m as usize] {
                l = m;
            } else if x < self.x[m as usize] {
                r = m;
            } else {
                return m;
            }
        }

        // Return the closest of the two bracketing samples.
        if (x - self.x[l as usize]) < (self.x[r as usize] - x) {
            l
        } else {
            r
        }
    }
}
