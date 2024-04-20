//#[derive(SmartDebug, Clone)]
//pub struct HirNode {
//    #[debug(skip)]
//    pub parent: crate::interpreter::hir::WeakNode,
//    pub tag: TagName,
//    pub attributes: Vec<Attr>,
//    pub content: Vec<TextOrHirNode>,
//}

use crate::color::{native_color, Theme};
use crate::interpreter::hir::{unwrap_hir_node, Hir, HirNode, TextOrHirNode};
use crate::interpreter::html::style::{FontStyle, FontWeight, Style, TextDecoration};
use crate::interpreter::html::{Attr, attr, HeaderType, Picture, style, TagName};
use crate::interpreter::{html, Span};
use crate::positioner::{Positioned, Row, Section, Spacer, DEFAULT_MARGIN};
use crate::table::Table;
use crate::text::{Text, TextBox};
use crate::utils::Align;
use crate::{table, Element};
use comrak::Anchorizer;
use glyphon::FamilyOwned;
use std::collections::VecDeque;
use std::num::NonZeroU8;
use wgpu::TextureFormat;

#[derive(Debug, Copy, Clone, Default)]
struct TextOptions {
    pub underline: bool,
    pub bold: bool,
    pub italic: bool,
    pub strike_through: bool,
    pub small: bool,
    pub code: bool,
    pub pre_formatted: bool,
    pub block_quote: u8,
    pub align: Option<Align>,
}

#[derive(Debug, Copy, Clone, Default)]
struct InheritedState {
    global_indent: f32,
    text_options: TextOptions,
    span: Span,

    /// Li render as ether as "· " or as an "{1..}. ".
    list_prefix: Option<Option<NonZeroU8>>,
}
impl InheritedState {
    fn set_align(&mut self, align: Option<Align>) {
        self.text_options.align = align.or(self.text_options.align);
    }
}

type Content = Vec<TextOrHirNode>;
type Attributes = Vec<Attr>;

pub(crate) struct Ast {
    pub ast: VecDeque<Element>,
    pub anchorizer: Anchorizer,
    pub theme: Theme,
    pub current_textbox: TextBox,
    pub hidpi_scale: f32,
    pub surface_format: TextureFormat,
    pub link: Option<String>,
}
impl Ast {
    pub fn new() -> Self {
        Self {
            ast: VecDeque::new(),
            anchorizer: Default::default(),
            current_textbox: Default::default(),
            hidpi_scale: Default::default(),
            theme: Theme::dark_default(),
            surface_format: TextureFormat::Bgra8UnormSrgb,
            link: None,
        }
    }
    pub fn interpret(mut self, hir: Hir) -> Self {
        let content = hir.content();
        self.process_content(Default::default(), content);
        self
    }
    pub fn into_inner(self) -> VecDeque<Element> {
        self.ast
    }

    fn process_content(&mut self, inherited_state: InheritedState, content: Content) {
        for node in content {
            match node {
                TextOrHirNode::Text(str) => self.push_text(inherited_state, str),
                TextOrHirNode::Hir(node) => {
                    self.process_node(inherited_state, unwrap_hir_node(node))
                }
            }
        }
    }

