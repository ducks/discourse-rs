//! A color is internally represented as either RGBA or HSLA.
//!
//! Colors can be constructed in Sass through names (e.g. red, blue, aqua)
//! or the builtin functions `rgb()`, `rgba()`, `hsl()`, and `hsla()`,
//! all of which can accept 1-4 arguments.
//!
//! It is necessary to retain the original values with which the
//! color was constructed.
//! E.g. `hsla(.999999999999, 100, 100, 1)` should retain its full HSLA
//! values to an arbitrary precision.
//!
//! Color values matching named colors are implicitly converted to named colors
//! E.g. `rgba(255, 0, 0, 1)` => `red`
//!
//! Named colors retain their original casing,
//! so `rEd` should be emitted as `rEd`.
//!
//! discourse-rs: legacy colors follow dart-sass 1.104 (its color spaces
//! module). Channels are never rounded along the way: the RGB channels
//! are kept as computed, conversions between RGB and HSL use them as they
//! are, and a color keeps the space it was made in (`hsl()` makes an HSL
//! color; `lighten()` and friends work in HSL and convert back to the
//! color's own space). Only `red()`, `green()` and `blue()` round, as
//! dart-sass's do, and the serializer decides how the result is written.

use crate::value::{fuzzy_equals, fuzzy_round, Number};
pub(crate) use name::NAMED_COLORS;

mod name;

// todo: only store alpha once on color
#[derive(Debug, Clone)]
pub struct Color {
    rgba: Rgb,
    /// The HSL channels of a color in the HSL space (hue in degrees,
    /// saturation and lightness from 0 to 1); None in the RGB space.
    hsla: Option<Hsl>,
    alpha: Number,
    pub(crate) format: ColorFormat,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum ColorFormat {
    Rgb,
    Hsl,
    /// Literal string from source text. Either a named color like `red` or a hex color
    // todo: make this is a span and lookup text from codemap
    Literal(String),
    /// Use the most appropriate format
    Infer,
}

impl PartialEq for Color {
    fn eq(&self, other: &Self) -> bool {
        fuzzy_equals(self.alpha().0, other.alpha().0) && self.rgba == other.rgba
    }
}

impl Eq for Color {}

impl Color {
    pub(crate) const fn new_rgba(
        red: Number,
        green: Number,
        blue: Number,
        alpha: Number,
        format: ColorFormat,
    ) -> Color {
        Color {
            rgba: Rgb::new(red, green, blue),
            alpha,
            hsla: None,
            format,
        }
    }
}

#[derive(Debug, Clone)]
struct Rgb {
    red: Number,
    green: Number,
    blue: Number,
}

impl PartialEq for Rgb {
    fn eq(&self, other: &Self) -> bool {
        fuzzy_equals(self.red.0, other.red.0)
            && fuzzy_equals(self.green.0, other.green.0)
            && fuzzy_equals(self.blue.0, other.blue.0)
    }
}

impl Eq for Rgb {}

impl Rgb {
    pub const fn new(red: Number, green: Number, blue: Number) -> Self {
        Rgb { red, green, blue }
    }
}

#[derive(Debug, Clone)]
struct Hsl {
    hue: Number,
    saturation: Number,
    luminance: Number,
}

impl Hsl {
    pub const fn new(hue: Number, saturation: Number, luminance: Number) -> Self {
        Hsl {
            hue,
            saturation,
            luminance,
        }
    }
}

// RGBA color functions
impl Color {
    pub fn new(red: u8, green: u8, blue: u8, alpha: u8, format: String) -> Self {
        Color {
            rgba: Rgb::new(red.into(), green.into(), blue.into()),
            hsla: None,
            alpha: alpha.into(),
            format: ColorFormat::Literal(format),
        }
    }

    /// Create a new `Color` with just RGBA values.
    /// Color representation is created automatically.
    pub fn from_rgba(
        mut red: Number,
        mut green: Number,
        mut blue: Number,
        mut alpha: Number,
    ) -> Self {
        red = red.clamp(0.0, 255.0);
        green = green.clamp(0.0, 255.0);
        blue = blue.clamp(0.0, 255.0);
        alpha = alpha.clamp(0.0, 1.0);

        Color::new_rgba(red, green, blue, alpha, ColorFormat::Infer)
    }

