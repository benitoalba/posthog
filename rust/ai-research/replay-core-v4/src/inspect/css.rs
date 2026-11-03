//! Image URLs in CSS: the `url()`s of an image property, or of a custom property that reaches one through `var()`.
//! [`rewrite_css`] is the one reader of them. The inspector runs it to choose the fetch jobs, and the writer runs it to
//! replace each URL with what the block stores.

use url::Url;

use super::assets::Replacement;
use crate::scan::images::is_data_url;
use crate::url_policy::{
    is_fetched_image_type, is_protocol_relative, is_remote, try_canonicalize, MAX_URL_LEN,
};

/// Far more than a real `image-set()` lists. A longer set keeps all its candidates.
const MAX_IMAGE_SET_CANDIDATES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssKind {
    /// Declarations or rules: a `style` attribute, `<style>` text, `_cssText` or a CSSOM rule.
    Text,
    /// The value of one property that can hold an image, as a style diff or a CSSOM declaration sets it.
    Value,
}

/// An image URL in CSS, after its CSS escapes, and absolute when the page wrote it root-relative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CssImage<'a> {
    pub url: &'a str,
    /// A renderer can load it. A candidate of an `image-set()` that the recorder does not cut to one is not shown.
    pub shown: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CssRewriteCounts {
    pub image_sets: u64,
    pub relative_urls: u64,
    pub unresolved: u64,
}

/// The origin that a root-relative URL on `page_url` resolves against, as `https` because the fetcher loads nothing else.
pub fn page_base(page_url: &str) -> Option<Url> {
    if page_url.len() > MAX_URL_LEN {
        return None;
    }
    let mut base = Url::parse(page_url).ok()?;
    if base.scheme() != "https" {
        base.set_scheme("https").ok()?;
    }
    base.set_path("/");
    base.set_query(None);
    base.set_fragment(None);
    Some(base)
}

/// Only a root-relative URL, as `/a.png`, resolves. It needs the page's origin and nothing else, so no part of a page
/// path, which can hold a token, reaches a fetch, and a `<base>` or a style sheet's own URL cannot change it.
fn resolve(base: Option<&Url>, source: &str) -> Option<String> {
    is_root_relative(source)
        .then(|| base?.join(source).ok())
        .flatten()
        .map(String::from)
}

pub fn can_hold_image(property: &str) -> bool {
    let property = property.trim();
    property.starts_with("--") || is_image_property(property)
}

/// Whether `css` holds a `url(` or an `image-set(`, so that most CSS strings need no slot.
pub fn may_hold_image(css: &str) -> bool {
    let bytes = css.as_bytes();
    let ends_with = |end: usize, name: &[u8]| {
        end >= name.len() && bytes[end - name.len()..end].eq_ignore_ascii_case(name)
    };
    memchr::memchr_iter(b'(', bytes)
        .any(|open| ends_with(open, b"url") || ends_with(open, b"image-set"))
}