    fn process_node(&mut self, mut inherited_state: InheritedState, node: HirNode) {
        let content = node.content;
        let attributes = node.attributes;

        match node.tag {
            TagName::Paragraph => {
                self.push_text_box(inherited_state);

                self.process_content(inherited_state, content);

                self.push_text_box(inherited_state);
                self.push_spacer();
            }
            TagName::Anchor => {
                for attr in attributes {
                    match attr {
                        Attr::Href(link) => self.link = Some(link),
                        Attr::Anchor(a) => self.current_textbox.set_anchor(a),
                        _ => {}
                    }
                }
                self.process_content(inherited_state, content);
            }
            TagName::Div => {
                self.push_text_box(inherited_state);
                self.process_content(inherited_state, content);
                self.push_text_box(inherited_state);
            }
            TagName::Table => {
                return;
                let table = Table::new();
                todo!("Table implementation");
                //handle_table(...)

                self.push_element(table);
                return;
            }
            //_ => {
            //    tracing::warn!("Interpreter didn't implement {:?}", node.tag);
            //    return;
            //}
            TagName::BlockQuote => {
                self.push_text_box(inherited_state);
                inherited_state.text_options.block_quote += 1;
                inherited_state.global_indent += DEFAULT_MARGIN / 2.;

                self.process_content(inherited_state, content);

                self.push_text_box(inherited_state);
                if inherited_state.global_indent == DEFAULT_MARGIN / 2. {
                    self.push_spacer();
                }
            }
            TagName::BoldOrStrong => {
                inherited_state.text_options.bold = true;
                self.process_content(inherited_state, content);
            }
            TagName::Break => {
                self.push_text_box(inherited_state);
                self.process_content(inherited_state, content);
            }
            TagName::Code => {
                inherited_state.text_options.code = true;
                self.process_content(inherited_state, content);
            }
            TagName::Details => {
                return;
                self.push_text_box(inherited_state);
                self.push_spacer();
                let section = Section::bare(self.hidpi_scale);
                *section.hidden.borrow_mut() = true;
                todo!("Details Implementation");
                // handle_details(...)
                self.push_element(section);
                return;
            }
            TagName::Summary => {
                tracing::warn!("Summary can only be in an Details element");
                return;
            }
            TagName::EmphasisOrItalic => {
                inherited_state.text_options.italic = true;
                self.process_content(inherited_state, content);
            }
            TagName::Header(header) => {
                self.push_text_box(inherited_state);
                self.push_spacer();

                inherited_state.set_align(attributes.iter().find_map(|attr| attr.to_align()));
                inherited_state.text_options.bold = true;
                self.current_textbox.font_size *= header.size_multiplier();
                
                if header == HeaderType::H1 {
                    inherited_state.text_options.underline = true;
                }
                self.process_content(inherited_state, content);

                let anchor = self
                    .current_textbox
                    .texts
                    .iter()
                    .flat_map(|t| t.text.chars())
                    .collect();
                let anchor = self.anchorizer.anchorize(anchor);
                self.current_textbox.set_anchor(format!("#{anchor}"));
                self.push_text_box(inherited_state);
                self.push_spacer();
            }
            TagName::HorizontalRuler => {
                self.push_element(Spacer::visible());
                self.process_content(inherited_state, content);
            }
            TagName::Picture => {
                tracing::warn!("");
                return;
            }
            TagName::Source => {
                tracing::warn!("");
                return;
            }
            TagName::Image => {
                tracing::warn!("");
                return;
            }
            TagName::Input => {
                let mut is_checkbox = false;
                let mut is_checked = false;
                for attr in attributes {
                    match attr {
                        Attr::IsCheckbox => is_checkbox = true,
                        Attr::IsChecked => is_checked = true,
                        _ => {}
                    }
                }
                if is_checkbox {
                    self.current_textbox.set_checkbox(Some(is_checked));
                }
                self.process_content(inherited_state, content);
            }
            TagName::ListItem => {
                
            }
            TagName::OrderedList => {
                return;
            }
            TagName::PreformattedText => {
                self.push_text_box(inherited_state);
                let style = attributes.iter().find_map(|attr| attr.to_style()).unwrap_or_default();
                for style in style::Iter::new(&style) {
                    if let Style::BackgroundColor(color) = style {
                        let native_color = self.native_color(color);
                        self.current_textbox.set_background_color(native_color);
                    }
                }
                inherited_state.text_options.pre_formatted = true;
                self.current_textbox.set_code_block(true);
                self.process_content(inherited_state, content);

                self.push_text_box(inherited_state);
                
                self.push_spacer();
                inherited_state.text_options.pre_formatted = false;
                self.current_textbox.set_code_block(false);
            }
            TagName::Section => {
                return;
            }
            TagName::Small => {
                inherited_state.text_options.small = true;
                self.process_content(inherited_state, content);
            }
            TagName::Span => {
                let style_str = attributes.iter().find_map(|attr|attr.to_style()).unwrap_or_default();
                for style in style::Iter::new(&style_str) {
                    match style {
                        Style::Color(color) => {
                            inherited_state.span.color = native_color(color, &self.surface_format)
                        }
                        Style::FontWeight(weight) => inherited_state.span.weight = weight,
                        Style::FontStyle(style) => inherited_state.span.style = style,
                        Style::TextDecoration(decor) => inherited_state.span.decor = decor,
                        _ => {}
                    }
                }
                if inherited_state.span.weight == FontWeight::Bold {
                    dbg!(&content, &inherited_state);
                }
                self.process_content(inherited_state, content);
            }
            TagName::Strikethrough => {
                inherited_state.text_options.strike_through = true;
                self.process_content(inherited_state, content);
            }
            TagName::TableBody => {
                return;
            }
            TagName::TableDataCell => {
                return;
            }
            TagName::TableHead => {
                return;
            }
            TagName::TableHeader => {
                return;
            }
            TagName::TableRow => {
                return;
            }
            TagName::Underline => {
                inherited_state.text_options.underline = true;
                self.process_content(inherited_state, content);
            }
            TagName::UnorderedList => {
                return;
            }
            TagName::Root => {
                tracing::error!("Root element can't reach interpreter.");
                return;
            }
        }
    }

