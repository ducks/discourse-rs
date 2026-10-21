use std::io::Write;

use codemap::{CodeMap, Span};

use crate::{
    ast::{CssStmt, MediaQuery, Style, SupportsRule},
    color::{Color, ColorFormat, NAMED_COLORS},
    common::{BinaryOp, Brackets, ListSeparator, QuoteKind},
    error::SassResult,
    selector::{
        Combinator, ComplexSelector, ComplexSelectorComponent, CompoundSelector, Namespace, Pseudo,
        SelectorList, SimpleSelector,
    },
    utils::hex_char_for,
    value::{
        fuzzy_equals, ArgList, CalculationArg, CalculationName, SassCalculation, SassFunction,
        SassMap, SassNumber, Value,
    },
    Options,
};

pub(crate) fn serialize_selector_list(
    list: &SelectorList,
    options: &Options,
    span: Span,
) -> String {
    let map = CodeMap::new();
    let mut serializer = Serializer::new(options, &map, false, span);

    serializer.write_selector_list(list);

    serializer.finish_for_expr()
}

pub(crate) fn serialize_calculation_arg(
    arg: &CalculationArg,
    options: &Options,
    span: Span,
) -> SassResult<String> {
    let map = CodeMap::new();
    let mut serializer = Serializer::new(options, &map, false, span);

    serializer.write_calculation_arg(arg)?;

    Ok(serializer.finish_for_expr())
}

pub(crate) fn serialize_number(
    number: &SassNumber,
    options: &Options,
    span: Span,
) -> SassResult<String> {
    let map = CodeMap::new();
    let mut serializer = Serializer::new(options, &map, false, span);

    serializer.visit_number(number)?;

    Ok(serializer.finish_for_expr())
}

pub(crate) fn serialize_value(val: &Value, options: &Options, span: Span) -> SassResult<String> {
    let map = CodeMap::new();
    let mut serializer = Serializer::new(options, &map, false, span);

    serializer.visit_value(val, span)?;

    Ok(serializer.finish_for_expr())
}

pub(crate) fn inspect_value(val: &Value, options: &Options, span: Span) -> SassResult<String> {
    let map = CodeMap::new();
    let mut serializer = Serializer::new(options, &map, true, span);

    serializer.visit_value(val, span)?;

    Ok(serializer.finish_for_expr())
}

pub(crate) fn inspect_float(number: f64, options: &Options, span: Span) -> String {
    let map = CodeMap::new();
    let mut serializer = Serializer::new(options, &map, true, span);

    serializer.write_float(number);

    serializer.finish_for_expr()
}

pub(crate) fn inspect_map(map: &SassMap, options: &Options, span: Span) -> SassResult<String> {
    let code_map = CodeMap::new();
    let mut serializer = Serializer::new(options, &code_map, true, span);

    serializer.visit_map(map, span)?;

    Ok(serializer.finish_for_expr())
}

pub(crate) fn inspect_function_ref(
    func: &SassFunction,
    options: &Options,
    span: Span,
) -> SassResult<String> {
    let code_map = CodeMap::new();
    let mut serializer = Serializer::new(options, &code_map, true, span);

    serializer.visit_function_ref(func, span)?;

    Ok(serializer.finish_for_expr())
}

pub(crate) fn inspect_number(
    number: &SassNumber,
    options: &Options,
    span: Span,
) -> SassResult<String> {
    let map = CodeMap::new();
    let mut serializer = Serializer::new(options, &map, true, span);

    serializer.visit_number(number)?;

    Ok(serializer.finish_for_expr())
}

pub(crate) struct Serializer<'a> {
    indentation: usize,
    options: &'a Options<'a>,
    inspect: bool,
    indent_width: usize,
    // todo: use this field
    _quote: bool,
    buffer: Vec<u8>,
    map: &'a CodeMap,
    span: Span,
}

impl<'a> Serializer<'a> {
    pub fn new(options: &'a Options<'a>, map: &'a CodeMap, inspect: bool, span: Span) -> Self {
        Self {
            inspect,
            _quote: true,
            indentation: 0,
            indent_width: 2,
            options,
            buffer: Vec::new(),
            map,
            span,
        }
    }

    fn omit_spaces_around_complex_component(&self, component: &ComplexSelectorComponent) -> bool {
        self.options.is_compressed()
            && matches!(component, ComplexSelectorComponent::Combinator(..))
    }

    fn write_pseudo_selector(&mut self, pseudo: &Pseudo) {
        if let Some(sel) = &pseudo.selector {
            if pseudo.name == "not" && sel.is_invisible() {
                return;
            }
        }

        self.buffer.push(b':');

        if !pseudo.is_syntactic_class {
            self.buffer.push(b':');
        }

        self.buffer.extend_from_slice(pseudo.name.as_bytes());

        if pseudo.argument.is_none() && pseudo.selector.is_none() {
            return;
        }

        self.buffer.push(b'(');
        if let Some(arg) = &pseudo.argument {
            self.buffer.extend_from_slice(arg.as_bytes());
            if pseudo.selector.is_some() {
                self.buffer.push(b' ');
            }
        }

        if let Some(sel) = &pseudo.selector {
            self.write_selector_list(sel);
        }

        self.buffer.push(b')');
    }

