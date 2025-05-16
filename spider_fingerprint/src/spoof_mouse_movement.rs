use rand::Rng;
use std::vec::Vec;

/// Represents mouse movements generated using Bézier curves.
pub struct BezierMouse {}

/// Computes the gamma function with an accuracy
/// of 16 floating point digits. "An Analysis Of The Lanczos Gamma Approximation", Glendon Ralph Pugh, 2004.
pub(crate) fn gamma(z: f64) -> f64 {
    const GAMMA_DK: &[f64] = &[
        2.48574089138753565546e-5,
        1.05142378581721974210,
        -3.45687097222016235469,
        4.51227709466894823700,
        -2.98285225323576655721,
        1.05639711577126713077,
        -1.95428773191645869583e-1,
        1.70970543404441224307e-2,
        -5.71926117404305781283e-4,
        4.63399473359905636708e-6,
        -2.71994908488607703910e-9,
    ];
    const TWO_SQRT_E_OVER_PI: f64 = 1.8603827342052657173362492472666631120594218414085755;
    const GAMMA_R: f64 = 10.900511;
    if z < 0.5 {
        std::f64::consts::PI
            / ((std::f64::consts::PI * z).sin()
                * GAMMA_DK
                    .iter()
                    .enumerate()
                    .skip(1)
                    .fold(GAMMA_DK[0], |s, i| s + i.1 / (i.0 as f64 - z))
                * TWO_SQRT_E_OVER_PI
                * ((0.5 - z + GAMMA_R) / std::f64::consts::E).powf(0.5 - z))
    } else {
        GAMMA_DK
            .iter()
            .enumerate()
            .skip(1)
            .fold(GAMMA_DK[0], |s, i| s + i.1 / (z + i.0 as f64 - 1.0))
            * TWO_SQRT_E_OVER_PI
            * ((z - 0.5 + GAMMA_R) / std::f64::consts::E).powf(z - 0.5)
    }
}

impl BezierMouse {
    /// The Bernstein polynomial of n, i as a function of t.
    ///
    /// * `i` - The index of the Bernstein polynomial.
    /// * `n` - The degree of the polynomial.
    /// * `t` - The parameter (array of values) ranging from 0 to 1.
    ///
    /// # Returns
    ///
    /// A vector representing the Bernstein polynomial values.
    pub fn bernstein_poly(i: usize, n: usize, t: &Vec<f64>) -> Vec<f64> {
        t.iter()
            .map(|&t_val| {
                Self::comb(n, i) as f64
                    * (t_val.powi(i as i32))
                    * (1.0 - t_val).powi((n - i) as i32)
            })
            .collect()
    }

    /// Computes the number of combinations (n choose k).
    ///
    /// * `n` - Total number of items.
    /// * `k` - Number of items to choose.
    ///
    /// # Returns
    ///
    /// The number of combinations.
    pub fn comb(n: usize, k: usize) -> usize {
        (gamma((n + 1) as f64) / (gamma((k + 1) as f64) * gamma((n - k + 1) as f64))) as usize
    }

    /// Computes the Bézier curve for the given control points.
    ///
    /// * `points` - A vector of control points as tuples (x, y).
    /// * `num_steps` - The number of steps in the curve.
    ///
    /// # Returns
    ///
    /// A vector of points representing the Bézier curve.
    pub fn bezier_curve(points: &Vec<(f64, f64)>, num_steps: usize) -> Vec<(f64, f64)> {
        let n_points = points.len();
        let xpoints: Vec<f64> = points.iter().map(|p| p.0).collect();
        let ypoints: Vec<f64> = points.iter().map(|p| p.1).collect();
        let t: Vec<f64> = (0..num_steps)
            .map(|i| i as f64 / (num_steps as f64 - 1.0))
            .collect();

        let polynomial_array: Vec<Vec<f64>> = (0..n_points)
            .map(|i| Self::bernstein_poly(i, n_points - 1, &t))
            .collect();

        let xvals: Vec<f64> = polynomial_array
            .iter()
            .map(|poly| {
                poly.iter()
                    .zip(&xpoints)
                    .map(|(&p, &x)| (p * x as f64))
                    .sum()
            })
            .collect();

        let yvals: Vec<f64> = polynomial_array
            .iter()
            .map(|poly| {
                poly.iter()
                    .zip(&ypoints)
                    .map(|(&p, &y)| (p * y as f64))
                    .sum()
            })
            .collect();

        xvals.into_iter().zip(yvals.into_iter()).collect()
    }

