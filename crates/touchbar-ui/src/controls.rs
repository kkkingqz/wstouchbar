use crate::ColorRole;

/// A finite scalar range shared by sliders, meters, and value adapters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContinuousValue {
    minimum: f32,
    maximum: f32,
    value: f32,
    step: Option<f32>,
}

impl ContinuousValue {
    pub fn new(minimum: f32, maximum: f32, value: f32) -> Self {
        let minimum = if minimum.is_finite() { minimum } else { 0.0 };
        let maximum = if maximum.is_finite() && maximum > minimum {
            maximum
        } else {
            minimum + 1.0
        };
        let value = if value.is_nan() {
            minimum
        } else if value.is_infinite() {
            if value.is_sign_positive() {
                maximum
            } else {
                minimum
            }
        } else {
            value.clamp(minimum, maximum)
        };
        Self {
            minimum,
            maximum,
            value,
            step: None,
        }
    }

    pub fn unit(value: f32) -> Self {
        Self::new(0.0, 1.0, value)
    }

    pub fn step(mut self, step: f32) -> Self {
        self.step = (step.is_finite() && step > 0.0).then_some(step);
        self.value = self.quantize(self.value);
        self
    }

    pub fn minimum(self) -> f32 {
        self.minimum
    }

    pub fn maximum(self) -> f32 {
        self.maximum
    }

    pub fn value(self) -> f32 {
        self.value
    }

    pub fn normalized(self) -> f32 {
        ((self.value - self.minimum) / (self.maximum - self.minimum)).clamp(0.0, 1.0)
    }

    pub fn value_at(self, normalized: f32) -> f32 {
        let normalized = if normalized.is_finite() {
            normalized.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.quantize(self.minimum + (self.maximum - self.minimum) * normalized)
    }

    pub fn with_normalized(self, normalized: f32) -> Self {
        Self {
            value: self.value_at(normalized),
            ..self
        }
    }

    fn quantize(self, value: f32) -> f32 {
        self.step.map_or(value, |step| {
            (self.minimum + ((value - self.minimum) / step).round() * step)
                .clamp(self.minimum, self.maximum)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderStyle {
    pub track: ColorRole,
    pub fill: ColorRole,
    pub thumb: ColorRole,
    pub track_height: f32,
    pub thumb_diameter: f32,
    pub tick_count: u8,
    pub show_thumb: bool,
}

impl Default for SliderStyle {
    fn default() -> Self {
        Self {
            track: ColorRole::Track,
            fill: ColorRole::Accent,
            thumb: ColorRole::Foreground,
            track_height: 7.0,
            thumb_diameter: 18.0,
            tick_count: 0,
            show_thumb: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterStyle {
    pub track: ColorRole,
    pub low: ColorRole,
    pub high: ColorRole,
    pub peak: ColorRole,
    /// Zero draws a continuous meter; otherwise this is the segment count.
    pub segments: u8,
    pub gap: f32,
}

impl Default for MeterStyle {
    fn default() -> Self {
        Self {
            track: ColorRole::Track,
            low: ColorRole::Accent,
            high: ColorRole::Foreground,
            peak: ColorRole::Foreground,
            segments: 0,
            gap: 2.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TinyGraphStyle {
    pub low: ColorRole,
    pub high: ColorRole,
    pub baseline: Option<ColorRole>,
    pub gap: f32,
    pub minimum_bar_height: f32,
}

impl Default for TinyGraphStyle {
    fn default() -> Self {
        Self {
            low: ColorRole::Muted,
            high: ColorRole::Accent,
            baseline: Some(ColorRole::Track),
            gap: 1.0,
            minimum_bar_height: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrubberCellStyle {
    pub background: ColorRole,
    pub highlighted_background: ColorRole,
    pub selected_background: ColorRole,
    pub foreground: ColorRole,
    pub selected_foreground: ColorRole,
    pub corner_radius: f32,
}

impl Default for ScrubberCellStyle {
    fn default() -> Self {
        Self {
            background: ColorRole::Control,
            highlighted_background: ColorRole::ControlPressed,
            selected_background: ColorRole::Accent,
            foreground: ColorRole::Foreground,
            selected_foreground: ColorRole::OnAccent,
            corner_radius: 8.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_values_clamp_map_and_quantize() {
        let value = ContinuousValue::new(-10.0, 10.0, 30.0).step(2.5);
        assert_eq!(value.value(), 10.0);
        assert_eq!(value.normalized(), 1.0);
        assert_eq!(value.value_at(0.63), 2.5);
        assert_eq!(value.with_normalized(-1.0).value(), -10.0);
    }

    #[test]
    fn invalid_ranges_are_safely_normalized() {
        let value = ContinuousValue::new(f32::NAN, -2.0, f32::INFINITY);
        assert_eq!(value.minimum(), 0.0);
        assert_eq!(value.maximum(), 1.0);
        assert_eq!(value.value(), 1.0);
        assert_eq!(value.value_at(f32::NAN), 0.0);
    }
}
