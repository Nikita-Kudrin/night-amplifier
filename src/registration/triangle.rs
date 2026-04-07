//! Triangle types for star pattern matching.
//!
//! Triangles are formed from triplets of stars and used for scale-invariant
//! pattern matching between frames.

use crate::detection::Star;

/// A triangle formed by three stars (an asterism).
#[derive(Debug, Clone)]
pub struct Triangle {
    /// Indices of the three stars forming this triangle.
    pub indices: [usize; 3],
    /// Side lengths sorted ascending: [shortest, middle, longest].
    pub sides: [f32; 3],
    /// Scale-invariant descriptor: (shortest/longest, middle/longest).
    pub descriptor: (f32, f32),
}

impl Triangle {
    /// Creates a triangle from three stars.
    pub fn from_stars(stars: &[Star], i: usize, j: usize, k: usize) -> Self {
        let a = stars[i].distance_to(&stars[j]);
        let b = stars[j].distance_to(&stars[k]);
        let c = stars[k].distance_to(&stars[i]);

        let mut sides = [a, b, c];
        sides.sort_by(|x, y| x.partial_cmp(y).unwrap());

        let longest = sides[2].max(1e-6);
        let descriptor = (sides[0] / longest, sides[1] / longest);

        Triangle {
            indices: [i, j, k],
            sides,
            descriptor,
        }
    }

    /// Checks if this triangle's descriptor matches another within tolerance.
    pub fn matches(&self, other: &Triangle, tolerance: f32) -> bool {
        let d0 = (self.descriptor.0 - other.descriptor.0).abs();
        let d1 = (self.descriptor.1 - other.descriptor.1).abs();
        d0 < tolerance && d1 < tolerance
    }

    /// Returns the perimeter of the triangle.
    pub fn perimeter(&self) -> f32 {
        self.sides.iter().sum()
    }

    /// Returns the area of the triangle using Heron's formula.
    pub fn area(&self) -> f32 {
        let s = self.perimeter() / 2.0;
        let a = self.sides[0];
        let b = self.sides[1];
        let c = self.sides[2];
        (s * (s - a) * (s - b) * (s - c)).max(0.0).sqrt()
    }
}

/// Computes the side length opposite to each vertex in a triangle.
pub fn vertex_opposite_sides(stars: &[Star], indices: &[usize; 3]) -> [f32; 3] {
    let s0 = stars[indices[1]].distance_to(&stars[indices[2]]);
    let s1 = stars[indices[0]].distance_to(&stars[indices[2]]);
    let s2 = stars[indices[0]].distance_to(&stars[indices[1]]);
    [s0, s1, s2]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_stars() -> Vec<Star> {
        vec![
            Star::new(100.0, 100.0, 1000.0, 0.9, 50.0),
            Star::new(200.0, 100.0, 900.0, 0.85, 45.0),
            Star::new(150.0, 200.0, 800.0, 0.8, 40.0),
        ]
    }

    #[test]
    fn test_triangle_creation() {
        let stars = create_test_stars();
        let triangle = Triangle::from_stars(&stars, 0, 1, 2);

        assert!(triangle.sides[0] <= triangle.sides[1]);
        assert!(triangle.sides[1] <= triangle.sides[2]);
        assert!(triangle.descriptor.0 <= 1.0);
        assert!(triangle.descriptor.1 <= 1.0);
        assert!(triangle.descriptor.0 <= triangle.descriptor.1);
    }

    #[test]
    fn test_triangle_matching() {
        let stars = create_test_stars();
        let t1 = Triangle::from_stars(&stars, 0, 1, 2);
        let t2 = Triangle::from_stars(&stars, 0, 1, 2);

        assert!(t1.matches(&t2, 0.01));
    }

    #[test]
    fn test_triangle_area() {
        let stars = vec![
            Star::new(0.0, 0.0, 100.0, 0.5, 10.0),
            Star::new(10.0, 0.0, 100.0, 0.5, 10.0),
            Star::new(0.0, 10.0, 100.0, 0.5, 10.0),
        ];
        let triangle = Triangle::from_stars(&stars, 0, 1, 2);

        assert!((triangle.area() - 50.0).abs() < 0.1);
    }
}