/// `css` with each image URL replaced by what `asset` returns for it, and each `image-set()` cut to the one candidate
/// a renderer can load. `None` when nothing changed. Only whole `url()` and `image-set()` functions in the value of an
/// image property change, so a class name such as `bg-[url(/a.png)]`, a quoted string or a comment keeps its text.
pub fn rewrite_css(
    css: &str,
    kind: CssKind,
    base: Option<&Url>,
    asset: &mut impl FnMut(CssImage<'_>) -> Option<Replacement>,
    counts: &mut CssRewriteCounts,
) -> Option<String> {
    scan_css(css, kind, base, asset, counts, true)
}

/// Calls `found` with each image URL in `css`, as [`rewrite_css`] finds them, and builds no rewritten text.
pub fn css_image_urls(
    css: &str,
    kind: CssKind,
    base: Option<&Url>,
    found: &mut impl FnMut(CssImage<'_>),
) {
    let mut asset = |image: CssImage<'_>| {
        found(image);
        None
    };
    scan_css(
        css,
        kind,
        base,
        &mut asset,
        &mut CssRewriteCounts::default(),
        false,
    );
}

fn scan_css(
    css: &str,
    kind: CssKind,
    base: Option<&Url>,
    asset: &mut impl FnMut(CssImage<'_>) -> Option<Replacement>,
    counts: &mut CssRewriteCounts,
    write: bool,
) -> Option<String> {
    let mut out = Splice::new(css);
    let mut rewrite = Rewrite {
        base,
        asset,
        counts,
        write,
    };
    match kind {
        CssKind::Text => {
            for (start, end) in image_value_ranges(css) {
                rewrite.value(css, start, end, &mut out);
            }
        }
        CssKind::Value => rewrite.value(css, 0, css.len(), &mut out),
    }
    out.finish()
}

struct Splice<'s> {
    text: &'s str,
    copied_to: usize,
    out: Option<String>,
}

impl<'s> Splice<'s> {
    fn new(text: &'s str) -> Self {
        Self {
            text,
            copied_to: 0,
            out: None,
        }
    }

    fn replace(&mut self, start: usize, end: usize, with: &str) {
        let out = self
            .out
            .get_or_insert_with(|| String::with_capacity(self.text.len()));
        out.push_str(&self.text[self.copied_to..start]);
        out.push_str(with);
        self.copied_to = end;
    }

    fn finish(self) -> Option<String> {
        let mut out = self.out?;
        out.push_str(&self.text[self.copied_to..]);
        Some(out)
    }
}

struct Rewrite<'a, F> {
    base: Option<&'a Url>,
    asset: &'a mut F,
    counts: &'a mut CssRewriteCounts,
    /// Builds the replacement text. Off, a page cannot make the inspector hold a rewritten copy of its CSS.
    write: bool,
}