    fn write_namespace(&mut self, namespace: &Namespace) {
        match namespace {
            Namespace::Empty => self.buffer.push(b'|'),
            Namespace::Asterisk => self.buffer.extend_from_slice(b"*|"),
            Namespace::Other(namespace) => {
                self.buffer.extend_from_slice(namespace.as_bytes());
                self.buffer.push(b'|');
            }
            Namespace::None => {}
        }
    }

    fn write_simple_selector(&mut self, simple: &SimpleSelector) {
        match simple {
            SimpleSelector::Id(name) => {
                self.buffer.push(b'#');
                self.buffer.extend_from_slice(name.as_bytes());
            }
            SimpleSelector::Class(name) => {
                self.buffer.push(b'.');
                self.buffer.extend_from_slice(name.as_bytes());
            }
            SimpleSelector::Placeholder(name) => {
                self.buffer.push(b'%');
                self.buffer.extend_from_slice(name.as_bytes());
            }
            SimpleSelector::Universal(namespace) => {
                self.write_namespace(namespace);
                self.buffer.push(b'*');
            }
            SimpleSelector::Pseudo(pseudo) => self.write_pseudo_selector(pseudo),
            SimpleSelector::Type(name) => {
                self.write_namespace(&name.namespace);
                self.buffer.extend_from_slice(name.ident.as_bytes());
            }
            SimpleSelector::Attribute(attr) => write!(&mut self.buffer, "{}", attr).unwrap(),
            SimpleSelector::Parent(..) => unreachable!("It should not be possible to format `&`."),
        }
    }

    fn write_compound_selector(&mut self, compound: &CompoundSelector) {
        let mut did_write = false;
        for simple in &compound.components {
            if did_write {
                self.write_simple_selector(simple);
            } else {
                let len = self.buffer.len();
                self.write_simple_selector(simple);
                if self.buffer.len() != len {
                    did_write = true;
                }
            }
        }

        // If we emit an empty compound, it's because all of the components got
        // optimized out because they match all selectors, so we just emit the
        // universal selector.
        if !did_write {
            self.buffer.push(b'*');
        }
    }

    fn write_complex_selector_component(&mut self, component: &ComplexSelectorComponent) {
        match component {
            ComplexSelectorComponent::Combinator(Combinator::NextSibling) => self.buffer.push(b'+'),
            ComplexSelectorComponent::Combinator(Combinator::Child) => self.buffer.push(b'>'),
            ComplexSelectorComponent::Combinator(Combinator::FollowingSibling) => {
                self.buffer.push(b'~')
            }
            ComplexSelectorComponent::Compound(compound) => self.write_compound_selector(compound),
        }
    }

    fn write_complex_selector(&mut self, complex: &ComplexSelector) {
        let mut last_component = None;

        for component in &complex.components {
            if let Some(c) = last_component {
                if !self.omit_spaces_around_complex_component(c)
                    && !self.omit_spaces_around_complex_component(component)
                {
                    self.buffer.push(b' ');
                }
            }
            self.write_complex_selector_component(component);
            last_component = Some(component);
        }
    }

    fn write_selector_list(&mut self, list: &SelectorList) {
        let complexes = list.components.iter().filter(|c| !c.is_invisible());

        let mut first = true;

        for complex in complexes {
            if first {
                first = false;
            } else {
                self.buffer.push(b',');
                if complex.line_break {
                    self.write_newline();
                } else {
                    self.write_optional_space();
                }
            }
            self.write_complex_selector(complex);
        }
    }

    fn write_newline(&mut self) {
        if !self.options.is_compressed() {
            self.buffer.push(b'\n');
        }
    }

    fn write_comma_separator(&mut self) {
        self.buffer.push(b',');
        self.write_optional_space();
    }

    fn write_calculation_name(&mut self, name: CalculationName) {
        match name {
            CalculationName::Calc => self.buffer.extend_from_slice(b"calc"),
            CalculationName::Min => self.buffer.extend_from_slice(b"min"),
            CalculationName::Max => self.buffer.extend_from_slice(b"max"),
            CalculationName::Clamp => self.buffer.extend_from_slice(b"clamp"),
        }
    }

    fn visit_calculation(&mut self, calculation: &SassCalculation) -> SassResult<()> {
        self.write_calculation_name(calculation.name);
        self.buffer.push(b'(');

        if let Some((last, slice)) = calculation.args.split_last() {
            for arg in slice {
                self.write_calculation_arg(arg)?;
                self.write_comma_separator();
            }

            self.write_calculation_arg(last)?;
        }

        self.buffer.push(b')');

        Ok(())
    }

