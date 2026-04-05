use common::types::PriceMatrix;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathError {
    #[error("Invalid hop count {0}; must be between 2 and 6")]
    InvalidHops(usize),
    #[error("Matrix is empty")]
    EmptyMatrix,
}

pub struct PathFinder {
    max_hops: usize,
}

impl PathFinder {
    pub fn new(max_hops: usize) -> Result<Self, PathError> {
        if !(2..=6).contains(&max_hops) {
            return Err(PathError::InvalidHops(max_hops));
        }
        Ok(Self { max_hops })
    }

    /// Simple count of possible cycles (for diagnostics)
    #[must_use]
    pub fn count_paths(&self, matrix: &PriceMatrix) -> usize {
        if matrix.n < 2 {
            return 0;
        }
        let mut count = 0;
        for src in 0..matrix.n {
            count += self.count_from(matrix, src, src, 0);
        }
        count
    }

    fn count_from(&self, matrix: &PriceMatrix, src: usize, cur: usize, depth: usize) -> usize {
        if depth > self.max_hops {
            return 0;
        }

        let mut count = 0;
        for next in 0..matrix.n {
            if next == cur {
                continue;
            }
            if matrix.get(cur, next).unwrap_or(f64::INFINITY).is_infinite() {
                continue;
            }

            if next == src && depth >= 2 {
                count += 1;
            } else if depth < self.max_hops {
                count += self.count_from(matrix, src, next, depth + 1);
            }
        }
        count
    }
}