impl<F: FnMut(CssImage<'_>) -> Option<Replacement>> Rewrite<'_, F> {
    /// Rewrites the functions in `css[start..end]`, the value of one image property.
    fn value(&mut self, css: &str, start: usize, end: usize, out: &mut Splice<'_>) {
        let value = &css[..end];
        let bytes = value.as_bytes();
        let mut position = start;
        while position < end {
            if starts_with(bytes, position, b"/*") {
                position = skip_comment(bytes, position).unwrap_or(end);
                continue;
            }
            if bytes[position] == b'\\' {
                position = skip_escape(bytes, position);
                continue;
            }
            if matches!(bytes[position], b'\'' | b'"') {
                position = skip_quoted(bytes, position).unwrap_or(end);
                continue;
            }
            let image_set = if function_at(bytes, position, b"image-set") {
                Some(b"image-set".len())
            } else if function_at(bytes, position, b"-webkit-image-set") {
                Some(b"-webkit-image-set".len())
            } else {
                None
            };
            if let Some(name_len) = image_set {
                let Some(set_end) = matching_paren(bytes, position + name_len) else {
                    return;
                };
                if let Some(text) = self.image_set(&value[position..set_end], name_len) {
                    out.replace(position, set_end, &text);
                }
                position = set_end;
                continue;
            }
            if function_at(bytes, position, b"url") {
                let (resume_at, source) = scan_url_function(value, position);
                if let Some(text) = source.and_then(|source| self.url(&source, true)) {
                    out.replace(position, resume_at, &text);
                }
                position = resume_at;
                continue;
            }
            position += 1;
        }
    }

    /// The `url()` that replaces one with this source. `None` keeps the function as the page wrote it.
    fn url(&mut self, source: &str, shown: bool) -> Option<String> {
        // A fragment names an element of the page, such as an SVG mask, and loads nothing.
        if source.starts_with('#') {
            return None;
        }
        if is_root_relative(source) {
            let Some(absolute) = resolve(self.base, source) else {
                self.counts.unresolved += 1;
                return None;
            };
            self.counts.relative_urls += 1;
            let replaced = (self.asset)(CssImage {
                url: &absolute,
                shown,
            });
            return self
                .write
                .then(|| url_function(&replaced.map_or(absolute, |r| r.text)));
        }
        if is_remote(source) || is_data_url(source) {
            let replaced = (self.asset)(CssImage { url: source, shown });
            return replaced
                .filter(|_| self.write)
                .map(|r| url_function(&r.text));
        }
        None
    }

    /// A renderer picks the candidate for its pixel ratio and fails offline on any the fetcher did not take, so the set
    /// keeps one: the highest resolution of a stored type. Fetching one candidate per set limits the load on the image's
    /// host. A set with no such candidate, or one the recorder does not read in full, keeps every candidate, each URL
    /// replaced where it is.
    fn image_set(&mut self, function: &str, name_len: usize) -> Option<String> {
        let contents_start = name_len + 1;
        let contents = &function[contents_start..function.len() - 1];
        let mut candidates = Vec::new();
        let mut readable = true;
        split_image_set(contents, &mut |_, text| match image_set_candidate(text)
            .filter(|_| candidates.len() < MAX_IMAGE_SET_CANDIDATES)
        {
            Some(candidate) => candidates.push(candidate),
            None => readable = false,
        })?;
        let base = self.base;
        let loads = |source: &str| {
            is_data_url(source)
                || (is_remote(source) && try_canonicalize(source).is_ok())
                || (base.is_some() && is_root_relative(source))
        };
        let kept = candidates
            .iter()
            .enumerate()
            .filter(|_| readable)
            .filter_map(|(index, candidate)| {
                candidate.source.as_deref().filter(|source| loads(source))?;
                Some((index, candidate.resolution_dppx?))
            })
            .fold(None, |best: Option<(usize, f64)>, candidate| {
                if best.is_none_or(|best| candidate.1 > best.1) {
                    Some(candidate)
                } else {
                    best
                }
            });
        let Some((index, _)) = kept else {
            return self.each_image_set_candidate(function, contents_start);
        };
        let kept = &candidates[index];
        let source = kept.source.as_deref().unwrap_or_default();
        let url = self.url(source, true);
        if candidates.len() > 1 {
            self.counts.image_sets += 1;
        }
        if !self.write || (candidates.len() == 1 && url.is_none()) {
            return None;
        }
        let candidate = match url {
            Some(url) => format!("{url} {}", kept.descriptors.trim())
                .trim_end()
                .to_owned(),
            None => kept.text.to_owned(),
        };
        Some(format!("{}({candidate})", &function[..name_len]))
    }

    fn each_image_set_candidate(
        &mut self,
        function: &str,
        contents_start: usize,
    ) -> Option<String> {
        let contents = &function[contents_start..function.len() - 1];
        let mut out = Splice::new(function);
        split_image_set(contents, &mut |at, text| {
            let start = contents_start + at + (text.len() - text.trim_start().len());
            let candidate = text.trim();
            let bytes = candidate.as_bytes();
            let (end, source) = if function_at(bytes, 0, b"url") {
                scan_url_function(candidate, 0)
            } else if matches!(bytes.first(), Some(b'\'' | b'"')) {
                match skip_quoted(bytes, 0) {
                    Some(end) => (end, css_unescape(&candidate[1..end - 1], true)),
                    None => return,
                }
            } else {
                return;
            };
            if let Some(text) = source.and_then(|source| self.url(source.trim(), false)) {
                out.replace(start, start + end, &text);
            }
        })?;
        out.finish()
    }
}

fn image_value_ranges(css: &str) -> Vec<(usize, usize)> {
    let bytes = css.as_bytes();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    // Where the text of the current declaration starts, after the whitespace that leads it.
    let mut segment_start: Option<usize> = None;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut position = 0;
    while position < bytes.len() {
        if segment_start.is_none() && !bytes[position].is_ascii_whitespace() {
            segment_start = Some(position);
        }
        if starts_with(bytes, position, b"/*") {
            position = skip_comment(bytes, position).unwrap_or(bytes.len());
            continue;
        }
        if bytes[position] == b'\\' {
            position = skip_escape(bytes, position);
            continue;
        }
        if matches!(bytes[position], b'\'' | b'"') {
            position = skip_quoted(bytes, position).unwrap_or(bytes.len());
            continue;
        }
        match bytes[position] {
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'{' if paren_depth == 0 && bracket_depth == 0 => {
                brace_depth += 1;
                segment_start = None;
            }
            b'}' if paren_depth == 0 && bracket_depth == 0 => {
                brace_depth = brace_depth.saturating_sub(1);
                segment_start = None;
            }
            b';' if paren_depth == 0 && bracket_depth == 0 => segment_start = None,
            // A colon in a custom property value, as in `--link: https://…`, is not a new declaration.
            b':' if paren_depth == 0
                && bracket_depth == 0
                && ranges.last().is_none_or(|previous| position >= previous.1) =>
            {
                // Each colon trims only the whitespace before it, so a long selector list stays linear.
                let property = css[segment_start.unwrap_or(position)..position].trim_end();
                if can_hold_image(property) {
                    ranges.push((
                        position + 1,
                        declaration_end(bytes, position + 1, brace_depth),
                    ));
                }
            }
            _ => {}
        }
        position += 1;
    }
    ranges
}

fn declaration_end(bytes: &[u8], start: usize, brace_depth: usize) -> usize {
    let mut position = start;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    while position < bytes.len() {
        if starts_with(bytes, position, b"/*") {
            position = skip_comment(bytes, position).unwrap_or(bytes.len());
            continue;
        }
        if bytes[position] == b'\\' {
            position = skip_escape(bytes, position);
            continue;
        }
        if matches!(bytes[position], b'\'' | b'"') {
            position = skip_quoted(bytes, position).unwrap_or(bytes.len());
            continue;
        }
        match bytes[position] {
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b';' if paren_depth == 0 && bracket_depth == 0 => return position,
            b'}' if paren_depth == 0 && bracket_depth == 0 && brace_depth > 0 => return position,
            _ => {}
        }
        position += 1;
    }
    bytes.len()
}

/// The longest name [`is_image_property`] accepts, so that a longer text is not lowercased.
const MAX_IMAGE_PROPERTY_LEN: usize = "-webkit-mask-box-image-source".len();

fn is_image_property(property: &str) -> bool {
    property.len() <= MAX_IMAGE_PROPERTY_LEN
        && matches!(
            property.to_ascii_lowercase().as_str(),
            "background"
                | "background-image"
                | "border-image"
                | "border-image-source"
                | "content"
                | "cursor"
                | "list-style"
                | "list-style-image"
                | "mask"
                | "mask-border"
                | "mask-border-source"
                | "mask-image"
                | "shape-outside"
                | "symbols"
                | "-webkit-border-image"
                | "-webkit-border-image-source"
                | "-webkit-box-reflect"
                | "-webkit-mask"
                | "-webkit-mask-box-image"
                | "-webkit-mask-box-image-source"
                | "-webkit-mask-image"
        )
}

fn is_root_relative(source: &str) -> bool {
    source.starts_with('/') && !is_protocol_relative(source)
}

/// References, markers and URLs that the URL crate serialized hold no `"`, so the quotes hold any URL written here.
fn url_function(url: &str) -> String {
    format!("url(\"{url}\")")
}

struct ImageSetCandidate<'a> {
    text: &'a str,
    source: Option<String>,
    descriptors: &'a str,
    resolution_dppx: Option<f64>,
}

