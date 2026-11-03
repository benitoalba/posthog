//! What each asset URL of a stored block becomes. The inspector records where each image slot's string sits, and the
//! writer gives each one to [`AssetRewrite::slot`], whose entry points read it as one URL, a srcset or CSS. Every URL
//! they find goes through [`AssetRewrite::asset_url`]: a remote URL becomes its `imageurl:v4` reference, as in v3, or
//! a marker that names why the policy declines it, so that no credential, signature or private host stays in the block,
//! and an inline image becomes its `image:v4` reference. A font URL of an `@font-face` `src` gets a `fonturl:v4`
//! reference or marker instead, or the slug of the family when the renderer ships it.

use std::collections::BTreeMap;

use metrics::counter;
use posthog_replay_anonymizer::v4_image_refs::{
    bundled_font_ref, ImageDecline, IMAGE_DECLINED_PREFIX,
};
use replay_core_v4::inspect::assets::{AssetContext, Replacement};
use replay_core_v4::inspect::css::{rewrite_css, CssAssetKind, CssRewriteCounts};
use replay_core_v4::inspect::srcset::rewrite_srcset;
use replay_core_v4::refs::{declined_url_ref, UrlAsset};
use replay_core_v4::scan::images::{inline_image, is_data_url, ImageSink, InlineImage};
use replay_core_v4::seal::image_url_key;
use replay_core_v4::url_policy::{is_remote, MAX_URL_LEN};

use crate::urls::url_reference;

/// The counter increments of one block, which the writer emits only for a block it keeps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassCounters {
    srcset_rewrites: BTreeMap<&'static str, u64>,
    css_rewrites: BTreeMap<(&'static str, &'static str), u64>,
    url_rewrites: BTreeMap<(&'static str, &'static str, &'static str), u64>,
    pub removed_images: [u64; ImageDecline::ALL.len()],
}

impl PassCounters {
    pub fn emit(&self) {
        for (&outcome, &count) in &self.srcset_rewrites {
            counter!("ai_research_v4_recorder_srcset_rewrites_total", "outcome" => outcome)
                .increment(count);
        }
        for (&(kind, outcome), &count) in &self.css_rewrites {
            counter!(
                "ai_research_v4_recorder_css_rewrites_total",
                "kind" => kind,
                "outcome" => outcome,
            )
            .increment(count);
        }
        for (&(kind, outcome, reason), &count) in &self.url_rewrites {
            counter!(
                "ai_research_v4_recorder_image_url_rewrites_total",
                "kind" => kind,
                "outcome" => outcome,
                "reason" => reason,
            )
            .increment(count);
        }
        for (reason, &count) in ImageDecline::ALL.iter().zip(&self.removed_images) {
            counter!("ai_research_v4_recorder_removed_images_total", "reason" => reason.label())
                .increment(count);
        }
    }

    fn add_css(&mut self, counts: CssRewriteCounts) {
        for (key, count) in [
            (("image_set", "rewritten"), counts.image_sets),
            (("relative_url", "rewritten"), counts.relative_urls),
            (("relative_url", "unresolved"), counts.unresolved),
        ] {
            if count > 0 {
                *self.css_rewrites.entry(key).or_default() += count;
            }
        }
    }
}

/// The remote URLs one block remembers the decision for. A page repeats an image URL in many slots, and each decision
/// parses and hashes the URL. A URL longer than the fetcher takes is not kept, so the cache holds a few MiB at most.
const MAX_REMEMBERED_URLS: usize = 4096;

/// What a remote URL becomes, and the labels it counts under.
#[derive(Clone)]
struct Decision {
    text: String,
    outcome: &'static str,
    reason: &'static str,
}

pub struct AssetRewrite {
    team_id: i64,
    key_month: String,
    url_key: [u8; 32],
    decisions: BTreeMap<(UrlAsset, String), Decision>,
    pub counters: PassCounters,
}