    fn write_calculation_arg(&mut self, arg: &CalculationArg) -> SassResult<()> {
        match arg {
            CalculationArg::Number(num) => self.visit_number(num)?,
            CalculationArg::Calculation(calc) => {
                self.visit_calculation(calc)?;
            }
            CalculationArg::String(s) | CalculationArg::Interpolation(s) => {
                self.buffer.extend_from_slice(s.as_bytes());
            }
            CalculationArg::Operation { lhs, op, rhs } => {
                let paren_left = match &**lhs {
                    CalculationArg::Interpolation(..) => true,
                    CalculationArg::Operation { op: op2, .. } => op2.precedence() < op.precedence(),
                    _ => false,
                };

                if paren_left {
                    self.buffer.push(b'(');
                }

                self.write_calculation_arg(lhs)?;

                if paren_left {
                    self.buffer.push(b')');
                }

                let operator_whitespace =
                    !self.options.is_compressed() || matches!(op, BinaryOp::Plus | BinaryOp::Minus);

                if operator_whitespace {
                    self.buffer.push(b' ');
                }

                // todo: avoid allocation with `write_binary_operator` method
                self.buffer.extend_from_slice(op.to_string().as_bytes());

                if operator_whitespace {
                    self.buffer.push(b' ');
                }

                let paren_right = match &**rhs {
                    CalculationArg::Interpolation(..) => true,
                    CalculationArg::Operation { op: op2, .. } => {
                        CalculationArg::parenthesize_calculation_rhs(*op, *op2)
                    }
                    _ => false,
                };

                if paren_right {
                    self.buffer.push(b'(');
                }

                self.write_calculation_arg(rhs)?;

                if paren_right {
                    self.buffer.push(b')');
                }
            }
        }

        Ok(())
    }

    /// discourse-rs: dart-sass's `_writeRgb`: integer channels as such,
    /// any other as a percentage.
    fn write_rgb(&mut self, color: &Color) {
        let is_opaque = fuzzy_equals(color.alpha().0, 1.0);
        let (r, g, b) = color.rgb_channels();

        if is_opaque {
            self.buffer.extend_from_slice(b"rgb(");
        } else {
            self.buffer.extend_from_slice(b"rgba(");
        }

        if r.round() == r && g.round() == g && b.round() == b {
            self.write_float(r);
            self.write_comma_separator();
            self.write_float(g);
            self.write_comma_separator();
            self.write_float(b);
        } else {
            self.write_float(r * 100.0 / 255.0);
            self.buffer.push(b'%');
            self.write_comma_separator();
            self.write_float(g * 100.0 / 255.0);
            self.buffer.push(b'%');
            self.write_comma_separator();
            self.write_float(b * 100.0 / 255.0);
            self.buffer.push(b'%');
        }

        if !is_opaque {
            self.write_comma_separator();
            self.write_float(color.alpha().0);
        }

        self.buffer.push(b')');
    }

    /// discourse-rs: dart-sass's `_writeHsl`.
    fn write_hsl(&mut self, color: &Color) {
        let is_opaque = fuzzy_equals(color.alpha().0, 1.0);
        let (hue, saturation, lightness, alpha) = color.as_hsla();

        if is_opaque {
            self.buffer.extend_from_slice(b"hsl(");
        } else {
            self.buffer.extend_from_slice(b"hsla(");
        }

        self.write_float(hue.0);
        self.write_comma_separator();
        self.write_float(saturation.0 * 100.0);
        self.buffer.push(b'%');
        self.write_comma_separator();
        self.write_float(lightness.0 * 100.0);
        self.buffer.push(b'%');

        if !is_opaque {
            self.write_comma_separator();
            self.write_float(alpha.0);
        }

        self.buffer.push(b')');
    }

    fn write_hex_component(&mut self, channel: u32) {
        debug_assert!(channel < 256);

        self.buffer.push(hex_char_for(channel >> 4) as u8);
        self.buffer.push(hex_char_for(channel & 0xF) as u8);
    }

    fn is_symmetrical_hex(channel: u32) -> bool {
        channel & 0xF == channel >> 4
    }

    /// discourse-rs: `_canUseHex`: every channel fuzzily an integer from 0
    /// to 255.
    fn can_use_hex(color: &Color) -> bool {
        let (r, g, b) = color.rgb_channels();
        [r, g, b].iter().all(|&c| {
            fuzzy_equals(c, c.round())
                && (c > 0.0 || fuzzy_equals(c, 0.0))
                && c < 256.0
                && !fuzzy_equals(c, 256.0)
        })
    }

    /// The color's channels rounded, for a hex code or a name.
    fn rounded_channels(color: &Color) -> [u8; 3] {
        let (r, g, b) = color.rgb_channels();
        [r.round() as u8, g.round() as u8, b.round() as u8]
    }

    fn write_long_hex(&mut self, channels: [u8; 3]) {
        self.buffer.push(b'#');
        for channel in channels {
            self.write_hex_component(u32::from(channel));
        }
    }