fn split_image_set<'a>(contents: &'a str, each: &mut impl FnMut(usize, &'a str)) -> Option<()> {
    let bytes = contents.as_bytes();
    let mut start = 0;
    let mut position = 0;
    let mut depth = 0usize;
    while position <= bytes.len() {
        if position == bytes.len() || (bytes[position] == b',' && depth == 0) {
            each(start, &contents[start..position]);
            start = position + 1;
            position += 1;
            continue;
        }
        if bytes[position] == b'\\' {
            position = skip_escape(bytes, position);
            continue;
        }
        if starts_with(bytes, position, b"/*") {
            position = skip_comment(bytes, position)?;
            continue;
        }
        if matches!(bytes[position], b'\'' | b'"') {
            position = skip_quoted(bytes, position)?;
            continue;
        }
        match bytes[position] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        position += 1;
    }
    Some(())
}

fn image_set_candidate(candidate: &str) -> Option<ImageSetCandidate<'_>> {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return None;
    }
    let bytes = candidate.as_bytes();
    let (source, descriptors) = if function_at(bytes, 0, b"url") {
        let (resume_at, source) = scan_url_function(candidate, 0);
        (source, &candidate[resume_at..])
    } else if matches!(bytes[0], b'\'' | b'"') {
        let end = skip_quoted(bytes, 0)?;
        (
            css_unescape(&candidate[1..end - 1], true).map(|s| s.trim().to_owned()),
            &candidate[end..],
        )
    } else {
        (None, "")
    };
    Some(ImageSetCandidate {
        text: candidate,
        source: source.filter(|source| !source.is_empty() && !source.starts_with('#')),
        descriptors,
        resolution_dppx: parse_descriptors(descriptors),
    })
}

