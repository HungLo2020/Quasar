use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NavError {
    InvalidDimensions,
    CellCountMismatch,
    OutOfBounds,
    InvalidCost,
    Tilemap(String),
    Navmesh(String),
}

impl NavError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidDimensions => "nav.invalid_dimensions",
            Self::CellCountMismatch => "nav.cell_count_mismatch",
            Self::OutOfBounds => "nav.out_of_bounds",
            Self::InvalidCost => "nav.invalid_cost",
            Self::Tilemap(_) => "nav.tilemap",
            Self::Navmesh(_) => "nav.navmesh",
        }
    }
}

impl fmt::Display for NavError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => {
                formatter.write_str("grid dimensions must be nonzero and fit in memory")
            }
            Self::CellCountMismatch => {
                formatter.write_str("blocked flags do not match grid dimensions")
            }
            Self::OutOfBounds => formatter.write_str("grid cell is out of bounds"),
            Self::InvalidCost => {
                formatter.write_str("movement cost must be finite and non-negative")
            }
            Self::Tilemap(message) => write!(formatter, "tilemap adapter: {message}"),
            Self::Navmesh(message) => write!(formatter, "navmesh adapter: {message}"),
        }
    }
}

impl std::error::Error for NavError {}
pub type NavResult<T> = Result<T, NavError>;