    fn push_text(&mut self, state: InheritedState, mut string: String) {
        let text_native_color = self.native_color(self.theme.text_color);
        if string == "\n" {
            if state.text_options.pre_formatted {
                self.current_textbox.texts.push(Text::new(
                    "\n".to_string(),
                    self.hidpi_scale,
                    text_native_color,
                ));
            }
            if let Some(last_text) = self.current_textbox.texts.last() {
                if let Some(last_char) = last_text.text.chars().last() {
                    if !last_char.is_whitespace() {
                        self.current_textbox.texts.push(Text::new(
                            " ".to_string(),
                            self.hidpi_scale,
                            text_native_color,
                        ));
                    }
                }
            }
            // TODO
            //if let Some((row, newline_counter)) = state.inline_images.take() {
            //    if newline_counter == 0 {
            //        self.push_element(row);
            //        self.push_spacer();
            //    } else {
            //        state.inline_images = Some((row, newline_counter - 1));
            //    }
            //}
        } else if string.trim().is_empty() && !state.text_options.pre_formatted {
            if let Some(last_text) = self.current_textbox.texts.last() {
                if let Some(last_char) = last_text.text.chars().last() {
                    if !last_char.is_whitespace() {
                        self.current_textbox.texts.push(Text::new(
                            " ".to_string(),
                            self.hidpi_scale,
                            text_native_color,
                        ));
                    }
                }
            }
        } else {
            if self.current_textbox.texts.is_empty() && !state.text_options.pre_formatted {
                #[allow(
                unknown_lints, // Rust is still bad with back compat on new lints
                clippy::assigning_clones // Hit's a borrow-check issue. Needs a different impl
                )]
                {
                    string = string.trim_start().to_owned();
                }
            }

            let mut text = Text::new(string, self.hidpi_scale, text_native_color);
            // TODO
            //if let Some(prefix) = state.pending_list_prefix.take() {
            //    if self.current_textbox.texts.is_empty() {
            //        self.current_textbox.texts.push(
            //            Text::new(prefix, self.hidpi_scale, text_native_color).make_bold(true),
            //        );
            //    }
            //}
            if state.text_options.block_quote >= 1 {
                self.current_textbox
                    .set_quote_block(state.text_options.block_quote as usize);
            }
            if state.text_options.code {
                text = text
                    .with_color(state.span.color)
                    .with_family(FamilyOwned::Monospace);
                if state.span.weight == FontWeight::Bold {
                    text = text.make_bold(true);
                }
                if state.span.style == FontStyle::Italic {
                    text = text.make_italic(true);
                }
                if state.span.decor == TextDecoration::Underline {
                    text = text.make_underlined(true);
                }
            }
            // TODO
            //for elem in self.state.element_stack.iter().rev() {
            //    if let crate::interpreter::html::element::Element::Header(header) = elem {
            //        self.current_textbox.font_size *= header.ty.size_multiplier();
            //        text = text.make_bold(true);
            //        break;
            //    }
            //}
            if let Some(link) = self.link.take() {
                text = text.with_link(link.to_string());
                text = text.with_color(self.native_color(self.theme.link_color));
            }
            if state.text_options.bold {
                text = text.make_bold(true);
            }
            if state.text_options.italic {
                text = text.make_italic(true);
            }
            if state.text_options.underline {
                text = text.make_underlined(true);
            }
            if state.text_options.strike_through {
                text = text.make_striked(true);
            } 

            if state.text_options.small {
                self.current_textbox.font_size = 12.;
            }
            self.current_textbox.texts.push(text);
        }
    }

    fn push_element<T: Into<Element>>(&mut self, element: T) {
        self.ast.push_back(element.into())
    }
    fn push_text_box(&mut self, state: InheritedState) {
        //if let Some((row, count)) = self.state.inline_images.take() {
        //    if count == 0 {
        //        self.push_element(row);
        //        self.push_spacer();
        //    } else {
        //        self.state.inline_images = Some((row, count))
        //    }
        //}

        let mut tb = std::mem::replace(
            &mut self.current_textbox,
            TextBox::new(vec![], self.hidpi_scale),
        );
        self.current_textbox.indent = state.global_indent;

        if !tb.texts.is_empty() {
            let content = tb.texts.iter().any(|text| !text.text.is_empty());

            if content {
                tb.indent = state.global_indent;
                //let section = self.state.element_iter_mut().rev().find_map(|e| {
                //    if let crate::interpreter::html::element::Element::Details(section) = e {
                //        Some(section)
                //    } else {
                //        None
                //    }
                //});
                //if let Some(section) = section {
                //    section
                //        .elements
                //        .push(Positioned::new(self.current_textbox.clone()));
                //} else {
                //    self.push_element(self.current_textbox.clone());
                //}

                self.push_element(tb);
            }
        }
    }

    fn push_spacer(&mut self) {
        self.push_element(Spacer::invisible());
    }

    #[must_use]
    fn native_color(&self, color: u32) -> [f32; 4] {
        native_color(color, &self.surface_format)
    }
}

#[cfg(test)]
mod tests {
    use crate::interpreter::ast::{Ast, TextOptions};
    use crate::interpreter::hir::Hir;
    use html5ever::tendril::StrTendril;
    use html5ever::tokenizer::{BufferQueue, Tokenizer};

    fn prepare<T: Into<String>>(html: T) -> Hir {
        let mut buffer = BufferQueue::default();
        buffer.push_back(StrTendril::from(html.into()));

        let mut tokenizer = Tokenizer::new(Hir::new(), Default::default());
        let _ = tokenizer.feed(&mut buffer);
        tokenizer.end();
        tokenizer.sink
    }
}
