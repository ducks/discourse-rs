//! The Sass color functions the color definitions use, as dart-sass
//! computes them (no rounding along the way) and writes them out: a color
//! as written when nothing changed it, hex when every channel came out
//! whole, else `rgb()`/`rgba()` in percentages to ten decimals.

/// A legacy (rgb) Sass color: channels 0-255, alpha 0-1, and the text it
/// was written as while unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
    written: Option<String>,
}

impl Color {
    /// `#rgb` or `#rrggbb`, kept as written.
    pub fn hex(text: &str) -> Option<Color> {
        let digits = text.strip_prefix('#').unwrap_or(text);
        let channel = |s: &str| u8::from_str_radix(s, 16).ok().map(f64::from);
        let (r, g, b) = match digits.len() {
            3 => {
                let double = |i: usize| channel(&digits[i..i + 1].repeat(2));
                (double(0)?, double(1)?, double(2)?)
            }
            6 => (
                channel(&digits[0..2])?,
                channel(&digits[2..4])?,
                channel(&digits[4..6])?,
            ),
            _ => return None,
        };
        Some(Color {
            r,
            g,
            b,
            a: 1.0,
            written: Some(format!("#{}", digits.to_ascii_lowercase())),
        })
    }

    /// `rgb(r, g, b)` as written in a stylesheet.
    pub fn written_rgb(r: u8, g: u8, b: u8, text: &str) -> Color {
        Color {
            r: f64::from(r),
            g: f64::from(g),
            b: f64::from(b),
            a: 1.0,
            written: Some(text.to_string()),
        }
    }

    fn computed(r: f64, g: f64, b: f64, a: f64) -> Color {
        Color {
            r,
            g,
            b,
            a,
            written: None,
        }
    }

    /// `color.red()` and friends: the channel rounded, as dart-sass gives
    /// it for a legacy color.
    pub fn channels(&self) -> [f64; 3] {
        [
            fuzzy_round(self.r),
            fuzzy_round(self.g),
            fuzzy_round(self.b),
        ]
    }

    /// `dc-color-brightness`
    pub fn brightness(&self) -> f64 {
        let [r, g, b] = self.channels();
        r * 0.299 + g * 0.587 + b * 0.114
    }

    /// `rgba($color, $alpha)`
    pub fn with_alpha(&self, a: f64) -> Color {
        Color::computed(self.r, self.g, self.b, a)
    }

    /// `color.scale($color, $lightness: $amount)`, `amount` from -1 to 1.
    pub fn scale_lightness(&self, amount: f64) -> Color {
        let (h, s, l) = self.to_hsl();
        let l = if amount > 0.0 {
            l + (100.0 - l) * amount
        } else {
            l + l * amount
        };
        self.with_hsl(h, s, l)
    }

    /// `color.adjust($color, $lightness: $amount)`, `amount` in percent.
    pub fn adjust_lightness(&self, amount: f64) -> Color {
        let (h, s, l) = self.to_hsl();
        self.with_hsl(h, s, (l + amount).clamp(0.0, 100.0))
    }