fn parse_descriptors(descriptors: &str) -> Option<f64> {
    let bytes = descriptors.as_bytes();
    let mut resolution = None;
    let mut position = 0;
    loop {
        skip_whitespace(bytes, &mut position);
        if position == bytes.len() {
            break;
        }
        if function_at(bytes, position, b"type") {
            let end = matching_paren(bytes, position + 4)?;
            let media_type = descriptors[position + 5..end - 1]
                .trim()
                .trim_matches(['"', '\'']);
            if !is_fetched_image_type(media_type) {
                return None;
            }
            position = end;
            continue;
        }
        let token_end = bytes[position..]
            .iter()
            .position(u8::is_ascii_whitespace)
            .map_or(bytes.len(), |length| position + length);
        if resolution
            .replace(parse_resolution(&descriptors[position..token_end])?)
            .is_some()
        {
            return None;
        }
        position = token_end;
    }
    let value = resolution.unwrap_or(1.0);
    (value.is_finite() && value > 0.0).then_some(value)
}

fn parse_resolution(token: &str) -> Option<f64> {
    if let Some(value) = token.strip_suffix("dppx") {
        value.parse::<f64>().ok()
    } else if let Some(value) = token.strip_suffix("dpi") {
        value.parse::<f64>().ok().map(|value| value / 96.0)
    } else if let Some(value) = token.strip_suffix("dpcm") {
        value.parse::<f64>().ok().map(|value| value * 2.54 / 96.0)
    } else {
        token.strip_suffix('x')?.parse::<f64>().ok()
    }
}

/// Where the `url()` at `start` ends, and its URL after its CSS escapes. A browser reads a `url(` that the text ends
/// before its `)`, so that one runs to the end. `None` for a function that holds no URL a browser loads.
fn scan_url_function(value: &str, start: usize) -> (usize, Option<String>) {
    let bytes = value.as_bytes();
    let Some(resume_at) = matching_paren(bytes, start + 3) else {
        let rest = value[start + 4..].trim();
        let source = match rest.as_bytes().first() {
            Some(&quote @ (b'\'' | b'"')) => {
                css_unescape(rest[1..].trim_end_matches(char::from(quote)), true)
            }
            _ => css_unescape(rest, false),
        };
        return (
            value.len(),
            source
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
        );
    };
    let contents_end = resume_at - 1;
    let mut position = start + 4;
    skip_whitespace(bytes, &mut position);
    let source = if matches!(bytes.get(position), Some(b'\'' | b'"')) {
        let content_start = position + 1;
        let Some(end) = skip_quoted(bytes, position) else {
            return (resume_at, None);
        };
        position = end;
        skip_whitespace(bytes, &mut position);
        if position != contents_end {
            return (resume_at, None);
        }
        css_unescape(&value[content_start..end - 1], true)
    } else {
        css_unescape(value[position..contents_end].trim(), false)
    };
    (
        resume_at,
        source
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
    )
}