    pub fn from_rgba_fn(
        mut red: Number,
        mut green: Number,
        mut blue: Number,
        mut alpha: Number,
    ) -> Self {
        red = red.clamp(0.0, 255.0);
        green = green.clamp(0.0, 255.0);
        blue = blue.clamp(0.0, 255.0);
        alpha = alpha.clamp(0.0, 1.0);

        Color::new_rgba(red, green, blue, alpha, ColorFormat::Rgb)
    }

    /// `color.red` in dart-sass: the channel rounded.
    pub fn red(&self) -> Number {
        self.rgba.red.round()
    }

    pub fn blue(&self) -> Number {
        self.rgba.blue.round()
    }

    pub fn green(&self) -> Number {
        self.rgba.green.round()
    }

    /// The RGB channels as computed (`color.toSpace(rgb).channel0` ...).
    pub(crate) fn rgb_channels(&self) -> (f64, f64, f64) {
        (self.rgba.red.0, self.rgba.green.0, self.rgba.blue.0)
    }

    /// Whether the color is in the HSL space.
    pub(crate) fn is_hsl_space(&self) -> bool {
        self.hsla.is_some()
    }

    /// `isInGamut`: RGB channels within 0-255; an HSL color's saturation
    /// and lightness within 0-100%.
    pub(crate) fn is_in_gamut(&self) -> bool {
        let within = |v: f64, max: f64| {
            (v < max || fuzzy_equals(v, max)) && (v > 0.0 || fuzzy_equals(v, 0.0))
        };
        match &self.hsla {
            Some(h) => within(h.saturation.0, 1.0) && within(h.luminance.0, 1.0),
            None => {
                let (r, g, b) = self.rgb_channels();
                within(r, 255.0) && within(g, 255.0) && within(b, 255.0)
            }
        }
    }

    /// This color converted back to the space `original` is in
    /// (`toSpace(color.space)`): an RGB color loses the HSL channels and
    /// any format.
    pub(crate) fn in_space_of(mut self, original: &Color) -> Self {
        if original.hsla.is_none() {
            self.hsla = None;
            self.format = ColorFormat::Infer;
        }
        self
    }

    /// The color in the RGB space (`toSpace(rgb)`).
    pub(crate) fn to_rgb_space(&self) -> Self {
        if self.hsla.is_none() {
            return self.clone();
        }
        Color::new_rgba(
            self.rgba.red,
            self.rgba.green,
            self.rgba.blue,
            self.alpha,
            ColorFormat::Infer,
        )
    }

    /// Mix two colors together with weight (dart-sass's `_mixLegacy`,
    /// channels unrounded).
    pub fn mix(&self, other: &Color, weight: Number) -> Self {
        let weight = weight.clamp(0.0, 100.0);
        let normalized_weight = weight * Number(2.0) - Number::one();
        let alpha_distance = self.alpha() - other.alpha();

        let combined_weight1 = if normalized_weight * alpha_distance == Number(-1.0) {
            normalized_weight
        } else {
            (normalized_weight + alpha_distance)
                / (Number::one() + normalized_weight * alpha_distance)
        };
        let weight1 = (combined_weight1 + Number::one()) / Number(2.0);
        let weight2 = Number::one() - weight1;

        Color::new_rgba(
            self.rgba.red * weight1 + other.rgba.red * weight2,
            self.rgba.green * weight1 + other.rgba.green * weight2,
            self.rgba.blue * weight1 + other.rgba.blue * weight2,
            self.alpha() * weight + other.alpha() * (Number::one() - weight),
            ColorFormat::Infer,
        )
    }
}

/// HSLA color functions
impl Color {
    /// The hue in degrees.
    pub fn hue(&self) -> Number {
        self.as_hsla().0
    }

    /// The saturation in percent.
    pub fn saturation(&self) -> Number {
        self.as_hsla().1 * Number(100.0)
    }

    /// The lightness in percent.
    pub fn lightness(&self) -> Number {
        self.as_hsla().2 * Number(100.0)
    }