    /// discourse-rs: dart-sass 1.104's `_writeLegacyColor`. Compressed, the
    /// shortest of a name, a short or long hex code, `rgb()` or `hsl()`;
    /// expanded, an HSL color as `hsl()`, a color written in the source as
    /// written (`rgb()` from `rgb()`), else a name, a long hex code or
    /// `rgb()`.
    pub fn visit_color(&mut self, color: &Color) {
        let opaque = fuzzy_equals(color.alpha().0, 1.0);

        // Out-of-gamut colors can only be written as HSL.
        if !color.is_in_gamut() && !self.inspect {
            self.write_hsl(color);
            return;
        }

        if self.options.is_compressed() {
            if opaque && Self::can_use_hex(color) {
                let channels = Self::rounded_channels(color);
                let short = channels.iter().all(|&c| Self::is_symmetrical_hex(u32::from(c)));
                match NAMED_COLORS.get_by_rgba(channels) {
                    Some(name) if name.len() <= if short { 4 } else { 7 } => {
                        self.buffer.extend_from_slice(name.as_bytes());
                    }
                    _ if short => {
                        self.buffer.push(b'#');
                        for channel in channels {
                            self.buffer.push(hex_char_for(u32::from(channel) & 0xF) as u8);
                        }
                    }
                    _ => self.write_long_hex(channels),
                }
                return;
            }

            let start = self.buffer.len();
            self.write_rgb(color);
            let rgb = self.buffer.split_off(start);
            self.write_hsl(color);
            let hsl = self.buffer.split_off(start);
            // Two more characters for HSL's `%`s.
            if rgb.len() <= hsl.len() + 2 {
                self.buffer.extend_from_slice(&rgb);
            } else {
                self.buffer.extend_from_slice(&hsl);
            }
            return;
        }

        if color.is_hsl_space() {
            self.write_hsl(color);
            return;
        }

        match &color.format {
            ColorFormat::Rgb => {
                self.write_rgb(color);
                return;
            }
            ColorFormat::Literal(text) => {
                self.buffer.extend_from_slice(text.as_bytes());
                return;
            }
            ColorFormat::Hsl | ColorFormat::Infer => {}
        }

        // Generated transparent colors are always written as rgba().
        if opaque {
            if Self::can_use_hex(color) {
                let channels = Self::rounded_channels(color);
                if let Some(name) = NAMED_COLORS.get_by_rgba(channels) {
                    self.buffer.extend_from_slice(name.as_bytes());
                } else {
                    self.write_long_hex(channels);
                }
                return;
            }
        }

        self.write_rgb(color);
    }

    fn write_media_query(&mut self, query: &MediaQuery) {
        if let Some(modifier) = &query.modifier {
            self.buffer.extend_from_slice(modifier.as_bytes());
            self.buffer.push(b' ');
        }

        if let Some(media_type) = &query.media_type {
            self.buffer.extend_from_slice(media_type.as_bytes());

            if !query.conditions.is_empty() {
                self.buffer.extend_from_slice(b" and ");
            }
        }

        if query.conditions.len() == 1 && query.conditions.first().unwrap().starts_with("(not ") {
            self.buffer.extend_from_slice(b"not ");
            let condition = query.conditions.first().unwrap();
            self.buffer
                .extend_from_slice(condition["(not ".len()..condition.len() - 1].as_bytes());
        } else {
            // discourse-rs: compressed output joins conditions with `and `
            // (dart-sass).
            let operator = match (query.conjunction, self.options.is_compressed()) {
                (true, true) => "and ",
                (true, false) => " and ",
                (false, true) => "or ",
                (false, false) => " or ",
            };
            self.buffer
                .extend_from_slice(query.conditions.join(operator).as_bytes());
        }
    }

    pub fn visit_number(&mut self, number: &SassNumber) -> SassResult<()> {
        if let Some(as_slash) = &number.as_slash {
            self.visit_number(&as_slash.0)?;
            self.buffer.push(b'/');
            self.visit_number(&as_slash.1)?;
            return Ok(());
        }

        if !self.inspect && number.unit.is_complex() {
            return Err((
                format!(
                    "{} isn't a valid CSS value.",
                    inspect_number(number, self.options, self.span)?
                ),
                self.span,
            )
                .into());
        }

        self.write_float(number.num.0);
        write!(&mut self.buffer, "{}", number.unit)?;

        Ok(())
    }

