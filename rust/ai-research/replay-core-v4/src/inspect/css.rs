//! Image URLs in CSS: the `url()`s of an image property, or of a custom property that reaches one through `var()`.
//! Font URLs are the `url()`s in the `src` descriptor of an `@font-face` rule. `cssparser` tokenizes as CSS Syntax 3 does, so a name, an escape, a comment or a bad URL reads here as the renderer reads it.
//! [`rewrite_css`] is the one reader of them. The inspector runs it through [`css_asset_urls`] to choose the fetch jobs, and the writer runs it to replace each URL with what the block stores.

use cssparser::{
    match_ignore_ascii_case, AtRuleParser, CowRcStr, DeclarationParser, Delimiter, ParseError,
    Parser, ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, SourcePosition,
    Token,
};
use url::Url;

use super::assets::Replacement;
use crate::fonts::bundled_font;
use crate::scan::images::is_data_url;
use crate::url_policy::{
    is_fetched_image_type, is_protocol_relative, is_remote, try_canonicalize, MAX_URL_LEN,
};

/// Far more than a real `image-set()` lists. A longer set keeps all its candidates.
const MAX_IMAGE_SET_CANDIDATES: usize = 32;
/// The depth at which `cssparser` enters no more blocks, to keep the stack bounded. Its default, set here so that the scan knows where it stops.
const NESTED_BLOCK_LIMIT: u8 = 75;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssKind {
    /// Declarations or rules: a `style` attribute, `<style>` text, `_cssText` or a CSSOM rule.
    Text,
    /// The value of one property that can hold an image, as a style diff or a CSSOM declaration sets it.
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssAssetKind {
    Image,
    /// A `src` URL of an `@font-face` rule, with the slug of the rule's family when the renderer ships that family.
    Font {
        bundled: Option<&'static str>,
    },
}

/// A URL in CSS after its CSS escapes, and absolute when the page wrote it root-relative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CssAsset<'a> {
    pub url: &'a str,
    pub kind: CssAssetKind,
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

/// Whether `css` holds a `url(` or an `image-set(`, so that most CSS strings need no slot. An escaped function name
/// such as `\75 rl(` holds a `(` after something else, so any `\` before a `(` counts too.
pub fn may_hold_image(css: &str) -> bool {
    let bytes = css.as_bytes();
    let ends_with = |end: usize, name: &[u8]| {
        end >= name.len() && bytes[end - name.len()..end].eq_ignore_ascii_case(name)
    };
    memchr::memchr_iter(b'(', bytes).any(|open| {
        ends_with(open, b"url") || ends_with(open, b"image-set") || bytes[..open].contains(&b'\\')
    })
}

/// `css` with each URL replaced by what `asset` returns for it, and each `image-set()` cut to the one candidate a
/// renderer can load. `None` when nothing changed. Only whole `url()` and `image-set()` functions in the value of an
/// image property or of an `@font-face` `src` change, so a class name such as `bg-[url(/a.png)]`, a quoted string or a
/// comment keeps its text.
pub fn rewrite_css(
    css: &str,
    kind: CssKind,
    base: Option<&Url>,
    asset: &mut impl FnMut(CssAsset<'_>) -> Option<Replacement>,
    counts: &mut CssRewriteCounts,
) -> Option<String> {
    scan_css(css, kind, base, asset, counts, true).0
}

/// Calls `found` with each URL in `css`, as [`rewrite_css`] finds them, and builds no rewritten text. Returns false when
/// rules or functions nest past the depth `cssparser` reads, so that a URL there goes unread.
pub fn css_asset_urls(
    css: &str,
    kind: CssKind,
    base: Option<&Url>,
    found: &mut impl FnMut(CssAsset<'_>),
) -> bool {
    let mut asset = |found_asset: CssAsset<'_>| {
        found(found_asset);
        None
    };
    scan_css(
        css,
        kind,
        base,
        &mut asset,
        &mut CssRewriteCounts::default(),
        false,
    )
    .1
}

fn scan_css(
    css: &str,
    kind: CssKind,
    base: Option<&Url>,
    asset: &mut impl FnMut(CssAsset<'_>) -> Option<Replacement>,
    counts: &mut CssRewriteCounts,
    write: bool,
) -> (Option<String>, bool) {
    let mut out = Splice::new(css);
    let mut rewrite = Rewrite {
        base,
        asset,
        counts,
        write,
        whole: true,
    };
    match kind {
        CssKind::Text => {
            let found = declarations(css);
            rewrite.whole &= !found.nested_too_deep;
            let mut families: Vec<Option<&str>> = Vec::new();
            for declaration in &found.found {
                if let DeclarationKind::FontFamily(face) = declaration.kind {
                    if families.len() <= face {
                        families.resize(face + 1, None);
                    }
                    families[face] = Some(&css[declaration.start..declaration.end]);
                }
            }
            for declaration in &found.found {
                let kind = match declaration.kind {
                    DeclarationKind::Image => CssAssetKind::Image,
                    DeclarationKind::FontSrc(face) => CssAssetKind::Font {
                        bundled: families.get(face).copied().flatten().and_then(bundled_font),
                    },
                    DeclarationKind::FontFamily(_) => continue,
                };
                rewrite.value(css, (declaration.start, declaration.end), kind, &mut out);
            }
        }
        CssKind::Value => rewrite.value(css, (0, css.len()), CssAssetKind::Image, &mut out),
    }
    (out.finish(), rewrite.whole)
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
    whole: bool,
}

impl<F: FnMut(CssAsset<'_>) -> Option<Replacement>> Rewrite<'_, F> {
    /// Rewrites the functions in `css[range]`, the value of one image property or `@font-face` `src`.
    fn value(
        &mut self,
        css: &str,
        range: (usize, usize),
        kind: CssAssetKind,
        out: &mut Splice<'_>,
    ) {
        let value = &css[range.0..range.1];
        let mut input = Parser::new(value);
        input.set_nested_block_limit(NESTED_BLOCK_LIMIT);
        let whole = each_function(&mut input, 0, &mut |input, start, opened| {
            let text = match opened {
                Opened::ImageSet if kind == CssAssetKind::Image => self.image_set(input, start),
                Opened::ImageSet => {
                    skip_block(input);
                    None
                }
                Opened::UnquotedUrl(url) => self.url(&url, kind, true),
                Opened::UrlFunction => {
                    quoted_url_source(input).and_then(|url| self.url(&url, kind, true))
                }
            };
            if let Some(text) = text {
                let end = range.0 + input.position().byte_index();
                out.replace(range.0 + start.byte_index(), end, &text);
            }
        });
        self.whole &= whole;
    }

    /// The `url()` that replaces one with this source. `None` keeps the function as the page wrote it.
    fn url(&mut self, source: &str, kind: CssAssetKind, shown: bool) -> Option<String> {
        let source = source.trim();
        // A fragment names an element of the page, such as an SVG mask, and loads nothing.
        if source.is_empty() || source.starts_with('#') {
            return None;
        }
        if is_root_relative(source) {
            let Some(absolute) = resolve(self.base, source) else {
                self.counts.unresolved += 1;
                return None;
            };
            self.counts.relative_urls += 1;
            let replaced = (self.asset)(CssAsset {
                url: &absolute,
                kind,
                shown,
            });
            return self
                .write
                .then(|| url_function(&replaced.map_or(absolute, |r| r.text)));
        }
        if is_remote(source) || is_data_url(source) {
            let replaced = (self.asset)(CssAsset {
                url: source,
                kind,
                shown,
            });
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
    fn image_set(&mut self, input: &mut Parser<'_>, start: SourcePosition) -> Option<String> {
        let opening = input.slice_from(start);
        let name = opening[..opening.len() - 1].to_owned();
        let candidates = input
            .parse_nested_block(|input| Ok::<_, ParseError<()>>(image_set_candidates(input)))
            .ok()
            .flatten();
        let function = input.slice_from(start);
        let base = self.base;
        let loads = |source: &str| {
            is_data_url(source)
                || (is_remote(source) && try_canonicalize(source).is_ok())
                || (base.is_some() && is_root_relative(source))
        };
        let kept = candidates.as_ref().and_then(|candidates| {
            candidates
                .iter()
                .enumerate()
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
                })
        });
        let (Some(candidates), Some((index, _))) = (&candidates, kept) else {
            return self.each_image_set_source(function, name.len() + 1);
        };
        let kept = &candidates[index];
        let url = self.url(
            kept.source.as_deref().unwrap_or_default(),
            CssAssetKind::Image,
            true,
        );
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
        Some(format!("{name}({candidate})"))
    }

    /// Replaces the source of each candidate in place, reading the set again one candidate at a time, so that a long set
    /// is never held.
    fn each_image_set_source(&mut self, function: &str, contents_start: usize) -> Option<String> {
        let mut out = Splice::new(function);
        let mut input = Parser::new(&function[contents_start..function.len() - 1]);
        input.set_nested_block_limit(NESTED_BLOCK_LIMIT);
        loop {
            let _ = input.parse_until_before(Delimiter::Comma, |input| {
                input.skip_whitespace();
                let start = input.position();
                let source = match input.next() {
                    Ok(Token::UnquotedUrl(url)) => Some(url.to_string()),
                    Ok(Token::QuotedString(url)) => Some(url.to_string()),
                    Ok(Token::Function(name)) if name.eq_ignore_ascii_case("url") => {
                        quoted_url_source(input)
                    }
                    _ => None,
                };
                let end = input.position();
                if let Some(text) =
                    source.and_then(|source| self.url(&source, CssAssetKind::Image, false))
                {
                    out.replace(
                        contents_start + start.byte_index(),
                        contents_start + end.byte_index(),
                        &text,
                    );
                }
                while input.next().is_ok() {}
                Ok::<_, ParseError<()>>(())
            });
            if input.next().is_err() {
                break;
            }
        }
        out.finish()
    }
}

/// References, markers and URLs that the URL crate serialized hold no `"`, so the quotes hold any URL written here.
fn url_function(url: &str) -> String {
    format!("url(\"{url}\")")
}

pub fn can_hold_image(property: &str) -> bool {
    let property = property.trim();
    property.starts_with("--") || is_image_property(property)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclarationKind {
    Image,
    FontSrc(usize),
    FontFamily(usize),
}

struct Declaration {
    start: usize,
    end: usize,
    kind: DeclarationKind,
}

#[derive(Clone, Copy)]
enum Block {
    Style,
    FontFace(usize),
}

/// The text can be a style attribute, a style sheet or one rule, so its top level reads as a rule body, which holds declarations and rules alike.
fn declarations(css: &str) -> Declarations {
    let mut declarations = Declarations {
        found: Vec::new(),
        font_faces: 0,
        block: Block::Style,
        depth: 0,
        nested_too_deep: false,
    };
    let mut input = Parser::new(css);
    input.set_nested_block_limit(NESTED_BLOCK_LIMIT);
    declarations.read_block(&mut input, Block::Style);
    declarations
}

struct Declarations {
    found: Vec<Declaration>,
    font_faces: usize,
    block: Block,
    depth: u8,
    nested_too_deep: bool,
}

impl Declarations {
    fn read_block(&mut self, input: &mut Parser<'_>, block: Block) {
        let outer = std::mem::replace(&mut self.block, block);
        self.nested_too_deep |= self.depth >= NESTED_BLOCK_LIMIT;
        let body: RuleBodyParser<'_, '_, Self, (), ()> = RuleBodyParser::new(input, self);
        for _ in body {}
        self.block = outer;
    }

    fn read_nested_block(&mut self, input: &mut Parser<'_>, block: Block) {
        self.depth += 1;
        self.read_block(input, block);
        self.depth -= 1;
    }
}

impl<'i> DeclarationParser<'i> for Declarations {
    type Declaration = ();
    type Error = ();

    /// A value that holds a `{}` block is a nested rule, as `a:hover{…}` is, unless the property is a custom one.
    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _: &ParserState,
    ) -> Result<(), ParseError<()>> {
        let start = input.position();
        let custom = name.starts_with("--");
        while let Ok(token) = input.next() {
            if !custom && matches!(token, Token::CurlyBracketBlock) {
                return Err(ParseError::unexpected_token());
            }
        }
        let kind = match self.block {
            Block::FontFace(face) => font_descriptor(&name, face),
            Block::Style => can_hold_image(&name).then_some(DeclarationKind::Image),
        };
        if let Some(kind) = kind {
            self.found.push(Declaration {
                start: start.byte_index(),
                end: input.position().byte_index(),
                kind,
            });
        }
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for Declarations {
    type Prelude = Block;
    type AtRule = ();
    type Error = ();

    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Block, ParseError<()>> {
        let font_face = name.eq_ignore_ascii_case("font-face") && input.is_exhausted();
        while input.next().is_ok() {}
        if !font_face {
            return Ok(Block::Style);
        }
        self.font_faces += 1;
        Ok(Block::FontFace(self.font_faces - 1))
    }

    fn rule_without_block(&mut self, _: Block, _: &ParserState) -> Result<(), ()> {
        Ok(())
    }

    fn parse_block(
        &mut self,
        block: Block,
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<(), ParseError<()>> {
        self.read_nested_block(input, block);
        Ok(())
    }
}

impl<'i> QualifiedRuleParser<'i> for Declarations {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = ();

    fn parse_prelude(&mut self, input: &mut Parser<'i>) -> Result<(), ParseError<()>> {
        while input.next().is_ok() {}
        Ok(())
    }

    fn parse_block(
        &mut self,
        _: (),
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<(), ParseError<()>> {
        self.read_nested_block(input, Block::Style);
        Ok(())
    }
}

impl<'i> RuleBodyItemParser<'i, (), ()> for Declarations {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        matches!(self.block, Block::Style)
    }
}

fn font_descriptor(descriptor: &str, face: usize) -> Option<DeclarationKind> {
    if descriptor.eq_ignore_ascii_case("src") {
        Some(DeclarationKind::FontSrc(face))
    } else if descriptor.eq_ignore_ascii_case("font-family") {
        Some(DeclarationKind::FontFamily(face))
    } else {
        None
    }
}

fn is_image_property(property: &str) -> bool {
    match_ignore_ascii_case! { property,
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
        | "-webkit-mask-image" => true,
        _ => false,
    }
}

enum Opened<'i> {
    UnquotedUrl(CowRcStr<'i>),
    UrlFunction,
    ImageSet,
}

/// Calls `each` at each `url()` and `image-set()` in `input`, at any depth. Returns false when it skipped a block too deep to read whole, as `each` enters one more block for an `image-set()` candidate.
fn each_function<'i>(
    input: &mut Parser<'i>,
    depth: u8,
    each: &mut impl FnMut(&mut Parser<'i>, SourcePosition, Opened<'i>),
) -> bool {
    let mut whole = true;
    loop {
        let start = input.position();
        let token = match input.next_including_whitespace_and_comments() {
            Err(_) => return whole,
            Ok(token) => token,
        };
        let opens_block = matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        );
        if opens_block && depth + 1 >= NESTED_BLOCK_LIMIT {
            whole = false;
            continue;
        }
        let opened = match token {
            Token::UnquotedUrl(url) => Opened::UnquotedUrl(url.clone()),
            Token::Function(name) if name.eq_ignore_ascii_case("url") => Opened::UrlFunction,
            Token::Function(name) if is_image_set(name) => Opened::ImageSet,
            _ if opens_block => {
                let _ = input.parse_nested_block(|input| {
                    whole &= each_function(input, depth + 1, each);
                    Ok::<_, ParseError<()>>(())
                });
                continue;
            }
            _ => continue,
        };
        each(input, start, opened);
    }
}

fn is_image_set(name: &str) -> bool {
    name.eq_ignore_ascii_case("image-set") || name.eq_ignore_ascii_case("-webkit-image-set")
}

fn skip_block(input: &mut Parser<'_>) {
    let _ = input.parse_nested_block(|_| Ok::<_, ParseError<()>>(()));
}

/// The URL of a `url("…")` function, read as a string token, so after its escapes.
fn quoted_url_source(input: &mut Parser<'_>) -> Option<String> {
    input
        .parse_nested_block(|input| {
            input.skip_whitespace();
            Ok::<_, ParseError<()>>(input.expect_string()?.to_string())
        })
        .ok()
}

fn is_root_relative(source: &str) -> bool {
    source.starts_with('/') && !is_protocol_relative(source)
}

struct ImageSetCandidate<'a> {
    text: &'a str,
    source: Option<String>,
    descriptors: &'a str,
    resolution_dppx: Option<f64>,
}

/// `None` when a candidate is empty, or when the set lists more than [`MAX_IMAGE_SET_CANDIDATES`], which it stops holding.
fn image_set_candidates<'i>(input: &mut Parser<'i>) -> Option<Vec<ImageSetCandidate<'i>>> {
    let mut candidates = Vec::new();
    let mut count = 0;
    let mut readable = true;
    loop {
        let candidate = input
            .parse_until_before(Delimiter::Comma, |input| {
                Ok::<_, ParseError<()>>(image_set_candidate(input))
            })
            .ok()
            .flatten();
        count += 1;
        if count <= MAX_IMAGE_SET_CANDIDATES {
            match candidate {
                Some(candidate) => candidates.push(candidate),
                None => readable = false,
            }
        } else {
            candidates.clear();
        }
        if input.next().is_err() {
            break;
        }
    }
    (readable && count <= MAX_IMAGE_SET_CANDIDATES).then_some(candidates)
}

enum CandidateStart<'i> {
    Url(CowRcStr<'i>),
    String(CowRcStr<'i>),
    UrlFunction,
    Other,
}

fn image_set_candidate<'i>(input: &mut Parser<'i>) -> Option<ImageSetCandidate<'i>> {
    input.skip_whitespace();
    let start = input.position();
    let first = match input.next() {
        Err(_) => return None,
        Ok(Token::UnquotedUrl(url)) => CandidateStart::Url(url.clone()),
        Ok(Token::QuotedString(url)) => CandidateStart::String(url.clone()),
        Ok(Token::Function(name)) if name.eq_ignore_ascii_case("url") => {
            CandidateStart::UrlFunction
        }
        Ok(_) => CandidateStart::Other,
    };
    let source = match first {
        CandidateStart::Url(url) | CandidateStart::String(url) => Some(url.to_string()),
        CandidateStart::UrlFunction => quoted_url_source(input),
        CandidateStart::Other => None,
    };
    let source_end = input.position();
    let resolution_dppx = parse_descriptors(input);
    Some(ImageSetCandidate {
        text: input.slice_from(start).trim_end(),
        source: source
            .map(|source| source.trim().to_owned())
            .filter(|source| !source.is_empty() && !source.starts_with('#')),
        descriptors: input.slice(source_end..input.position()),
        resolution_dppx,
    })
}

enum Descriptor {
    Resolution(f64),
    Type,
}

fn parse_descriptors(input: &mut Parser<'_>) -> Option<f64> {
    let mut resolution = None;
    let mut readable = true;
    loop {
        let descriptor = match input.next() {
            Err(_) => break,
            Ok(Token::Dimension { value, unit, .. }) => {
                resolution_dppx(f64::from(*value), unit).map(Descriptor::Resolution)
            }
            Ok(Token::Function(name)) if name.eq_ignore_ascii_case("type") => {
                Some(Descriptor::Type)
            }
            Ok(_) => None,
        };
        readable &= match descriptor {
            Some(Descriptor::Resolution(dppx)) => resolution.replace(dppx).is_none(),
            Some(Descriptor::Type) => input
                .parse_nested_block(|input| {
                    Ok::<_, ParseError<()>>(is_fetched_image_type(input.expect_string()?.trim()))
                })
                .unwrap_or(false),
            None => false,
        };
    }
    let value = resolution.unwrap_or(1.0);
    (readable && value.is_finite() && value > 0.0).then_some(value)
}

fn resolution_dppx(value: f64, unit: &str) -> Option<f64> {
    match_ignore_ascii_case! { unit,
        "x" | "dppx" => Some(value),
        "dpi" => Some(value / 96.0),
        "dpcm" => Some(value * 2.54 / 96.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A URL as `asset` saw it: `!` before one a renderer does not show, and `font` or the bundled slug for a font.
    fn label(asset: &CssAsset<'_>) -> String {
        let shown = if asset.shown { "" } else { "!" };
        match asset.kind {
            CssAssetKind::Image => format!("{shown}{}", asset.url),
            CssAssetKind::Font { bundled: None } => format!("{shown}font {}", asset.url),
            CssAssetKind::Font {
                bundled: Some(slug),
            } => format!("{shown}{slug} {}", asset.url),
        }
    }

    /// The CSS as `rewrite_css` leaves it, with each URL replaced by `<label>`, and the labels in the order `asset` saw
    /// them.
    fn rewritten(css: &str, page_url: Option<&str>) -> (String, Vec<String>) {
        let base = page_url.and_then(page_base);
        let mut seen = Vec::new();
        let out = rewrite_css(
            css,
            CssKind::Text,
            base.as_ref(),
            &mut |asset| {
                let label = label(&asset);
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
    fn reads_each_font_face_src_url_with_its_rules_family() {
        let css = concat!(
            "/* fonts */@FONT-FACE{font-family:'Brand Sans';src:url(https://cdn.example.com/brand.woff2) format('woff2'),local(Arial),url(\"https://cdn.example.com/brand.woff\")}",
            "@media print{@font-face{src:url(https://cdn.example.com/print.woff2);font-family:Print}}",
            "@font-face{src:url(https://cdn.example.com/nameless.woff2)}",
            "@font-face{font-family:Self;src:url(/fonts/self.woff2)}",
            "@font-face{font-family:Roboto;src:url(https://cdn.example.com/roboto.woff2)}",
            ".a{font-family:Inter;src:url(https://cdn.example.com/not-a-face.woff2);background:url(https://cdn.example.com/a.png)}",
            ".\\@font-face{src:url(https://cdn.example.com/escaped.woff2);background:url(https://cdn.example.com/escaped.png)}",
            "@font-face/* trailing */{font-family:Trail;src:url(https://cdn.example.com/trail.woff2)}",
            "@\\66 ont-face{font-family:Keyword;src:url(https://cdn.example.com/keyword.woff2)}",
            "@font-face{font-family:Descriptor;\\73 rc:url(https://cdn.example.com/descriptor.woff2)}",
            "@font-face{font-family:Function;src:\\75 rl(https://cdn.example.com/function.woff2)}",
            "@font-face{font-family:Source;src:url(https://cdn.example.com/source\\2e woff2)}",
            "@font-face{font-family:Relative;src:url(//cdn.example.com/relative\\2e woff2)}",
            "@font-face{font-family:Continued;src:url(\"https://cdn.example.com/continued.\\\nwoff2\")}",
            "@font-face{font-family:Broken;src:url(https://cdn.example.com/broken.\\\nwoff2)}",
            "@font-face{font-family:Commented;/* lead */src/**/:url(\"https://cdn.example.com/commented.woff2\"/* tail */)}",
        );
        let (out, seen) = rewritten(css, Some("https://app.example.com/"));
        assert_eq!(
            seen,
            [
                "font https://cdn.example.com/brand.woff2",
                "font https://cdn.example.com/brand.woff",
                "font https://cdn.example.com/print.woff2",
                "font https://cdn.example.com/nameless.woff2",
                "font https://app.example.com/fonts/self.woff2",
                "roboto https://cdn.example.com/roboto.woff2",
                "https://cdn.example.com/a.png",
                "https://cdn.example.com/escaped.png",
                "font https://cdn.example.com/trail.woff2",
                "font https://cdn.example.com/keyword.woff2",
                "font https://cdn.example.com/descriptor.woff2",
                "font https://cdn.example.com/function.woff2",
                "font https://cdn.example.com/source.woff2",
                "font //cdn.example.com/relative.woff2",
                "font https://cdn.example.com/continued.woff2",
                "font https://cdn.example.com/commented.woff2",
            ]
        );
        // A URL token with an escaped line break is a bad URL, which loads nothing, so it stays.
        assert!(out.contains("broken.\\\nwoff2"), "{out}");
        assert!(
            out.contains("src:url(https://cdn.example.com/not-a-face.woff2)"),
            "{out}"
        );
    }

    #[test]
    fn reads_past_escapes_outside_strings() {
        // Tailwind escapes brackets, parentheses and quotes in its arbitrary value class names.
        let selector = r".bg-\[url\(\'\/hero\.png\'\)\]{background-image:url('/hero.png')}";
        let css = format!(
            r"{selector}.a{{background:url(https://cdn.example.com/a.png)}}.p{{background-\69mage:url(https://cdn.example.com/p.png)}}.f{{background:\75 rl(https://cdn.example.com/f.png)}}"
        );
        let (out, seen) = rewritten(&css, None);
        assert_eq!(
            seen,
            [
                "https://cdn.example.com/a.png",
                "https://cdn.example.com/p.png",
                "https://cdn.example.com/f.png",
            ]
        );
        assert!(out.starts_with(selector), "{out}");
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
                ".a{/* lead */background/**/:url(\"https://cdn.example.com/c.png\"/* tail */)}",
                r#".a{/* lead */background/**/:url("<https://cdn.example.com/c.png>")}"#,
            ),
            (
                r".a{background:url(https\3a //cdn.example.com/a.png#access_token=x)}",
                r#".a{background:url("<https://cdn.example.com/a.png#access_token=x>")}"#,
            ),
            (
                ".a{background:url(https://cdn.example.com/a.png",
                r#".a{background:url("<https://cdn.example.com/a.png>")"#,
            ),
            // A parenthesis makes a bad URL token, which a browser does not load.
            (
                ".a{background:url(https://cdn.example.com/a(1).png)}",
                ".a{background:url(https://cdn.example.com/a(1).png)}",
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

    #[test]
    fn says_when_rules_nest_past_what_it_reads() {
        let deep = format!(
            "{}.a{{background:url(https://cdn.example.com/a.png)}}{}",
            "@media print{".repeat(80),
            "}".repeat(80)
        );
        assert!(!css_asset_urls(&deep, CssKind::Text, None, &mut |_| {}));
        assert!(css_asset_urls(
            ".a{background:url(https://cdn.example.com/a.png)}",
            CssKind::Text,
            None,
            &mut |_| {}
        ));
    }
}
