//! Width-independent display content retained after an MCP call completes.
//!
//! The wire result can contain multi-megabyte image, audio, or resource bodies. Validate each
//! block once with the MCP model, then retain only what history rendering actually displays.

use crate::text_formatting::format_json_compact;
use base64::Engine;
use codex_protocol::mcp::CallToolResult;
use image::DynamicImage;
use image::ImageReader;
use rmcp::model::ContentBlock;
use rmcp::model::ResourceContents;
use serde::Deserialize;
use std::borrow::Cow;
use std::io::Cursor;
use tracing::error;
use url::Url;

const MAX_BROWSER_SESSION_ID_BYTES: usize = 256;
const MAX_BROWSER_URL_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub(super) struct BrowserSummary {
    pub(super) pages: u32,
    pub(super) state: String,
    pub(super) access: Option<String>,
    pub(super) live_url: Option<String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum McpResultKind {
    Standard,
    NodeRepl,
}

#[derive(Debug)]
pub(super) struct McpToolResult {
    pub(super) content: Vec<McpContentBlock>,
    pub(super) is_error: bool,
    pub(super) has_image: bool,
    pub(super) browser: Option<BrowserSummary>,
}

#[derive(Debug)]
pub(super) struct McpContentBlock {
    display: McpContentDisplay,
    pub(super) is_image: bool,
    /// Code mode uses a top-level `text` field even on malformed or non-text blocks.
    original_text: Option<String>,
}

#[derive(Debug)]
enum McpContentDisplay {
    Text(String),
    Summary(Cow<'static, str>),
    Json(String),
}

impl McpToolResult {
    /// Consumes a wire result, dropping bodies represented only by media or resource summaries.
    ///
    /// Canonical MCP deserialization preserves full text and the exact JSON fallback for malformed
    /// or unknown blocks. Every block is projected, but image decoding stops at the first fully
    /// valid image.
    pub(super) fn new(result: CallToolResult, kind: McpResultKind) -> Self {
        let browser = result
            .structured_content
            .as_ref()
            .and_then(parse_browser_summary);
        let mut has_image = false;
        let content = result
            .content
            .into_iter()
            .map(|block| {
                // Deserialize by reference so malformed blocks remain available for the exact
                // JSON fallback. Successful blocks no longer retain their wire representation.
                let parsed = ContentBlock::deserialize(&block);
                let is_image = matches!(&parsed, Ok(ContentBlock::Image(_)));
                let original_text = match (&parsed, kind) {
                    (Ok(ContentBlock::Text(_)), _) | (_, McpResultKind::Standard) => None,
                    (_, McpResultKind::NodeRepl) => block
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                };
                let display = match parsed {
                    Ok(ContentBlock::Text(text)) => McpContentDisplay::Text(text.text),
                    Ok(ContentBlock::Image(image)) => {
                        // Computer activity previews require a valid image. Stop decoding once
                        // one has been found; the result text only retains a readable marker.
                        if !has_image {
                            has_image = decode_mcp_image(&image.data).is_some();
                        }
                        McpContentDisplay::Summary("Returned image".into())
                    }
                    Ok(ContentBlock::Audio(_)) => {
                        McpContentDisplay::Summary("<audio content>".into())
                    }
                    Ok(ContentBlock::Resource(resource)) => {
                        let summary = match resource.resource {
                            ResourceContents::TextResourceContents { uri, .. }
                            | ResourceContents::BlobResourceContents { uri, .. } => {
                                format!("embedded resource: {uri}").into()
                            }
                            _ => "<unknown embedded resource>".into(),
                        };
                        McpContentDisplay::Summary(summary)
                    }
                    Ok(ContentBlock::ResourceLink(link)) => {
                        McpContentDisplay::Summary(format!("link: {}", link.uri).into())
                    }
                    Ok(_) | Err(_) => McpContentDisplay::Json(block.to_string()),
                };
                McpContentBlock {
                    display,
                    is_image,
                    original_text,
                }
            })
            .collect();

        Self {
            content,
            is_error: result.is_error.unwrap_or(false),
            has_image,
            browser,
        }
    }
}

fn parse_browser_summary(value: &serde_json::Value) -> Option<BrowserSummary> {
    let browser = value.get("browser")?;
    let session_id = browser.get("sessionId")?.as_str()?;
    if session_id.is_empty() || session_id.len() > MAX_BROWSER_SESSION_ID_BYTES {
        return None;
    }
    let pages = u32::try_from(browser.get("pages")?.as_array()?.len()).ok()?;
    let viewer = browser.get("viewer")?;
    let state = viewer.get("state")?.as_str()?;
    if !matches!(
        state,
        "ready" | "reconnecting" | "error" | "closed" | "disabled"
    ) {
        return None;
    }
    let access = browser
        .get("access")
        .and_then(serde_json::Value::as_str)
        .filter(|access| matches!(*access, "pod-local" | "extension"))
        .map(str::to_owned);
    let live_url = matches!(state, "ready" | "reconnecting")
        .then(|| browser.get("liveUrl").and_then(serde_json::Value::as_str))
        .flatten()
        .and_then(validate_browser_url);
    Some(BrowserSummary {
        pages,
        state: state.to_owned(),
        access,
        live_url,
    })
}

fn validate_browser_url(value: &str) -> Option<String> {
    if value.len() > MAX_BROWSER_URL_BYTES || value.chars().any(char::is_control) {
        return None;
    }
    let url = Url::parse(value).ok()?;
    let authority = value.split_once("://")?.1.split(['/', '?', '#']).next()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || authority.contains('@')
    {
        return None;
    }
    Some(value.to_owned())
}

impl McpContentBlock {
    /// Returns all retained text, with honest markers for media and embedded resource bodies.
    pub(super) fn render_full(&self) -> &str {
        if let Some(text) = &self.original_text {
            return text.trim_end_matches('\n');
        }
        match &self.display {
            McpContentDisplay::Text(text) | McpContentDisplay::Json(text) => {
                text.trim_end_matches('\n')
            }
            McpContentDisplay::Summary(summary) => summary,
        }
    }

    /// Returns the untruncated top-level text used by node_repl and cua_repl's compact and
    /// transcript views.
    ///
    /// Valid text blocks reuse their display storage. Only node_repl and cua_repl retain this extra
    /// field on malformed or non-text blocks, where it takes precedence over the usual display
    /// content.
    pub(super) fn text(&self) -> Option<&str> {
        match &self.display {
            McpContentDisplay::Text(text) => Some(text),
            McpContentDisplay::Summary(_) | McpContentDisplay::Json(_) => {
                self.original_text.as_deref()
            }
        }
    }

    /// Formats the complete text or fallback JSON. The caller applies a single preview limit
    /// across all blocks, while transcript and raw output retain the full result.
    pub(super) fn render(&self) -> Cow<'_, str> {
        match &self.display {
            McpContentDisplay::Text(text) | McpContentDisplay::Json(text) => {
                format_json_compact(text)
                    .map(Cow::Owned)
                    .unwrap_or(Cow::Borrowed(text))
            }
            McpContentDisplay::Summary(summary) => Cow::Borrowed(summary),
        }
    }
}

/// Fully decodes an MCP image before exposing an image preview in computer activity.
///
/// A header-only check would accept images whose decoder rejects their pixel data. Preserve the
/// existing behavior for invalid base64, unknown formats, corrupt images, and data URLs.
fn decode_mcp_image(data: &str) -> Option<DynamicImage> {
    let base64_data = if let Some(data_url) = data.strip_prefix("data:") {
        data_url.split_once(',')?.1
    } else {
        data
    };
    let raw_data = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| {
            error!("Failed to decode image data: {e}");
            e
        })
        .ok()?;
    let reader = ImageReader::new(Cursor::new(raw_data))
        .with_guessed_format()
        .map_err(|e| {
            error!("Failed to guess image format: {e}");
            e
        })
        .ok()?;

    reader
        .decode()
        .map_err(|e| {
            error!("Image decoding failed: {e}");
            e
        })
        .ok()
}