    /// Hue in degrees, saturation and lightness from 0 to 1, and alpha: the
    /// HSL color's own, else converted from the RGB channels as dart-sass
    /// converts sRGB to HSL (no rounding; a gray's hue is 0).
    pub fn as_hsla(&self) -> (Number, Number, Number, Number) {
        if let Some(h) = &self.hsla {
            return (h.hue, h.saturation, h.luminance, self.alpha());
        }

        let (r, g, b) = self.rgb_channels();
        let (red, green, blue) = (r / 255.0, g / 255.0, b / 255.0);
        let max = red.max(green).max(blue);
        let min = red.min(green).min(blue);
        let delta = max - min;

        let mut hue = if max == min {
            0.0
        } else if max == red {
            60.0 * (green - blue) / delta + 360.0
        } else if max == green {
            60.0 * (blue - red) / delta + 120.0
        } else {
            60.0 * (red - green) / delta + 240.0
        };

        let lightness = (min + max) / 2.0;
        let mut saturation = if lightness == 0.0 || lightness == 1.0 {
            0.0
        } else {
            100.0 * (max - lightness) / lightness.min(1.0 - lightness)
        };
        if saturation < 0.0 {
            hue += 180.0;
            saturation = saturation.abs();
        }
        // A missing hue (no saturation) is 0.
        let hue = if fuzzy_equals(saturation, 0.0) {
            0.0
        } else {
            hue.rem_euclid(360.0)
        };

        (
            Number(hue),
            Number(saturation / 100.0),
            Number(lightness),
            self.alpha(),
        )
    }

    /// `changeHsl`: the HSL channels changed, back in this color's space.
    fn change_hsl(&self, hue: Number, saturation: Number, lightness: Number) -> Self {
        Color::from_hsla(hue, saturation, lightness, self.alpha()).in_space_of(self)
    }

    pub fn adjust_hue(&self, degrees: Number) -> Self {
        let (hue, saturation, luminance, _) = self.as_hsla();
        self.change_hsl(hue + degrees, saturation, luminance)
    }

    pub fn lighten(&self, amount: Number) -> Self {
        let (hue, saturation, luminance, _) = self.as_hsla();
        self.change_hsl(hue, saturation, (luminance + amount).clamp(0.0, 1.0))
    }

    pub fn darken(&self, amount: Number) -> Self {
        let (hue, saturation, luminance, _) = self.as_hsla();
        self.change_hsl(hue, saturation, (luminance - amount).clamp(0.0, 1.0))
    }

    pub fn saturate(&self, amount: Number) -> Self {
        let (hue, saturation, luminance, _) = self.as_hsla();
        self.change_hsl(hue, (saturation + amount).clamp(0.0, 1.0), luminance)
    }

    pub fn desaturate(&self, amount: Number) -> Self {
        let (hue, saturation, luminance, _) = self.as_hsla();
        self.change_hsl(hue, (saturation - amount).clamp(0.0, 1.0), luminance)
    }

    pub fn from_hsla_fn(hue: Number, saturation: Number, luminance: Number, alpha: Number) -> Self {
        let mut color = Self::from_hsla(hue, saturation, luminance, alpha);
        color.format = ColorFormat::Hsl;
        color
    }

    /// An HSL color, its RGB channels converted as dart-sass converts HSL
    /// (no rounding).
    pub fn from_hsla(hue: Number, saturation: Number, lightness: Number, alpha: Number) -> Self {
        let hue = Number(hue.0.rem_euclid(360.0));
        let saturation = saturation.clamp(0.0, 1.0);
        let lightness = lightness.clamp(0.0, 1.0);
        let hsla = Hsl::new(hue, saturation, lightness);

        let scaled_hue = hue.0 / 360.0;
        let scaled_saturation = saturation.0;
        let scaled_lightness = lightness.0;

        let m2 = if scaled_lightness <= 0.5 {
            scaled_lightness * (scaled_saturation + 1.0)
        } else {
            scaled_lightness + scaled_saturation - scaled_lightness * scaled_saturation
        };

        let m1 = scaled_lightness * 2.0 - m2;

        let red = Self::hue_to_rgb(m1, m2, scaled_hue + 1.0 / 3.0) * 255.0;
        let green = Self::hue_to_rgb(m1, m2, scaled_hue) * 255.0;
        let blue = Self::hue_to_rgb(m1, m2, scaled_hue - 1.0 / 3.0) * 255.0;

        Color {
            rgba: Rgb::new(Number(red), Number(green), Number(blue)),
            hsla: Some(hsla),
            alpha: alpha.clamp(0.0, 1.0),
            format: ColorFormat::Infer,
        }
    }