    /// Generates mouse movements using Bézier curves.
    ///
    /// * `start_x` - The starting x-coordinate.
    /// * `start_y` - The starting y-coordinate.
    /// * `end_x` - The ending x-coordinate.
    /// * `end_y` - The ending y-coordinate.
    /// * `duration` - The duration of the movement (in seconds).
    /// * `complexity` - The number of control points.
    /// * `randomness` - Controls the randomness of control points (0.0 to 1.0).
    ///
    /// # Returns
    ///
    /// A vector of points representing the mouse movements.
    pub fn generate_bezier_mouse_movements(
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        duration: f64,
        complexity: usize,
        randomness: f64,
    ) -> Vec<(f64, f64)> {
        let complexity = usize::max(4, complexity);
        let mut control_points = vec![(start_x, start_y)];

        let mut rng = rand::rng();

        for _ in 0..complexity - 2 {
            let cx = rng.random_range(start_x..=end_x);
            let cy = rng.random_range(start_y..=end_y);
            control_points.push((cx, cy));
        }

        control_points.push((end_x, end_y));

        for i in 1..control_points.len() - 1 {
            control_points[i].0 += rng.random_range(-(randomness * 100.0)..=(randomness * 100.0));
            control_points[i].1 += rng.random_range(-(randomness * 100.0)..=(randomness * 100.0));
        }

        let num_steps = (duration * 60.0) as usize;

        Self::bezier_curve(&control_points, num_steps)
    }

    /// Generates a list of coordinates using Bézier curves.
    ///
    /// * `from_x` - The starting x-coordinate.
    /// * `from_y` - The starting y-coordinate.
    /// * `to_x` - The ending x-coordinate.
    /// * `to_y` - The ending y-coordinate.
    ///
    /// # Returns
    ///
    /// A vector of coordinates from start to end.
    pub fn generate_coordinates(from_x: f64, from_y: f64, to_x: f64, to_y: f64) -> Vec<(f64, f64)> {
        Self::generate_bezier_mouse_movements(from_x, from_y, to_x, to_y, 1.0, 4, 1.0)
    }

    /// Generates random coordinates using Bézier curves within the given viewport dimensions.
    ///
    /// * `viewport_width` - The width of the viewport.
    /// * `viewport_height` - The height of the viewport.
    ///
    /// # Returns
    ///
    /// Randomly generated coordinates within the viewport.
    pub fn generate_random_coordinates(
        viewport_width: f64,
        viewport_height: f64,
    ) -> Vec<(f64, f64)> {
        let start_x = 0.0;
        let start_y = 0.0;
        let mut rng = rand::rng();

        let end_x = rng.random_range(0.0..=viewport_width);
        let end_y = rng.random_range(0.0..=viewport_height);

        Self::generate_bezier_mouse_movements(start_x, start_y, end_x, end_y, 1.0, 4, 1.0)
    }

    /// Generates a list of y-coordinates for scrolling using Bézier curves.
    ///
    /// * `start_y` - The starting y-coordinate.
    /// * `end_y` - The ending y-coordinate.
    ///
    /// # Returns
    ///
    /// A vector of y-coordinates for scrolling.
    pub fn generate_scroll_coordinates(start_y: f64, end_y: f64) -> Vec<(f64, f64)> {
        let movements =
            Self::generate_bezier_mouse_movements(0.0, start_y, 0.0, end_y, 1.0, 4, 1.0);
        let mut y_coords: Vec<f64> = movements.iter().map(|&m| m.1).collect();
        y_coords.push(end_y);
        let x_coords: Vec<f64> = vec![0.0; y_coords.len()];
        x_coords.into_iter().zip(y_coords.into_iter()).collect()
    }
}

/// Represents mouse movements generated using Gaussian random walk and Bézier curves.
pub struct GaussianMouse;