impl AssetRewrite {
    /// `team_key` is the team's month key for `key_month`, the write month, as for the fetch jobs.
    pub fn new(team_id: i64, key_month: &str, team_key: &[u8; 32]) -> Self {
        Self {
            team_id,
            key_month: key_month.to_owned(),
            url_key: image_url_key(team_key),
            decisions: BTreeMap::new(),
            counters: PassCounters::default(),
        }
    }

    /// What the string of an image slot becomes. `None` keeps it as it is.
    pub fn slot(
        &mut self,
        value: &str,
        context: &AssetContext,
        sink: &mut ImageSink<'_>,
    ) -> Option<String> {
        match context {
            AssetContext::Url => self
                .asset_url(value, CssAssetKind::Image, sink)
                .map(|replaced| replaced.text),
            AssetContext::Srcset(slot) => {
                let rewrite = rewrite_srcset(value, *slot, |url| {
                    self.asset_url(url, CssAssetKind::Image, sink)
                })?;
                if let Some(outcome) = rewrite.outcome {
                    *self.counters.srcset_rewrites.entry(outcome).or_default() += 1;
                }
                Some(rewrite.value)
            }
            AssetContext::Css { kind, page_base } => {
                let mut counts = CssRewriteCounts::default();
                let rewritten = rewrite_css(
                    value,
                    *kind,
                    page_base.as_deref(),
                    &mut |asset| self.asset_url(asset.url, asset.kind, sink),
                    &mut counts,
                );
                self.counters.add_css(counts);
                rewritten
            }
        }
    }

    /// The one place an asset URL becomes what the block stores. `None` keeps a URL that loads nothing offline, such as
    /// a relative one, and an SVG data URI, which the text scrub reads in place.
    fn asset_url(
        &mut self,
        url: &str,
        kind: CssAssetKind,
        sink: &mut ImageSink<'_>,
    ) -> Option<Replacement> {
        let url = url.trim();
        if is_data_url(url) {
            return match inline_image(url, sink) {
                InlineImage::Replaced(text) => Some(Replacement {
                    declined: text.starts_with(IMAGE_DECLINED_PREFIX),
                    text,
                }),
                InlineImage::NotImage | InlineImage::Svg => None,
            };
        }
        if !is_remote(url) {
            return None;
        }
        let (asset, label) = match kind {
            CssAssetKind::Image => (UrlAsset::Image, "image"),
            CssAssetKind::Font { bundled: None } => (UrlAsset::Font, "font"),
            // The renderer ships the family, so the block needs no URL at all.
            CssAssetKind::Font {
                bundled: Some(slug),
            } => {
                *self
                    .counters
                    .url_rewrites
                    .entry(("bundled_font", "rewritten", "none"))
                    .or_default() += 1;
                return Some(Replacement {
                    text: bundled_font_ref(slug),
                    declined: false,
                });
            }
        };
        let key = (asset, url.to_owned());
        let decision = match self.decisions.get(&key) {
            Some(known) => known.clone(),
            None => {
                let decision = self.decide(url, asset);
                if url.len() <= MAX_URL_LEN && self.decisions.len() < MAX_REMEMBERED_URLS {
                    self.decisions.insert(key, decision.clone());
                }
                decision
            }
        };
        *self
            .counters
            .url_rewrites
            .entry((label, decision.outcome, decision.reason))
            .or_default() += 1;
        Some(Replacement {
            declined: decision.outcome == "declined",
            text: decision.text,
        })
    }

    fn decide(&self, url: &str, asset: UrlAsset) -> Decision {
        match url_reference(url, asset, self.team_id, &self.key_month, &self.url_key) {
            Ok((_, reference)) => Decision {
                text: reference,
                outcome: "rewritten",
                reason: "none",
            },
            Err(decline) => Decision {
                text: declined_url_ref(asset, decline.into()),
                outcome: "declined",
                reason: decline.label(),
            },
        }
    }
}
