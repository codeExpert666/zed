use super::{CursorLayout, Invisible, layout_chunk_renderer_with_width};
use crate::{
    ChunkRenderer, ChunkReplacement,
    display_map::{ChunkRendererId, HighlightedChunk, LongRow, RendererTextSource},
};
use gpui::{
    AnyElement, App, AvailableSpace, BackgroundExecutor, Font, FontRun, HighlightStyle,
    LineWrapper, Pixels, RetainedLineLayout, SharedString, Size, TextStyle, TextSystem, Window,
};

use std::{
    ops::Range,
    sync::{Arc, OnceLock},
};
use unicode_segmentation::UnicodeSegmentation;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) struct PreparedRowText {
    text: SharedString,
    chunks: Vec<PreparedChunk>,
    invisibles: Vec<Invisible>,
    oversized_cursor_widths: Vec<(Range<u32>, OnceLock<Pixels>)>,
    drop_executor: Option<BackgroundExecutor>,
    #[cfg(test)]
    cursor_text_input_bytes: AtomicUsize,
}

#[derive(Clone)]
pub(crate) struct PreparedRendererConstraint {
    renderer: ChunkRendererId,
    text: RendererTextSource,
    font: Font,
    font_size: Pixels,
    width: Option<Pixels>,
}

impl PreparedRowText {
    pub(crate) const CURSOR_TEXT_LIMIT: u32 = 1_024;

    pub(crate) fn new<'a>(highlighted_chunks: impl Iterator<Item = HighlightedChunk<'a>>) -> Self {
        let mut text = String::new();
        let mut chunks = Vec::new();
        for chunk in highlighted_chunks {
            let columns = text.len()..text.len() + chunk.text.len();
            text.push_str(chunk.text);
            chunks.push(PreparedChunk {
                columns,
                style: chunk.style,
                diagnostic_underline_severity: chunk.diagnostic_underline_severity,
                is_tab: chunk.is_tab,
                is_inlay: chunk.is_inlay,
                replacement: chunk.replacement,
            });
        }

        let mut invisibles = Vec::new();
        for chunk in &chunks {
            if chunk.replacement.is_some() || chunk.is_inlay {
                continue;
            }
            if chunk.is_tab {
                invisibles.push(Invisible::Tab {
                    line_start_offset: chunk.columns.start,
                    line_end_offset: chunk.columns.end,
                });
            } else {
                invisibles.extend(text[chunk.columns.clone()].char_indices().filter_map(
                    |(offset, character)| {
                        character.is_whitespace().then_some(Invisible::Whitespace {
                            line_start_offset: chunk.columns.start + offset,
                            line_end_offset: chunk.columns.start + offset + character.len_utf8(),
                        })
                    },
                ));
            }
        }
        let oversized_cursor_widths = text
            .grapheme_indices(true)
            .filter(|(_, grapheme)| grapheme.len() > Self::CURSOR_TEXT_LIMIT as usize)
            .map(|(start, grapheme)| {
                (
                    start as u32..(start + grapheme.len()) as u32,
                    OnceLock::new(),
                )
            })
            .collect();
        Self {
            text: SharedString::from(text),
            chunks,
            invisibles,
            oversized_cursor_widths,
            drop_executor: None,
            #[cfg(test)]
            cursor_text_input_bytes: AtomicUsize::new(0),
        }
    }

    pub(crate) fn prepare_cursor_widths(
        &self,
        row: &LongRow,
        style: &TextStyle,
        wrapper: &LineWrapper,
        text_system: &TextSystem,
    ) {
        for (columns, width) in &self.oversized_cursor_widths {
            width.get_or_init(|| {
                let text = self.cursor_text(columns.clone());
                if text.bytes().all(|byte| byte.is_ascii_whitespace()) {
                    return Pixels::ZERO;
                }
                let font = CursorLayout::text_font(
                    row.font_id_for_column(columns.start, columns),
                    style,
                    text_system,
                );
                wrapper
                    .layout_line(
                        &text,
                        &[FontRun {
                            len: text.len(),
                            font_id: text_system.resolve_font(&font),
                        }],
                    )
                    .width
            });
        }
    }

    pub(crate) fn with_drop_executor(mut self, executor: BackgroundExecutor) -> Self {
        self.drop_executor = Some(executor);
        self
    }

    pub(crate) fn chunks(
        &self,
        columns: Range<usize>,
    ) -> impl Iterator<Item = HighlightedChunk<'_>> {
        let start = columns.start;
        let end = columns.end;
        let first = self
            .chunks
            .partition_point(|chunk| chunk.columns.end <= start);
        self.chunks[first..]
            .iter()
            .take_while(move |chunk| start < end && chunk.columns.start < end)
            .map(move |chunk| HighlightedChunk {
                text: &self.text[chunk.columns.start.max(start)..chunk.columns.end.min(end)],
                style: chunk.style,
                diagnostic_underline_severity: chunk.diagnostic_underline_severity,
                is_tab: chunk.is_tab,
                is_inlay: chunk.is_inlay,
                replacement: chunk.replacement.clone(),
            })
    }

    #[cfg(test)]
    pub(crate) fn prepare_text_slice(&self, columns: Range<usize>) -> SharedString {
        if columns == (0..self.text.len()) {
            self.text.clone()
        } else {
            SharedString::new(&self.text[columns])
        }
    }

    pub(super) fn oversized_cursor_width(&self, column: u32) -> Option<Option<Pixels>> {
        let index = self
            .oversized_cursor_widths
            .partition_point(|(columns, _)| columns.end <= column);
        let (columns, width) = self.oversized_cursor_widths.get(index)?;
        columns.contains(&column).then(|| width.get().copied())
    }

    pub(super) fn cursor_text(&self, columns: Range<u32>) -> SharedString {
        let end = self.text.floor_char_boundary(columns.end as usize);
        let text = &self.text[columns.start as usize..end];
        #[cfg(test)]
        self.cursor_text_input_bytes
            .fetch_add(text.len(), Ordering::Relaxed);
        let grapheme = text.graphemes(true).next().unwrap_or_default();
        if let Some(invisible) = grapheme
            .chars()
            .next()
            .filter(|character| crate::display_map::is_invisible(*character))
            && let Some(replacement) = crate::display_map::replacement(invisible)
        {
            SharedString::from(replacement)
        } else if grapheme == "\n" {
            SharedString::new_static(" ")
        } else {
            SharedString::new(grapheme)
        }
    }

    #[cfg(test)]
    pub(super) fn cursor_text_input_bytes(&self) -> usize {
        self.cursor_text_input_bytes.load(Ordering::Relaxed)
    }

    pub(super) fn invisibles(&self, columns: Range<usize>) -> impl Iterator<Item = Invisible> + '_ {
        let first = self
            .invisibles
            .partition_point(|invisible| invisible.range().end <= columns.start);
        self.invisibles[first..]
            .iter()
            .copied()
            .take_while(move |invisible| {
                columns.start < columns.end && invisible.range().start < columns.end
            })
    }
}