impl GaussianMouse {
    /// Generates a random walk of specified length and standard deviation.
    ///
    /// # Arguments
    ///
    /// * `length` - The number of steps in the random walk.
    /// * `stddev` - The standard deviation of the distribution.
    ///
    /// # Returns
    ///
    /// A vector representing the cumulative sum of random values.
    fn random_walk(length: usize, stddev: f64) -> Vec<f64> {
        let mut rng = rand::rng();
        let mut walk = Vec::with_capacity(length);
        let mut current = 0.0;

        for _ in 0..length {
            let step: f64 = rng.random::<f64>().copysign(stddev);
            current += step;
            walk.push(current);
        }

        walk
    }

    /// Applies a Gaussian smoothing function to the input data.
    ///
    /// # Arguments
    ///
    /// * `data` - The input data vector.
    /// * `sigma` - The standard deviation for Gaussian smoothing.
    ///
    /// # Returns
    ///
    /// A vector representing the smoothed data.
    fn gaussian_smooth(data: &[f64], sigma: f64) -> Vec<f64> {
        // Example smoothing, replace with a proper Gaussian smoothing logic
        // For now, making a simple moving average as a placeholder
        let mut smoothed: Vec<f64> = vec![0.0; data.len()];
        let kernel_size = (6.0 * sigma).ceil() as usize;
        for (index, _) in data.iter().enumerate() {
            let mut sum = 0.0;
            let mut count = 0.0;
            for i in 0..kernel_size {
                if index + i < data.len() {
                    sum += data[index + i];
                    count += 1.0;
                }
                if index as i32 - i as i32 >= 0 {
                    sum += data[index - i];
                    count += 1.0;
                }
            }
            smoothed[index] = sum / count;
        }
        smoothed
    }

    /// Morphs a distribution to match target mean and standard deviation.
    ///
    /// # Arguments
    ///
    /// * `data` - The input data vector.
    /// * `target_mean` - Target mean value.
    /// * `target_std` - Target standard deviation.
    ///
    /// # Returns
    ///
    /// A vector representing the morphed data distribution.
    fn morph_distribution(data: &[f64], target_mean: f64, target_std: f64) -> Vec<f64> {
        let mean: f64 = data.iter().sum::<f64>() / (data.len() as f64);
        let std: f64 =
            (data.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / (data.len() as f64)).sqrt();

        data.iter()
            .map(|&x| ((x - mean) / std) * target_std + target_mean)
            .collect()
    }

    /// Computes a point on a quadratic Bezier curve.
    ///
    /// # Arguments
    ///
    /// * `p0` - The starting point.
    /// * `p1` - The control point.
    /// * `p2` - The ending point.
    /// * `t` - The parameter ranging from 0 to 1.
    ///
    /// # Returns
    ///
    /// A point on the Bézier curve.
    fn bezier_curve(p0: f64, p1: f64, p2: f64, t: f64) -> f64 {
        (1.0 - t).powi(2) * p0 + 2.0 * (1.0 - t) * t * p1 + t.powi(2) * p2
    }

