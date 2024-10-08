use rand::{rngs::SmallRng, Rng, SeedableRng};
use statrs::function::gamma::gamma;
use std::vec::Vec;

/// Represents mouse movements generated using Bézier curves.
pub struct BezierMouse {}

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

        let mut rng = SmallRng::from_entropy();

        for _ in 0..complexity - 2 {
            let cx = rng.gen_range(start_x..=end_x);
            let cy = rng.gen_range(start_y..=end_y);
            control_points.push((cx, cy));
        }

        control_points.push((end_x, end_y));

        for i in 1..control_points.len() - 1 {
            control_points[i].0 += rng.gen_range(-(randomness * 100.0)..=(randomness * 100.0));
            control_points[i].1 += rng.gen_range(-(randomness * 100.0)..=(randomness * 100.0));
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
        let end_x = SmallRng::from_entropy().gen_range(0.0..=viewport_width);
        let end_y = SmallRng::from_entropy().gen_range(0.0..=viewport_height);

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

#[tokio::test]
async fn random_cord() {
    let cords = BezierMouse::generate_random_coordinates(720.0, 1200.0);
    assert!(cords.len() >= 2, "random cords did not generate");
}