impl Drop for PreparedRowText {
    fn drop(&mut self) {
        if let Some(executor) = &self.drop_executor
            && executor.is_main_thread()
        {
            let retained = (
                std::mem::take(&mut self.text),
                std::mem::take(&mut self.chunks),
                std::mem::take(&mut self.invisibles),
                std::mem::take(&mut self.oversized_cursor_widths),
            );
            executor
                .spawn(async move {
                    drop(retained);
                })
                .detach();
        }
    }
}

impl PreparedRendererConstraint {
    pub(crate) fn new(
        renderer: ChunkRenderer,
        text: RendererTextSource,
        font: Font,
        font_size: Pixels,
    ) -> Self {
        Self {
            renderer: renderer.id,
            text,
            font,
            font_size,
            width: None,
        }
    }

    pub(crate) fn text(&self) -> &RendererTextSource {
        &self.text
    }

    pub(crate) fn width(&self) -> Option<Pixels> {
        self.width
    }

    pub(crate) fn matches_font(&self, font: &Font, font_size: Pixels) -> bool {
        self.font == *font && self.font_size == font_size
    }

    pub(crate) fn matches(
        &self,
        renderer: &ChunkRenderer,
        text: impl Into<RendererTextSource>,
        font: &Font,
        font_size: Pixels,
    ) -> bool {
        self.renderer == renderer.id
            && self.text.same_source(&text.into())
            && self.matches_font(font, font_size)
    }

    pub(crate) fn measure(&self, text_system: &Arc<TextSystem>) -> Self {
        let text = self.text.materialize();
        let wrapper = text_system.line_wrapper(self.font.clone(), self.font_size);
        let runs = [FontRun {
            len: text.len(),
            font_id: wrapper.font_id(),
        }];
        let width = match wrapper.layout_line_for_retention(&text, &runs) {
            RetainedLineLayout::Dense(_, _, width) => width,
            RetainedLineLayout::Compact(layout) => layout.width,
        };
        Self {
            width: Some(Pixels::from(width)),
            ..self.clone()
        }
    }

    pub(crate) fn layout(
        renderer: &ChunkRenderer,
        constraint: Option<&Self>,
        text: impl Into<RendererTextSource>,
        text_style: &TextStyle,
        text_width: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<(AnyElement, Size<Pixels>)> {
        let available_width = if renderer.constrain_width {
            let constraint = constraint.filter(|constraint| {
                constraint.matches(
                    renderer,
                    text,
                    &text_style.font(),
                    text_style.font_size.to_pixels(window.rem_size()),
                )
            })?;
            AvailableSpace::Definite(constraint.width()?)
        } else {
            AvailableSpace::MinContent
        };
        Some(layout_chunk_renderer_with_width(
            renderer,
            available_width,
            text_style,
            text_width,
            window,
            cx,
        ))
    }
}

struct PreparedChunk {
    columns: Range<usize>,
    style: Option<HighlightStyle>,
    diagnostic_underline_severity: Option<lsp::DiagnosticSeverity>,
    is_tab: bool,
    is_inlay: bool,
    replacement: Option<ChunkReplacement>,
}