    /// Generates mouse movements using Gaussian random walk and Bézier curves.
    ///
    /// # Arguments
    ///
    /// * `start_x` - Starting x-coordinate.
    /// * `start_y` - Starting y-coordinate.
    /// * `end_x` - Ending x-coordinate.
    /// * `end_y` - Ending y-coordinate.
    /// * `duration` - Duration of the movement in seconds.
    /// * `smoothness` - Controls the smoothness of the path (higher value = smoother).
    /// * `randomness` - Controls the randomness of the path (higher value = more random).
    ///
    /// # Returns
    ///
    /// A vector of tuples representing the mouse movements.
    fn generate_gaussian_mouse_movements(
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        duration: f64,
        smoothness: f64,
        randomness: f64,
    ) -> Vec<(f64, f64)> {
        let num_points = (duration * 60.0) as usize;
        let stddev = randomness * 10.0;

        // Random walks for x and y axes
        let random_x = Self::random_walk(num_points, stddev);
        let random_y = Self::random_walk(num_points, stddev);

        // Smoothing random walks
        let smooth_x = Self::gaussian_smooth(&random_x, smoothness);
        let smooth_y = Self::gaussian_smooth(&random_y, smoothness);

        // Morphing distributions to match human-like movements
        let human_mean_x = (end_x - start_x) as f64 / 2.0;
        let human_std_x = (end_x - start_x) as f64 / 6.0;
        let morphed_x = Self::morph_distribution(&smooth_x, human_mean_x, human_std_x);

        let human_mean_y = (end_y - start_y) as f64 / 2.0;
        let human_std_y = (end_y - start_y) as f64 / 6.0;
        let morphed_y = Self::morph_distribution(&smooth_y, human_mean_y, human_std_y);

        let mut rng = rand::rng();
        let control_x = rng.random_range(start_x..=end_x) as f64;
        let control_y = rng.random_range(start_y..=end_y) as f64;

        let t_values: Vec<f64> = (0..num_points)
            .map(|i| i as f64 / num_points as f64)
            .collect();

        // Generate Bezier curve paths
        let bezier_x: Vec<f64> = t_values
            .iter()
            .map(|&t| Self::bezier_curve(start_x as f64, control_x, end_x as f64, t))
            .collect();
        let bezier_y: Vec<f64> = t_values
            .iter()
            .map(|&t| Self::bezier_curve(start_y as f64, control_y, end_y as f64, t))
            .collect();

        // Final composed path
        let mut final_x = vec![start_x as f64];
        final_x.extend(bezier_x.iter().zip(&morphed_x).map(|(&bx, &mx)| bx + mx));
        final_x.push(end_x as f64);

        let mut final_y = vec![start_y as f64];
        final_y.extend(bezier_y.iter().zip(&morphed_y).map(|(&by, &my)| by + my));
        final_y.push(end_y as f64);

        // Combine x and y into a path
        final_x.into_iter().zip(final_y.into_iter()).collect()
    }

    /// Generates a list of coordinates using Gaussian random walk.
    ///
    /// # Arguments
    ///
    /// * `from_x` - The starting x-coordinate.
    /// * `from_y` - The starting y-coordinate.
    /// * `to_x` - The ending x-coordinate.
    /// * `to_y` - The ending y-coordinate.
    ///
    /// # Returns
    ///
    /// A vector of coordinates from start to end.
    pub fn generate_coordinates(from_x: f64, from_y: f64, to_x: f64, to_y: f64) -> Vec<(f64, f64)> {
        Self::generate_gaussian_mouse_movements(from_x, from_y, to_x, to_y, 1.0, 2.0, 1.0)
    }

    /// Generates random coordinates using Gaussian random walk within viewport dimensions.
    ///
    /// # Arguments
    ///
    /// * `viewport_width` - The width of the viewport.
    /// * `viewport_height` - The height of the viewport.
    ///
    /// # Returns
    ///
    /// Randomly generated coordinates within the viewport.
    pub fn generate_random_coordinates(
        viewport_width: f64,
        viewport_height: f64,
    ) -> Vec<(f64, f64)> {
        let start_x = 0.0;
        let start_y = 0.0;
        let mut rng = rand::rng();
        let end_x = rng.random_range(0.0..=viewport_width);
        let end_y = rng.random_range(0.0..=viewport_height);

        Self::generate_gaussian_mouse_movements(start_x, start_y, end_x, end_y, 1.0, 2.0, 1.0)
    }

    /// Generates a list of y-coordinates for scrolling using Gaussian random walk.
    ///
    /// # Arguments
    ///
    /// * `start_y` - The starting y-coordinate.
    /// * `end_y` - The ending y-coordinate.
    ///
    /// # Returns
    ///
    /// A vector of y-coordinates for scrolling.
    pub fn generate_scroll_coordinates(start_y: f64, end_y: f64) -> Vec<(f64, f64)> {
        let movements =
            Self::generate_gaussian_mouse_movements(0.0, start_y, 0.0, end_y, 1.0, 2.0, 1.0);
        let mut y_coords: Vec<f64> = movements.iter().map(|&(_, y)| y).collect();
        y_coords.push(end_y as f64);
        let x_coords: Vec<f64> = vec![0.0; y_coords.len()];

        x_coords.into_iter().zip(y_coords.into_iter()).collect()
    }
}

#[test]
fn random_cord() {
    let cords = BezierMouse::generate_random_coordinates(800.0, 600.0);
    assert!(cords.len() >= 2, "random bezier cords did not generate");
    let cords = GaussianMouse::generate_random_coordinates(800.0, 600.0);
    assert!(cords.len() >= 2, "random gaussian cords did not generate");
}
