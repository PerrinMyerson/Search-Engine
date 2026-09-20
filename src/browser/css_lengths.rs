use super::CssAxis;

pub(super) const DEFAULT_CSS_CELL_WIDTH_PX: usize = 8;
pub(super) const DEFAULT_CSS_CELL_HEIGHT_PX: usize = 12;
pub(super) const CSS_TEXT_CELL_UNITS: usize = 256;
pub(super) const CSS_DEFAULT_VIEWPORT_WIDTH_CELLS: f32 = 100.0;
pub(super) const CSS_DEFAULT_VIEWPORT_HEIGHT_CELLS: f32 = 44.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CssLengthContext {
    pub(super) viewport_width_px: usize,
    pub(super) viewport_height_px: usize,
    cell_width_px: usize,
    cell_height_px: usize,
}

impl CssLengthContext {
    pub(super) fn new(
        viewport_width_cells: usize,
        viewport_height_cells: usize,
        cell_width_px: usize,
        cell_height_px: usize,
    ) -> Self {
        let cell_width_px = cell_width_px.max(1);
        let cell_height_px = cell_height_px.max(1);
        Self {
            viewport_width_px: viewport_width_cells.max(1).saturating_mul(cell_width_px),
            viewport_height_px: viewport_height_cells.max(1).saturating_mul(cell_height_px),
            cell_width_px,
            cell_height_px,
        }
    }

    fn cell_px(self, axis: CssAxis) -> f32 {
        match axis {
            CssAxis::Horizontal => self.cell_width_px as f32,
            CssAxis::Vertical => self.cell_height_px as f32,
        }
    }

    pub(super) fn viewport_width_cells(self) -> usize {
        self.viewport_width_px
            .checked_div(self.cell_width_px)
            .unwrap_or(0)
            .max(1)
    }
}

impl Default for CssLengthContext {
    fn default() -> Self {
        Self::new(
            CSS_DEFAULT_VIEWPORT_WIDTH_CELLS as usize,
            CSS_DEFAULT_VIEWPORT_HEIGHT_CELLS as usize,
            DEFAULT_CSS_CELL_WIDTH_PX,
            DEFAULT_CSS_CELL_HEIGHT_PX,
        )
    }
}

thread_local! {
    static ACTIVE_CSS_LENGTH_CONTEXT: std::cell::Cell<CssLengthContext> =
        std::cell::Cell::new(CssLengthContext::default());
}

fn active_css_length_context() -> CssLengthContext {
    ACTIVE_CSS_LENGTH_CONTEXT.with(std::cell::Cell::get)
}

pub(super) fn with_css_length_context<T>(
    context: CssLengthContext,
    parse: impl FnOnce() -> T,
) -> T {
    ACTIVE_CSS_LENGTH_CONTEXT.with(|active| {
        let previous = active.replace(context);
        let result = parse();
        active.set(previous);
        result
    })
}

pub(super) fn css_axis_cell_px(axis: CssAxis) -> f32 {
    CssLengthContext::default().cell_px(axis)
}

pub(super) fn css_class_visually_hidden(class: &str) -> bool {
    matches!(class, "sr-only" | "visually-hidden" | "screen-reader-only")
}

pub(super) fn parse_css_clip_hides_paint(value: &str) -> bool {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    value.starts_with("rect(0") || value.starts_with("inset(50%") || value.starts_with("inset(100%")
}

pub(super) fn parse_css_length_pixels(value: &str) -> Option<f32> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if value.starts_with('-') {
        return None;
    }
    parse_css_signed_length_pixels(&value)
}

pub(super) fn parse_css_signed_length_pixels(value: &str) -> Option<f32> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if value.contains('%') || value == "auto" || value == "inherit" || value == "initial" {
        return None;
    }
    let context = active_css_length_context();
    let (numeric, multiplier) =
        if let Some((numeric, multiplier)) = parse_css_viewport_unit_length(&value, context) {
            (numeric, multiplier)
        } else if let Some(numeric) = value.strip_suffix("rem") {
            (numeric, 16.0)
        } else if let Some(numeric) = value.strip_suffix("em") {
            (numeric, 16.0)
        } else if let Some(numeric) = value.strip_suffix("ch") {
            (numeric, 8.0)
        } else if let Some(numeric) = value.strip_suffix("px") {
            (numeric, 1.0)
        } else {
            (value.as_str(), 1.0)
        };
    let pixels = numeric.parse::<f32>().ok()? * multiplier;
    pixels.is_finite().then_some(pixels)
}

fn parse_css_viewport_unit_length(value: &str, context: CssLengthContext) -> Option<(&str, f32)> {
    let viewport_width_px = context.viewport_width_px as f32;
    let viewport_height_px = context.viewport_height_px as f32;
    let viewport_min_px = viewport_width_px.min(viewport_height_px);
    let viewport_max_px = viewport_width_px.max(viewport_height_px);
    for (unit, unit_px) in [
        ("svw", viewport_width_px / 100.0),
        ("lvw", viewport_width_px / 100.0),
        ("dvw", viewport_width_px / 100.0),
        ("svh", viewport_height_px / 100.0),
        ("lvh", viewport_height_px / 100.0),
        ("dvh", viewport_height_px / 100.0),
        ("vmin", viewport_min_px / 100.0),
        ("vmax", viewport_max_px / 100.0),
        ("vw", viewport_width_px / 100.0),
        ("vh", viewport_height_px / 100.0),
    ] {
        if let Some(numeric) = value.strip_suffix(unit) {
            return Some((numeric, unit_px));
        }
    }
    None
}

pub(super) fn css_length_cells(value: &str, axis: CssAxis, max_cells: usize) -> Option<usize> {
    let pixels = parse_css_length_pixels(value)?;
    if pixels == 0.0 {
        return Some(0);
    }
    let cells = (pixels / active_css_length_context().cell_px(axis)).ceil() as usize;
    Some(cells.clamp(1, max_cells))
}

