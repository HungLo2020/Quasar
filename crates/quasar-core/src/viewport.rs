#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    Nearest,
    Linear,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scaling {
    #[default]
    Fit,
    /// Whole framebuffer-pixel multiples; falls back to Fit in smaller windows.
    /// Intended for square pixels; custom display aspect uses Fit.
    Integer,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct DisplayOptions {
    /// Display proportions, independently of framebuffer dimensions (non-square pixels).
    pub aspect: Option<(u32, u32)>,
    pub scaling: Scaling,
    pub filter: Filter,
}
impl DisplayOptions {
    pub fn validate(self) -> Result<(), &'static str> {
        if self.aspect.is_some_and(|(w, h)| w == 0 || h == 0) {
            return Err("display aspect must be positive");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    source: (u32, u32),
}
impl Viewport {
    pub fn calculate(
        source: (u32, u32),
        target: (u32, u32),
        options: DisplayOptions,
    ) -> Option<Self> {
        if source.0 == 0
            || source.1 == 0
            || target.0 == 0
            || target.1 == 0
            || options.validate().is_err()
        {
            return None;
        }
        let aspect = options.aspect.unwrap_or(source);
        let integer = options.scaling == Scaling::Integer
            && u64::from(aspect.0) * u64::from(source.1)
                == u64::from(aspect.1) * u64::from(source.0);
        let scale = (target.0 / source.0).min(target.1 / source.1);
        let (width, height) = if integer && scale > 0 {
            (source.0 * scale, source.1 * scale)
        } else if u64::from(target.0) * u64::from(aspect.1)
            > u64::from(target.1) * u64::from(aspect.0)
        {
            (
                ((u64::from(target.1) * u64::from(aspect.0) / u64::from(aspect.1)) as u32).max(1),
                target.1,
            )
        } else {
            (
                target.0,
                ((u64::from(target.0) * u64::from(aspect.1) / u64::from(aspect.0)) as u32).max(1),
            )
        };
        Some(Self {
            x: (target.0 - width) / 2,
            y: (target.1 - height) / 2,
            width,
            height,
            source,
        })
    }
    /// Maps physical cursor coordinates to framebuffer coordinates. Bars return None.
    pub fn to_source(self, x: f64, y: f64) -> Option<(f64, f64)> {
        let x = x - self.x as f64;
        let y = y - self.y as f64;
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x >= self.width as f64
            || y >= self.height as f64
        {
            return None;
        }
        Some((
            x * self.source.0 as f64 / self.width as f64,
            y * self.source.1 as f64 / self.height as f64,
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fit_integer_small_windows_and_non_square_pixels() {
        let v = Viewport::calculate((320, 200), (1000, 600), DisplayOptions::default()).unwrap();
        assert_eq!((v.x, v.y, v.width, v.height), (20, 0, 960, 600));
        assert_eq!(v.to_source(20.0, 0.0), Some((0.0, 0.0)));
        assert!(v.to_source(0.0, 0.0).is_none());
        let options = DisplayOptions {
            scaling: Scaling::Integer,
            ..Default::default()
        };
        let v = Viewport::calculate((320, 200), (900, 600), options).unwrap();
        assert_eq!((v.width, v.height), (640, 400));
        let v = Viewport::calculate((320, 200), (160, 100), options).unwrap();
        assert_eq!((v.width, v.height), (160, 100));
        let v = Viewport::calculate(
            (160, 210),
            (800, 600),
            DisplayOptions {
                aspect: Some((4, 3)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!((v.width, v.height), (800, 600));
        assert!(Viewport::calculate((0, 200), (800, 600), options).is_none());
        assert!(Viewport::calculate((320, 200), (0, 0), options).is_none());
    }
}