    fn hue_to_rgb(m1: f64, m2: f64, mut hue: f64) -> f64 {
        if hue < 0.0 {
            hue += 1.0;
        }
        if hue > 1.0 {
            hue -= 1.0;
        }

        if hue < 1.0 / 6.0 {
            m1 + (m2 - m1) * hue * 6.0
        } else if hue < 1.0 / 2.0 {
            m2
        } else if hue < 2.0 / 3.0 {
            m1 + (m2 - m1) * (2.0 / 3.0 - hue) * 6.0
        } else {
            m1
        }
    }

    pub fn invert(&self, weight: Number) -> Self {
        if weight.is_zero() {
            return self.clone();
        }

        let (r, g, b) = self.rgb_channels();
        let inverse = Color::new_rgba(
            Number(255.0 - r),
            Number(255.0 - g),
            Number(255.0 - b),
            self.alpha(),
            ColorFormat::Infer,
        );

        inverse.mix(self, weight).in_space_of(self)
    }

    pub fn complement(&self) -> Self {
        let (hue, saturation, luminance, _) = self.as_hsla();
        self.change_hsl(hue + Number(180.0), saturation, luminance)
    }
}

/// Opacity color functions
impl Color {
    pub fn alpha(&self) -> Number {
        if self.alpha > Number::one() {
            self.alpha / Number(255.0)
        } else {
            self.alpha
        }
    }

    /// `changeAlpha`: the same channels and space with another alpha.
    pub fn with_alpha(&self, alpha: Number) -> Self {
        Color {
            rgba: self.rgba.clone(),
            hsla: self.hsla.clone(),
            alpha: alpha.clamp(0.0, 1.0),
            format: ColorFormat::Infer,
        }
    }

    /// Makes a color more opaque.
    /// Takes a color and a number between 0 and 1,
    /// and returns a color with the opacity increased by that amount.
    pub fn fade_in(&self, amount: Number) -> Self {
        self.with_alpha(self.alpha() + amount)
    }

    /// Makes a color more transparent.
    /// Takes a color and a number between 0 and 1,
    /// and returns a color with the opacity decreased by that amount.
    pub fn fade_out(&self, amount: Number) -> Self {
        self.with_alpha(self.alpha() - amount)
    }
}

/// Other color functions
impl Color {
    pub fn to_ie_hex_str(&self) -> String {
        let (r, g, b) = self.rgb_channels();
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            fuzzy_round(self.alpha().0 * 255.0) as u8,
            fuzzy_round(r) as u8,
            fuzzy_round(g) as u8,
            fuzzy_round(b) as u8
        )
    }
}

/// HWB color functions
impl Color {
    pub fn from_hwb(hue: Number, white: Number, black: Number, mut alpha: Number) -> Color {
        let hue = Number(hue.rem_euclid(360.0) / 360.0);
        let mut scaled_white = white.0 / 100.0;
        let mut scaled_black = black.0 / 100.0;
        alpha = alpha.clamp(0.0, 1.0);

        let white_black_sum = scaled_white + scaled_black;

        if white_black_sum > 1.0 {
            scaled_white /= white_black_sum;
            scaled_black /= white_black_sum;
        }

        let factor = 1.0 - scaled_white - scaled_black;

        let to_rgb = |hue: f64| -> Number {
            let channel = Self::hue_to_rgb(0.0, 1.0, hue).mul_add(factor, scaled_white);
            Number(channel * 255.0)
        };

        let red = to_rgb(hue.0 + 1.0 / 3.0);
        let green = to_rgb(hue.0);
        let blue = to_rgb(hue.0 - 1.0 / 3.0);

        Color::new_rgba(red, green, blue, alpha, ColorFormat::Infer)
    }

    pub fn whiteness(&self) -> Number {
        let (r, g, b) = self.rgb_channels();
        Number(r.min(g).min(b) / 255.0)
    }

    pub fn blackness(&self) -> Number {
        let (r, g, b) = self.rgb_channels();
        Number(1.0 - r.max(g).max(b) / 255.0)
    }
}