pub(super) fn css_length_cells_rounded(
    value: &str,
    axis: CssAxis,
    max_cells: usize,
) -> Option<usize> {
    let pixels = parse_css_length_pixels(value)?;
    if pixels == 0.0 {
        return Some(0);
    }
    let cells = (pixels / active_css_length_context().cell_px(axis)).round() as usize;
    Some(cells.min(max_cells))
}

pub(super) fn css_length_cell_units(value: &str, axis: CssAxis, max_cells: usize) -> Option<usize> {
    let pixels = parse_css_length_pixels(value)?;
    if pixels == 0.0 {
        return Some(0);
    }
    let units = ((pixels / active_css_length_context().cell_px(axis)) * CSS_TEXT_CELL_UNITS as f32)
        .round() as usize;
    Some(units.clamp(1, max_cells.saturating_mul(CSS_TEXT_CELL_UNITS)))
}

#[derive(Debug, Clone, Copy)]
enum CalcValue {
    Number(f32),
    Length(f32),
}

struct CalcParser<'a> {
    rest: &'a str,
}

impl<'a> CalcParser<'a> {
    fn skip_ws(&mut self) {
        self.rest = self.rest.trim_start();
    }

    fn parse_expr(&mut self) -> Option<CalcValue> {
        let mut left = self.parse_term()?;
        loop {
            self.skip_ws();
            let op = match self.rest.chars().next() {
                Some(op @ ('+' | '-')) => op,
                _ => return Some(left),
            };
            self.rest = &self.rest[1..];
            let right = self.parse_term()?;
            left = match (left, right) {
                (CalcValue::Length(a), CalcValue::Length(b)) => {
                    CalcValue::Length(if op == '+' { a + b } else { a - b })
                }
                (CalcValue::Number(a), CalcValue::Number(b)) => {
                    CalcValue::Number(if op == '+' { a + b } else { a - b })
                }
                _ => return None,
            };
        }
    }

    fn parse_term(&mut self) -> Option<CalcValue> {
        let mut left = self.parse_factor()?;
        loop {
            self.skip_ws();
            let op = match self.rest.chars().next() {
                Some(op @ ('*' | '/')) => op,
                _ => return Some(left),
            };
            self.rest = &self.rest[1..];
            let right = self.parse_factor()?;
            left = match (op, left, right) {
                ('*', CalcValue::Length(a), CalcValue::Number(b))
                | ('*', CalcValue::Number(b), CalcValue::Length(a)) => CalcValue::Length(a * b),
                ('*', CalcValue::Number(a), CalcValue::Number(b)) => CalcValue::Number(a * b),
                ('/', _, CalcValue::Number(b)) if b == 0.0 => return None,
                ('/', CalcValue::Length(a), CalcValue::Number(b)) => CalcValue::Length(a / b),
                ('/', CalcValue::Number(a), CalcValue::Number(b)) => CalcValue::Number(a / b),
                _ => return None,
            };
        }
    }

    fn parse_factor(&mut self) -> Option<CalcValue> {
        self.skip_ws();
        let lower = self.rest.to_ascii_lowercase();
        if let Some(stripped) = lower.strip_prefix("calc(") {
            let _ = stripped;
            self.rest = &self.rest["calc".len()..];
            return self.parse_factor();
        }
        if let Some(rest) = self.rest.strip_prefix('(') {
            self.rest = rest;
            let value = self.parse_expr()?;
            self.skip_ws();
            let rest = self.rest.strip_prefix(')')?;
            self.rest = rest;
            return Some(value);
        }
        let token_end = self
            .rest
            .char_indices()
            .find(|(index, ch)| {
                (ch.is_whitespace() || matches!(ch, '+' | '*' | '/' | '(' | ')'))
                    || (*ch == '-' && *index > 0)
            })
            .map(|(index, _)| index)
            .unwrap_or(self.rest.len());
        if token_end == 0 {
            return None;
        }
        let token = &self.rest[..token_end];
        self.rest = &self.rest[token_end..];
        if let Ok(number) = token.parse::<f32>() {
            return Some(CalcValue::Number(number));
        }
        parse_css_signed_length_pixels(token).map(CalcValue::Length)
    }
}

fn eval_css_calc(expr: &str) -> Option<f32> {
    let mut parser = CalcParser { rest: expr.trim() };
    let value = parser.parse_expr()?;
    parser.skip_ws();
    if !parser.rest.is_empty() {
        return None;
    }
    match value {
        CalcValue::Length(px) => px.is_finite().then_some(px),
        CalcValue::Number(_) => None,
    }
}

/// Replace evaluable `calc(...)` expressions in a declaration block with their
/// computed pixel lengths. Unevaluable expressions (percentages, mixed units
/// needing layout context) are left untouched.
pub(super) fn resolve_css_calc_expressions(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if !lower.contains("calc(") {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len());
    let mut offset = 0usize;
    while let Some(found) = value[offset..].to_ascii_lowercase().find("calc(") {
        let start = offset + found;
        out.push_str(&value[offset..start]);
        let expr_start = start + "calc(".len();
        let bytes = value.as_bytes();
        let mut depth = 1usize;
        let mut end = expr_start;
        while end < bytes.len() && depth > 0 {
            match bytes[end] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            end += 1;
        }
        if depth != 0 {
            out.push_str(&value[start..]);
            return out;
        }
        match eval_css_calc(&value[expr_start..end - 1]) {
            Some(px) => out.push_str(&format!("{px:.2}px")),
            None => out.push_str(&value[start..end]),
        }
        offset = end;
    }
    out.push_str(&value[offset..]);
    out
}