    /// discourse-rs: dart-sass's `_writeNumber`. Integers as such, a number
    /// short enough written directly (a compressed `0.5` as `.5`, though
    /// not `-0.5`), any other rounded to ten decimals on its shortest
    /// decimal form, half up by the next digit (`_writeRounded`); inspect
    /// writes full precision.
    fn write_float(&mut self, float: f64) {
        if float.is_infinite() && float.is_sign_negative() {
            self.buffer.extend_from_slice(b"-Infinity");
            return;
        } else if float.is_infinite() {
            self.buffer.extend_from_slice(b"Infinity");
            return;
        }

        if float == 0.0 && float.is_sign_negative() {
            self.buffer.extend_from_slice(b"-0");
            return;
        }

        // `_asInt`: exactly an integer, or fuzzily one under inspect.
        let rounded = float.round();
        let is_int = if self.inspect {
            fuzzy_equals(float, rounded)
        } else {
            rounded == float
        };
        if is_int {
            write!(&mut self.buffer, "{}", rounded as i128).unwrap();
            return;
        }

        // Rust writes the shortest round-trip digits without an exponent, as
        // dart-sass's `_removeExponent(number.toString())` does.
        let mut text = format!("{float}");

        if self.inspect {
            self.buffer.extend_from_slice(text.as_bytes());
            return;
        }

        if text.len() < 10 + 2 {
            if self.options.is_compressed() && text.starts_with('0') {
                text.remove(0);
            }
            self.buffer.extend_from_slice(text.as_bytes());
            return;
        }

        self.write_rounded(&text);
    }

    /// discourse-rs: dart-sass's `_writeRounded`.
    fn write_rounded(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let mut digits = vec![0u8; bytes.len() + 1];
        let mut digits_index = 1;
        let mut text_index = 0;
        let negative = bytes[0] == b'-';
        if negative {
            text_index += 1;
        }
        loop {
            if text_index == bytes.len() {
                self.buffer.extend_from_slice(bytes);
                return;
            }
            let c = bytes[text_index];
            text_index += 1;
            if c == b'.' {
                break;
            }
            digits[digits_index] = c - b'0';
            digits_index += 1;
        }
        let first_fractional_digit = digits_index;

        let index_after_precision = text_index + 10;
        if index_after_precision >= bytes.len() {
            self.buffer.extend_from_slice(bytes);
            return;
        }

        while text_index < index_after_precision {
            digits[digits_index] = bytes[text_index] - b'0';
            digits_index += 1;
            text_index += 1;
        }

        if bytes[text_index] - b'0' >= 5 {
            loop {
                digits[digits_index - 1] += 1;
                if digits[digits_index - 1] != 10 {
                    break;
                }
                digits_index -= 1;
            }
        }

        while digits_index < first_fractional_digit {
            digits[digits_index] = 0;
            digits_index += 1;
        }
        while digits_index > first_fractional_digit && digits[digits_index - 1] == 0 {
            digits_index -= 1;
        }

        if digits_index == 2 && digits[0] == 0 && digits[1] == 0 {
            self.buffer.push(b'0');
            return;
        }

        if negative {
            self.buffer.push(b'-');
        }

        let mut written_index = 0;
        if digits[0] == 0 {
            written_index += 1;
            if self.options.is_compressed() && digits[1] == 0 {
                written_index += 1;
            }
        }
        while written_index < first_fractional_digit {
            self.buffer.push(b'0' + digits[written_index]);
            written_index += 1;
        }

        if digits_index > first_fractional_digit {
            self.buffer.push(b'.');
            while written_index < digits_index {
                self.buffer.push(b'0' + digits[written_index]);
                written_index += 1;
            }
        }
    }

    pub fn visit_group(
        &mut self,
        stmt: CssStmt,
        prev_was_group_end: bool,
        prev_requires_semicolon: bool,
    ) -> SassResult<()> {
        if prev_requires_semicolon {
            self.buffer.push(b';');
        }

        if !self.buffer.is_empty() {
            self.write_optional_newline();
        }

        if prev_was_group_end && !self.buffer.is_empty() {
            self.write_optional_newline();
        }

        self.visit_stmt(stmt)?;

        Ok(())
    }

    fn finish_for_expr(self) -> String {
        // SAFETY: todo
        unsafe { String::from_utf8_unchecked(self.buffer) }
    }

    pub fn finish(mut self, prev_requires_semicolon: bool) -> String {
        let is_not_ascii = self.buffer.iter().any(|&c| !c.is_ascii());

        if prev_requires_semicolon {
            self.buffer.push(b';');
        }

        if !self.buffer.is_empty() {
            self.write_optional_newline();
        }

        // SAFETY: todo
        let mut as_string = unsafe { String::from_utf8_unchecked(self.buffer) };

        if is_not_ascii && self.options.is_compressed() && self.options.allows_charset {
            as_string.insert(0, '\u{FEFF}');
        } else if is_not_ascii && self.options.allows_charset {
            as_string.insert_str(0, "@charset \"UTF-8\";\n");
        }

        as_string
    }

    fn write_indentation(&mut self) {
        if self.options.is_compressed() {
            return;
        }

        self.buffer.reserve(self.indentation);
        for _ in 0..self.indentation {
            self.buffer.push(b' ');
        }
    }