    /// Hue in degrees, saturation and lightness in percent.
    fn to_hsl(&self) -> (f64, f64, f64) {
        let (r, g, b) = (self.r / 255.0, self.g / 255.0, self.b / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let hue = if delta == 0.0 {
            0.0
        } else if max == r {
            (60.0 * (g - b) / delta).rem_euclid(360.0)
        } else if max == g {
            (120.0 + 60.0 * (b - r) / delta).rem_euclid(360.0)
        } else {
            (240.0 + 60.0 * (r - g) / delta).rem_euclid(360.0)
        };
        let lightness = 50.0 * (max + min);
        let saturation = if max == min {
            0.0
        } else if lightness < 50.0 {
            100.0 * delta / (max + min)
        } else {
            100.0 * delta / (2.0 - max - min)
        };
        (hue, saturation, lightness)
    }

    fn with_hsl(&self, hue: f64, saturation: f64, lightness: f64) -> Color {
        let h = hue / 360.0;
        let s = saturation / 100.0;
        let l = lightness / 100.0;
        let m2 = if l <= 0.5 {
            l * (s + 1.0)
        } else {
            l + s - l * s
        };
        let m1 = l * 2.0 - m2;
        let channel = |mut h: f64| {
            if h < 0.0 {
                h += 1.0;
            }
            if h > 1.0 {
                h -= 1.0;
            }
            let v = if h < 1.0 / 6.0 {
                m1 + (m2 - m1) * h * 6.0
            } else if h < 1.0 / 2.0 {
                m2
            } else if h < 2.0 / 3.0 {
                m1 + (m2 - m1) * (2.0 / 3.0 - h) * 6.0
            } else {
                m1
            };
            v * 255.0
        };
        Color::computed(
            channel(h + 1.0 / 3.0),
            channel(h),
            channel(h - 1.0 / 3.0),
            self.a,
        )
    }

    /// `srgb-scale($foreground, $background, $percent)`: an approximation
    /// of sRGB blending with gamma 2, through Discourse's own `sqrt`.
    pub fn srgb_scale(foreground: &Color, background: &Color, percent: f64) -> Color {
        let ratio = percent / 100.0;
        let iratio = 1.0 - ratio;
        let [fr, fg, fb] = foreground.channels();
        let [br, bg, bb] = background.channels();
        let mix = |f: f64, b: f64| sqrt(f * f * ratio + b * b * iratio);
        Color::computed(mix(fr, br), mix(fg, bg), mix(fb, bb), 1.0)
    }

    /// The color as dart-sass writes it.
    pub fn to_css(&self) -> String {
        if let Some(written) = &self.written {
            return written.clone();
        }
        let whole = |c: f64| (c - c.round()).abs() < 1e-11;
        if self.a >= 1.0 && whole(self.r) && whole(self.g) && whole(self.b) {
            return format!(
                "#{:02x}{:02x}{:02x}",
                self.r.round() as u8,
                self.g.round() as u8,
                self.b.round() as u8
            );
        }
        let percent = |c: f64| format!("{}%", number(c / 255.0 * 100.0));
        if self.a >= 1.0 {
            format!(
                "rgb({}, {}, {})",
                percent(self.r),
                percent(self.g),
                percent(self.b)
            )
        } else {
            format!(
                "rgba({}, {}, {}, {})",
                percent(self.r),
                percent(self.g),
                percent(self.b),
                number(self.a)
            )
        }
    }

    /// `hexToRGB($color)`: the rounded channels, comma separated.
    pub fn to_rgb_list(&self) -> String {
        let [r, g, b] = self.channels();
        format!("{r}, {g}, {b}")
    }
}

/// math.scss's `sqrt`: 24 Newton steps from 1.
fn sqrt(x: f64) -> f64 {
    let mut ret = 1.0;
    for _ in 0..24 {
        ret -= (ret * ret - x) / (2.0 * ret);
    }
    ret
}

/// dart-sass's `fuzzyRound`: to the nearest integer, halves up, within
/// its epsilon.
fn fuzzy_round(n: f64) -> f64 {
    let epsilon = 1e-11;
    if n > 0.0 {
        if (n % 1.0) < 0.5 - epsilon {
            n.floor()
        } else {
            n.ceil()
        }
    } else if (n % 1.0) > -0.5 - epsilon {
        n.ceil()
    } else {
        n.floor()
    }
}

/// A number as dart-sass writes it: up to ten decimals, no trailing
/// zeros.
pub fn number(n: f64) -> String {
    let fixed = format!("{n:.10}");
    let trimmed = fixed.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_come_out_as_dart_sass_writes_them() {
        let primary = Color::hex("#222").unwrap();
        assert_eq!(primary.to_css(), "#222");
        assert_eq!(
            primary.scale_lightness(0.9).to_css(),
            "rgb(91.3333333333%, 91.3333333333%, 91.3333333333%)"
        );
        let tertiary = Color::hex("#08c").unwrap();
        assert_eq!(tertiary.scale_lightness(0.5).to_css(), "#66ccff");
        assert_eq!(primary.scale_lightness(0.9).to_rgb_list(), "233, 233, 233");
        assert_eq!(
            Color::hex("#c80001")
                .unwrap()
                .with_alpha(0.7)
                .scale_lightness(0.5)
                .to_css(),
            "rgba(100%, 39.2156862745%, 39.5196078431%, 0.7)"
        );
    }
}