/// The text of a CSS string or URL token after its escapes, as CSS Syntax reads them. `None` for a URL token with an
/// escaped line break, which a browser does not load.
fn css_unescape(raw: &str, quoted: bool) -> Option<String> {
    if !raw.contains('\\') {
        return Some(raw.to_owned());
    }
    let mut text = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\\' {
            text.push(character);
            continue;
        }
        match chars.peek().copied() {
            None if quoted => {}
            None => text.push('\u{fffd}'),
            Some('\n' | '\r' | '\u{c}') if !quoted => return None,
            Some(line_break @ ('\n' | '\r' | '\u{c}')) => {
                chars.next();
                if line_break == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            Some(digit) if digit.is_ascii_hexdigit() => {
                let mut value = 0u32;
                for _ in 0..6 {
                    match chars.peek().and_then(|c| c.to_digit(16)) {
                        Some(digit) => {
                            value = value * 16 + digit;
                            chars.next();
                        }
                        None => break,
                    }
                }
                match chars.peek() {
                    Some(' ' | '\t' | '\n' | '\u{c}') => {
                        chars.next();
                    }
                    Some('\r') => {
                        chars.next();
                        if chars.peek() == Some(&'\n') {
                            chars.next();
                        }
                    }
                    _ => {}
                }
                text.push(
                    char::from_u32(value)
                        .filter(|&c| c != '\0')
                        .unwrap_or('\u{fffd}'),
                );
            }
            Some(escaped) => {
                chars.next();
                text.push(escaped);
            }
        }
    }
    Some(text)
}

fn function_at(bytes: &[u8], start: usize, name: &[u8]) -> bool {
    bytes
        .get(start..start + name.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        && bytes.get(start + name.len()) == Some(&b'(')
        && (start == 0
            || !matches!(bytes[start - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-'))
}

fn matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut position = open + 1;
    let mut depth = 1usize;
    while position < bytes.len() {
        if starts_with(bytes, position, b"/*") {
            position = skip_comment(bytes, position)?;
            continue;
        }
        if bytes[position] == b'\\' {
            position = skip_escape(bytes, position);
            continue;
        }
        if matches!(bytes[position], b'\'' | b'"') {
            position = skip_quoted(bytes, position)?;
            continue;
        }
        match bytes[position] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(position + 1);
                }
            }
            _ => {}
        }
        position += 1;
    }
    None
}

fn skip_quoted(bytes: &[u8], start: usize) -> Option<usize> {
    let quote = *bytes.get(start)?;
    let mut position = start + 1;
    while position < bytes.len() {
        match bytes[position] {
            b'\\' => position += 2,
            value if value == quote => return Some(position + 1),
            _ => position += 1,
        }
    }
    None
}

/// The position after the escape at `start` and the character it escapes, so that an escaped quote, bracket or
/// parenthesis does not count as one.
fn skip_escape(bytes: &[u8], start: usize) -> usize {
    let width = match bytes.get(start + 1) {
        Some(0xf0..) => 4,
        Some(0xe0..) => 3,
        Some(0xc0..) => 2,
        _ => 1,
    };
    (start + 1 + width).min(bytes.len())
}

fn skip_comment(bytes: &[u8], start: usize) -> Option<usize> {
    let relative = bytes
        .get(start + 2..)?
        .windows(2)
        .position(|pair| pair == b"*/")?;
    Some(start + 2 + relative + 2)
}

fn skip_whitespace(bytes: &[u8], position: &mut usize) {
    while bytes.get(*position).is_some_and(u8::is_ascii_whitespace) {
        *position += 1;
    }
}