    fn write_list_separator(&mut self, sep: ListSeparator) {
        match (sep, self.options.is_compressed()) {
            (ListSeparator::Space | ListSeparator::Undecided, _) => self.buffer.push(b' '),
            (ListSeparator::Comma, true) => self.buffer.push(b','),
            (ListSeparator::Comma, false) => self.buffer.extend_from_slice(b", "),
            (ListSeparator::Slash, true) => self.buffer.push(b'/'),
            (ListSeparator::Slash, false) => self.buffer.extend_from_slice(b" / "),
        }
    }

    fn elem_needs_parens(sep: ListSeparator, elem: &Value) -> bool {
        match elem {
            Value::List(elems, sep2, brackets) => {
                if elems.len() < 2 {
                    return false;
                }

                if *brackets == Brackets::Bracketed {
                    return false;
                }

                match sep {
                    ListSeparator::Comma => *sep2 == ListSeparator::Comma,
                    ListSeparator::Slash => {
                        *sep2 == ListSeparator::Comma || *sep2 == ListSeparator::Slash
                    }
                    _ => *sep2 != ListSeparator::Undecided,
                }
            }
            _ => false,
        }
    }

    fn visit_list(
        &mut self,
        list_elems: &[Value],
        sep: ListSeparator,
        brackets: Brackets,
        span: Span,
    ) -> SassResult<()> {
        if brackets == Brackets::Bracketed {
            self.buffer.push(b'[');
        } else if list_elems.is_empty() {
            if !self.inspect {
                return Err(("() isn't a valid CSS value.", span).into());
            }

            self.buffer.extend_from_slice(b"()");
            return Ok(());
        }

        let is_singleton = self.inspect
            && list_elems.len() == 1
            && (sep == ListSeparator::Comma || sep == ListSeparator::Slash);

        if is_singleton && brackets != Brackets::Bracketed {
            self.buffer.push(b'(');
        }

        let (mut x, mut y);
        let elems: &mut dyn Iterator<Item = &Value> = if self.inspect {
            x = list_elems.iter();
            &mut x
        } else {
            y = list_elems.iter().filter(|elem| !elem.is_blank());
            &mut y
        };

        let mut elems = elems.peekable();

        while let Some(elem) = elems.next() {
            if self.inspect {
                let needs_parens = Self::elem_needs_parens(sep, elem);
                if needs_parens {
                    self.buffer.push(b'(');
                }

                self.visit_value(elem, span)?;

                if needs_parens {
                    self.buffer.push(b')');
                }
            } else {
                self.visit_value(elem, span)?;
            }

            if elems.peek().is_some() {
                self.write_list_separator(sep);
            }
        }

        if is_singleton {
            match sep {
                ListSeparator::Comma => self.buffer.push(b','),
                ListSeparator::Slash => self.buffer.push(b'/'),
                _ => unreachable!(),
            }

            if brackets != Brackets::Bracketed {
                self.buffer.push(b')');
            }
        }

        if brackets == Brackets::Bracketed {
            self.buffer.push(b']');
        }

        Ok(())
    }

    fn write_map_element(&mut self, value: &Value, span: Span) -> SassResult<()> {
        let needs_parens = matches!(value, Value::List(_, ListSeparator::Comma, Brackets::None));

        if needs_parens {
            self.buffer.push(b'(');
        }

        self.visit_value(value, span)?;

        if needs_parens {
            self.buffer.push(b')');
        }

        Ok(())
    }

    fn visit_map(&mut self, map: &SassMap, span: Span) -> SassResult<()> {
        if !self.inspect {
            return Err((
                format!(
                    "{} isn't a valid CSS value.",
                    inspect_map(map, self.options, span)?
                ),
                span,
            )
                .into());
        }

        self.buffer.push(b'(');

        let mut elems = map.iter().peekable();

        while let Some((k, v)) = elems.next() {
            self.write_map_element(&k.node, k.span)?;
            self.buffer.extend_from_slice(b": ");
            self.write_map_element(v, k.span)?;
            if elems.peek().is_some() {
                self.buffer.extend_from_slice(b", ");
            }
        }

        self.buffer.push(b')');

        Ok(())
    }

    fn visit_unquoted_string(&mut self, string: &str) {
        let mut after_newline = false;
        self.buffer.reserve(string.len());

        for c in string.bytes() {
            match c {
                b'\n' => {
                    self.buffer.push(b' ');
                    after_newline = true;
                }
                b' ' => {
                    if !after_newline {
                        self.buffer.push(b' ');
                    }
                }
                _ => {
                    self.buffer.push(c);
                    after_newline = false;
                }
            }
        }
    }

