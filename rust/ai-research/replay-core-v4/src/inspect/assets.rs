//! Where each asset URL sits in a block's events. The inspector records every JSON string that an image slot reads, and
//! the writer replaces each one at that position, so no part of the recorder searches the stored bytes for a URL.

use std::sync::Arc;

use url::Url;

use super::css::{may_hold_image, CssKind};
use super::srcset::SrcsetSlot;
use crate::bytescan::Span;
use crate::scan::images::is_data_url;
use crate::url_policy::is_remote;

/// The slots one block keeps. Each takes a few dozen bytes, so this bounds what a hostile block can make the recorder
/// hold. The writer drops a block that has more.
pub const MAX_BLOCK_ASSET_SLOTS: usize = 1_000_000;

/// Why a block must not be stored: it can hold an asset URL that has no slot, which the writer would not replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unstorable {
    /// More slots than [`MAX_BLOCK_ASSET_SLOTS`].
    TooManyAssetSlots,
    /// CSS that nests past the depth `cssparser` reads.
    NestedTooDeep,
}

impl Unstorable {
    /// The outcome of the dropped block in the recorder's metrics.
    pub fn label(self) -> &'static str {
        match self {
            Unstorable::TooManyAssetSlots => "too_many_asset_slots",
            Unstorable::NestedTooDeep => "too_many_nested_blocks",
        }
    }
}

/// The bytes a slot's span indexes: the message's inner JSON, or the decoded JSON of the event's compressed string that
/// sits at this span of the inner JSON.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Buffer {
    #[default]
    Inner,
    Decoded(Span),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetContext {
    /// One URL: `src`, `rr_src`, `href`, `xlink:href` or `poster`.
    Url,
    Srcset(SrcsetSlot),
    /// `page_base` is what a root-relative URL resolves against: that of the event's window when the event came.
    Css {
        kind: CssKind,
        page_base: Option<Arc<Url>>,
    },
}

impl AssetContext {
    /// Whether a string in this slot can hold a URL that the writer replaces, so that most strings take no slot.
    pub fn may_hold_asset(&self, text: &str) -> bool {
        match self {
            AssetContext::Url => {
                let url = text.trim();
                is_remote(url) || is_data_url(url)
            }
            AssetContext::Srcset(_) => !text.trim().is_empty(),
            AssetContext::Css { .. } => may_hold_image(text),
        }
    }
}

/// A JSON string, its quotes included, that an image slot reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSlot {
    /// The event's position in its message.
    pub event: u32,
    pub buffer: Buffer,
    pub span: Span,
    pub context: AssetContext,
}

/// What the writer puts where an asset URL was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub text: String,
    /// The text is a declined marker, so the slot shows no image there.
    pub declined: bool,
}