fn starts_with(bytes: &[u8], start: usize, needle: &[u8]) -> bool {
    bytes.get(start..start + needle.len()) == Some(needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CSS as `rewrite_css` leaves it, with each URL replaced by `<url>`, and `!` before a URL a renderer does not
    /// show. Also the URLs in the order `asset` saw them.
    fn rewritten(css: &str, page_url: Option<&str>) -> (String, Vec<String>) {
        let base = page_url.and_then(page_base);
        let mut seen = Vec::new();
        let out = rewrite_css(
            css,
            CssKind::Text,
            base.as_ref(),
            &mut |image| {
                let label = format!("{}{}", if image.shown { "" } else { "!" }, image.url);
                seen.push(label.clone());
                Some(Replacement {
                    text: format!("<{label}>"),
                    declined: false,
                })
            },
            &mut CssRewriteCounts::default(),
        );
        (out.unwrap_or_else(|| css.to_owned()), seen)
    }

    #[test]
    fn reads_past_escapes_outside_strings() {
        // Tailwind escapes brackets, parentheses and quotes in its arbitrary value class names.
        let selector = r".bg-\[url\(\'\/hero\.png\'\)\]{background-image:url('/hero.png')}";
        assert_eq!(
            rewritten(
                &format!("{selector}.a{{background:url(https://cdn.example.com/a.png)}}"),
                None
            ),
            (
                format!(r#"{selector}.a{{background:url("<https://cdn.example.com/a.png>")}}"#),
                vec!["https://cdn.example.com/a.png".to_owned()]
            )
        );
    }

    #[test]
    fn scans_a_long_selector_list_in_linear_time() {
        let css = format!(
            "{} a:hover {{ background: url(https://cdn.example.com/a.png) }}",
            "a:hover, b:focus-visible,\n".repeat(50_000)
        );
        assert_eq!(rewritten(&css, None).1, ["https://cdn.example.com/a.png"]);
    }

    #[test]
    fn rewrites_each_image_url_where_the_page_wrote_it() {
        let page = Some("http://app.example.com/shop/item?id=1");
        for (css, expected) in [
            (
                ":root{--hero:url(https://cdn.example.com/hero.png)}",
                r#":root{--hero:url("<https://cdn.example.com/hero.png>")}"#,
            ),
            (
                ".a{background:url(/img/a.png)}",
                r#".a{background:url("<https://app.example.com/img/a.png>")}"#,
            ),
            (
                ".a{background:url(../img/a.png)}",
                ".a{background:url(../img/a.png)}",
            ),
            (".a{mask:url(#shape)}", ".a{mask:url(#shape)}"),
            (
                ".a{color:url(https://cdn.example.com/a.png)}",
                ".a{color:url(https://cdn.example.com/a.png)}",
            ),
            (
                r".a{background:url(https\3a //cdn.example.com/a.png#access_token=x)}",
                r#".a{background:url("<https://cdn.example.com/a.png#access_token=x>")}"#,
            ),
            (
                ".a{background:url(https://cdn.example.com/a.png",
                r#".a{background:url("<https://cdn.example.com/a.png>")"#,
            ),
            (
                r#".a{background-image:image-set("https://cdn.example.com/1x.png" 1x, url(https://cdn.example.com/2x.png) 2x)}"#,
                r#".a{background-image:image-set(url("<https://cdn.example.com/2x.png>") 2x)}"#,
            ),
            (
                r#".a{background:-webkit-image-set(url(/a.avif) type("image/avif") 2x, url(/a.jpg) type("image/jpeg") 1x, linear-gradient(red, blue) 3x)}"#,
                r#".a{background:-webkit-image-set(url("<https://app.example.com/a.jpg>") type("image/jpeg") 1x)}"#,
            ),
            (
                ".a{background:image-set(url(https://cdn.example.com/a.png))}",
                r#".a{background:image-set(url("<https://cdn.example.com/a.png>"))}"#,
            ),
            // A set with no candidate that can load keeps each one, as the declined marker it gets.
            (
                r#".a{background:image-set(url(https://10.0.0.7/a.png) 1x, "https://10.0.0.7/b.png" 2x)}"#,
                r#".a{background:image-set(url("<!https://10.0.0.7/a.png>") 1x, url("<!https://10.0.0.7/b.png>") 2x)}"#,
            ),
        ] {
            assert_eq!(rewritten(css, page).0, expected, "{css}");
        }
        let many: Vec<String> = (1..=MAX_IMAGE_SET_CANDIDATES + 1)
            .map(|i| format!("url(https://cdn.example.com/{i}.png) {i}x"))
            .collect();
        let (_, seen) = rewritten(
            &format!(".a{{background:image-set({})}}", many.join(", ")),
            page,
        );
        assert_eq!(seen.len(), many.len());
        assert!(seen.iter().all(|url| url.starts_with('!')), "{seen:?}");
    }

    #[test]
    fn counts_the_sets_it_cuts_and_the_urls_it_resolves() {
        let mut counts = CssRewriteCounts::default();
        rewrite_css(
            "url(/a.png), image-set(url(https://cdn.example.com/1x.png) 1x, url(https://cdn.example.com/2x.png) 2x)",
            CssKind::Value,
            None,
            &mut |_| None,
            &mut counts,
        );
        assert_eq!(
            counts,
            CssRewriteCounts {
                image_sets: 1,
                relative_urls: 0,
                unresolved: 1,
            }
        );
    }
}