    fn visit_quoted_string(&mut self, force_double_quote: bool, string: &str) {
        let mut has_single_quote = false;
        let mut has_double_quote = false;

        let mut buffer = Vec::new();

        if force_double_quote {
            buffer.push(b'"');
        }
        let mut iter = string.as_bytes().iter().copied().peekable();
        while let Some(c) = iter.next() {
            match c {
                b'\'' => {
                    if force_double_quote {
                        buffer.push(b'\'');
                    } else if has_double_quote {
                        self.visit_quoted_string(true, string);
                        return;
                    } else {
                        has_single_quote = true;
                        buffer.push(b'\'');
                    }
                }
                b'"' => {
                    if force_double_quote {
                        buffer.push(b'\\');
                        buffer.push(b'"');
                    } else if has_single_quote {
                        self.visit_quoted_string(true, string);
                        return;
                    } else {
                        has_double_quote = true;
                        buffer.push(b'"');
                    }
                }
                b'\x00'..=b'\x08' | b'\x0A'..=b'\x1F' => {
                    buffer.push(b'\\');
                    if c as u32 > 0xF {
                        buffer.push(hex_char_for(c as u32 >> 4) as u8);
                    }
                    buffer.push(hex_char_for(c as u32 & 0xF) as u8);

                    let next = match iter.peek() {
                        Some(v) => *v,
                        None => break,
                    };

                    if next.is_ascii_hexdigit() || next == b' ' || next == b'\t' {
                        buffer.push(b' ');
                    }
                }
                b'\\' => {
                    buffer.push(b'\\');
                    buffer.push(b'\\');
                }
                _ => buffer.push(c),
            }
        }

        if force_double_quote {
            buffer.push(b'"');
            self.buffer.extend_from_slice(&buffer);
        } else {
            let quote = if has_double_quote { b'\'' } else { b'"' };
            self.buffer.push(quote);
            self.buffer.extend_from_slice(&buffer);
            self.buffer.push(quote);
        }
    }

    fn visit_function_ref(&mut self, func: &SassFunction, span: Span) -> SassResult<()> {
        if !self.inspect {
            return Err((
                format!(
                    "{} isn't a valid CSS value.",
                    inspect_function_ref(func, self.options, span)?
                ),
                span,
            )
                .into());
        }

        self.buffer.extend_from_slice(b"get-function(");
        self.visit_quoted_string(false, func.name().as_str());
        self.buffer.push(b')');

        Ok(())
    }

    fn visit_arglist(&mut self, arglist: &ArgList, span: Span) -> SassResult<()> {
        self.visit_list(&arglist.elems, ListSeparator::Comma, Brackets::None, span)
    }

    fn visit_value(&mut self, value: &Value, span: Span) -> SassResult<()> {
        match value {
            Value::Dimension(num) => self.visit_number(num)?,
            Value::Color(color) => self.visit_color(color),
            Value::Calculation(calc) => self.visit_calculation(calc)?,
            Value::List(elems, sep, brackets) => self.visit_list(elems, *sep, *brackets, span)?,
            Value::True => self.buffer.extend_from_slice(b"true"),
            Value::False => self.buffer.extend_from_slice(b"false"),
            Value::Null => {
                if self.inspect {
                    self.buffer.extend_from_slice(b"null")
                }
            }
            Value::Map(map) => self.visit_map(map, span)?,
            Value::FunctionRef(func) => self.visit_function_ref(func, span)?,
            Value::String(s, QuoteKind::Quoted) => self.visit_quoted_string(false, s),
            Value::String(s, QuoteKind::None) => self.visit_unquoted_string(s),
            Value::ArgList(arglist) => self.visit_arglist(arglist, span)?,
        }

        Ok(())
    }

    fn write_style(&mut self, style: Style) -> SassResult<()> {
        if !self.options.is_compressed() {
            self.write_indentation();
        }

        self.buffer
            .extend_from_slice(style.property.resolve_ref().as_bytes());
        self.buffer.push(b':');

        // todo: _writeFoldedValue and _writeReindentedValue
        if !style.declared_as_custom_property && !self.options.is_compressed() {
            self.buffer.push(b' ');
        }

        self.visit_value(&style.value.node, style.value.span)?;

        Ok(())
    }

    fn write_import(&mut self, import: &str, modifiers: Option<String>) -> SassResult<()> {
        self.write_indentation();
        self.buffer.extend_from_slice(b"@import ");
        write!(&mut self.buffer, "{}", import)?;

        if let Some(modifiers) = modifiers {
            self.buffer.push(b' ');
            self.buffer.extend_from_slice(modifiers.as_bytes());
        }

        Ok(())
    }

    fn write_comment(&mut self, comment: &str, span: Span) -> SassResult<()> {
        if self.options.is_compressed() && !comment.starts_with("/*!") {
            return Ok(());
        }

        self.write_indentation();
        let col = self.map.look_up_pos(span.low()).position.column;
        let mut lines = comment.lines();

        if let Some(line) = lines.next() {
            self.buffer.extend_from_slice(line.trim_start().as_bytes());
        }

        let lines = lines
            .map(|line| {
                let diff = (line.len() - line.trim_start().len()).saturating_sub(col);
                format!("{}{}", " ".repeat(diff), line.trim_start())
            })
            .collect::<Vec<String>>()
            .join("\n");

        if !lines.is_empty() {
            write!(&mut self.buffer, "\n{}", lines)?;
        }

        Ok(())
    }

    pub fn requires_semicolon(stmt: &CssStmt) -> bool {
        match stmt {
            CssStmt::Style(_) | CssStmt::Import(_, _) => true,
            CssStmt::UnknownAtRule(rule, _) => !rule.has_body,
            _ => false,
        }
    }

    fn write_children(&mut self, mut children: Vec<CssStmt>) -> SassResult<()> {
        if self.options.is_compressed() {
            self.buffer.push(b'{');
        } else {
            self.buffer.extend_from_slice(b" {\n");
        }

        self.indentation += self.indent_width;

        let last = children.pop();

        for child in children {
            let needs_semicolon = Self::requires_semicolon(&child);
            let did_write = self.visit_stmt(child)?;

            if !did_write {
                continue;
            }

            if needs_semicolon {
                self.buffer.push(b';');
            }

            self.write_optional_newline();
        }

        if let Some(last) = last {
            let needs_semicolon = Self::requires_semicolon(&last);
            let did_write = self.visit_stmt(last)?;

            if did_write {
                if needs_semicolon && !self.options.is_compressed() {
                    self.buffer.push(b';');
                }

                self.write_optional_newline();
            }
        }

        self.indentation -= self.indent_width;

        if self.options.is_compressed() {
            self.buffer.push(b'}');
        } else {
            self.write_indentation();
            self.buffer.extend_from_slice(b"}");
        }

        Ok(())
    }

    fn write_optional_space(&mut self) {
        if !self.options.is_compressed() {
            self.buffer.push(b' ');
        }
    }

    fn write_optional_newline(&mut self) {
        if !self.options.is_compressed() {
            self.buffer.push(b'\n');
        }
    }

    fn write_supports_rule(&mut self, supports_rule: SupportsRule) -> SassResult<()> {
        self.write_indentation();
        self.buffer.extend_from_slice(b"@supports");

        if !supports_rule.params.is_empty() {
            // discourse-rs: compressed output leaves out the space before a
            // parenthesized condition (dart-sass).
            if !(self.options.is_compressed() && supports_rule.params.starts_with('(')) {
                self.buffer.push(b' ');
            }
            self.buffer
                .extend_from_slice(supports_rule.params.as_bytes());
        }

        self.write_children(supports_rule.body)?;

        Ok(())
    }

    /// Returns whether or not text was written
    fn visit_stmt(&mut self, stmt: CssStmt) -> SassResult<bool> {
        if stmt.is_invisible() {
            return Ok(false);
        }

        match stmt {
            CssStmt::RuleSet { selector, body, .. } => {
                self.write_indentation();
                self.write_selector_list(&selector.as_selector_list());

                self.write_children(body)?;
            }
            CssStmt::Media(media_rule, ..) => {
                self.write_indentation();
                self.buffer.extend_from_slice(b"@media");
                // discourse-rs: compressed output leaves out the space before a
                // query that starts with a condition (dart-sass).
                let spaced = media_rule.query.first().is_none_or(|q| {
                    q.modifier.is_some()
                        || q.media_type.is_some()
                        || (q.conditions.len() == 1 && q.conditions[0].starts_with("(not "))
                });
                if !self.options.is_compressed() || spaced {
                    self.buffer.push(b' ');
                }

                if let Some((last, rest)) = media_rule.query.split_last() {
                    for query in rest {
                        self.write_media_query(query);

                        self.buffer.push(b',');

                        self.write_optional_space();
                    }

                    self.write_media_query(last);
                }

                self.write_children(media_rule.body)?;
            }
            CssStmt::UnknownAtRule(unknown_at_rule, ..) => {
                self.write_indentation();
                self.buffer.push(b'@');
                self.buffer
                    .extend_from_slice(unknown_at_rule.name.as_bytes());

                if !unknown_at_rule.params.is_empty() {
                    write!(&mut self.buffer, " {}", unknown_at_rule.params)?;
                }

                if !unknown_at_rule.has_body {
                    debug_assert!(unknown_at_rule.body.is_empty());
                    return Ok(true);
                } else if unknown_at_rule.body.iter().all(CssStmt::is_invisible) {
                    self.buffer.extend_from_slice(b" {}");
                    return Ok(true);
                }

                self.write_children(unknown_at_rule.body)?;
            }
            CssStmt::Style(style) => self.write_style(style)?,
            CssStmt::Comment(comment, span) => self.write_comment(&comment, span)?,
            CssStmt::KeyframesRuleSet(keyframes_rule_set) => {
                self.write_indentation();
                // todo: i bet we can do something like write_with_separator to avoid extra allocation
                let selector = keyframes_rule_set
                    .selector
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>()
                    .join(", ");

                self.buffer.extend_from_slice(selector.as_bytes());

                self.write_children(keyframes_rule_set.body)?;
            }
            CssStmt::Import(import, modifier) => self.write_import(&import, modifier)?,
            CssStmt::Supports(supports_rule, _) => self.write_supports_rule(supports_rule)?,
        }

        Ok(true)
    }
}
