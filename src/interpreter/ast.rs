use std::cell::{Cell, Ref, RefCell};
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
use std::num::{NonZeroU8, NonZeroUsize};
use std::ops::DerefMut;
use html5ever::tendril::{SliceExt, StrTendril};
use lyon::geom::utils::tangent;
use wgpu::TextureFormat;

#[derive(Debug, Clone, Default)]
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
    pub link: Option<StrTendril>,
}

#[derive(Debug, Clone, Default)]
struct InheritedState {
    global_indent: f32,
    text_options: TextOptions,
    span: Span,

    /// Li render as ether as "· " or as an "{1..}. ".
    list_prefix: Option<Option<NonZeroUsize>>,
}
impl InheritedState {
    fn with_span_color(span_color: [f32; 4]) -> Self {
        Self {
            span: Span::with_color(span_color),
            ..Default::default()
        }
    }
}

impl InheritedState {
    fn set_align(&mut self, align: Option<Align>) {
        self.text_options.align = align.or(self.text_options.align);
    }
}

type Content = Vec<TextOrHirNode>;
type Attributes = Vec<Attr>;

pub struct Ast {
    pub ast: VecDeque<Element>,
    pub anchorizer: Anchorizer,
    pub theme: Theme,
    pub current_textbox: RefCell<TextBox>,
    pub hidpi_scale: f32,
    pub surface_format: TextureFormat,
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
        }
    }
    pub fn interpret(mut self, hir: Hir) -> Self {
        let content = hir.content();
        let state = InheritedState::with_span_color(self.native_color(self.theme.code_color));

        self.process_content(state, content);
        self
    }
    pub fn into_inner(self) -> VecDeque<Element> {
        self.ast
    }

    fn process_content(&mut self, inherited_state: InheritedState, content: Content) {
        for node in content {
            match node {
                TextOrHirNode::Text(str) => self.text(self.current_textbox.borrow_mut().deref_mut(), inherited_state.clone(), str),
                TextOrHirNode::Hir(node) => {
                    self.process_node(inherited_state.clone(), unwrap_hir_node(node))
                }
            }
        }
    }

    fn process_node(&mut self, mut inherited_state: InheritedState, node: HirNode) {
        let content = node.content;
        let attributes = node.attributes;

        match node.tag {
            TagName::Paragraph => {
                self.push_text_box(inherited_state.clone());

                self.process_content(inherited_state.clone(), content);

                self.push_text_box(inherited_state.clone());
                self.push_spacer();
            }
            TagName::Anchor => {
                for attr in attributes {
                    match attr {
                        Attr::Href(link) => inherited_state.text_options.link = Some(link.to_tendril()),
                        Attr::Anchor(a) => self.current_textbox.borrow_mut().set_anchor(a),
                        _ => {}
                    }
                }
                self.process_content(inherited_state, content);
            }
            TagName::Div => {
                self.push_text_box(inherited_state.clone());
                self.process_content(inherited_state.clone(), content);
                self.push_text_box(inherited_state);
            }
            //_ => {
            //    tracing::warn!("Interpreter didn't implement {:?}", node.tag);
            //    return;
            //}
            TagName::BlockQuote => {
                self.push_text_box(inherited_state.clone());
                inherited_state.text_options.block_quote += 1;
                inherited_state.global_indent += DEFAULT_MARGIN / 2.;

                self.process_content(inherited_state.clone(), content);

                let indent = inherited_state.global_indent;
                
                self.push_text_box(inherited_state);
                
                if indent == DEFAULT_MARGIN / 2. {
                    self.push_spacer();
                }
            }
            TagName::BoldOrStrong => {
                inherited_state.text_options.bold = true;
                self.process_content(inherited_state, content);
            }
            TagName::Break => {
                self.push_text_box(inherited_state.clone());
                self.process_content(inherited_state, content);
            }
            TagName::Code => {
                inherited_state.text_options.code = true;
                self.process_content(inherited_state, content);
            }
            TagName::Details => { //TODO
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
            TagName::Section => { //TODO
                return;
            }
            TagName::EmphasisOrItalic => {
                inherited_state.text_options.italic = true;
                self.process_content(inherited_state, content);
            }
            TagName::Header(header) => {
                self.push_text_box(inherited_state.clone());
                self.push_spacer();

                inherited_state.set_align(attributes.iter().find_map(|attr| attr.to_align()));
                inherited_state.text_options.bold = true;
                self.current_textbox.borrow_mut().font_size *= header.size_multiplier();

                if header == HeaderType::H1 {
                    inherited_state.text_options.underline = true;
                }
                self.process_content(inherited_state.clone(), content);

                let anchor = self
                    .current_textbox.borrow()
                    .texts
                    .iter()
                    .flat_map(|t| t.text.chars())
                    .collect();
                let anchor = self.anchorizer.anchorize(anchor);
                self.current_textbox.borrow_mut().set_anchor(format!("#{anchor}"));
                self.push_text_box(inherited_state);
                self.push_spacer();
            }
            TagName::HorizontalRuler => {
                self.push_element(Spacer::visible());
                self.process_content(inherited_state, content);
            }
            TagName::Picture => {
                tracing::warn!("No picture impl");
                return;
            }
            TagName::Source => {
                tracing::warn!("No source impl");
                return;
            }
            TagName::Image => {
                tracing::warn!("No image impl");
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
                    self.current_textbox.borrow_mut().set_checkbox(Some(is_checked));
                }
                self.process_content(inherited_state, content);
            }
            TagName::ListItem => {
                tracing::warn!("ListItem can only be in an List element");
                return;
            }
            TagName::OrderedList => {
                self.process_ordered_list(inherited_state, content, attributes);
            }
            TagName::UnorderedList => {
                self.process_unordered_list(inherited_state, content, attributes);
            }
            TagName::PreformattedText => {
                self.push_text_box(inherited_state.clone());
                let style = attributes.iter().find_map(|attr| attr.to_style()).unwrap_or_default();
                for style in style::Iter::new(&style) {
                    if let Style::BackgroundColor(color) = style {
                        let native_color = self.native_color(color);
                        self.current_textbox.borrow_mut().set_background_color(native_color);
                    }
                }
                inherited_state.text_options.pre_formatted = true;
                self.current_textbox.borrow_mut().set_code_block(true);
                self.process_content(inherited_state.clone(), content);

                self.push_text_box(inherited_state);
                self.push_spacer();
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
                self.process_content(inherited_state, content);
            }
            TagName::Strikethrough => {
                inherited_state.text_options.strike_through = true;
                self.process_content(inherited_state, content);
            }
            TagName::Table => {
                let mut table = Table::new();
                self.process_table(&mut table, inherited_state, content);
                self.push_spacer();
                self.push_element(table);
                self.push_spacer();
                return;
            }
            TagName::TableHead | TagName::TableBody => {
                tracing::warn!("TableHead and TableBody not supported");
                return;
            }
            TagName::TableRow => {
                tracing::warn!("Summary can only be in an Table element");
                return;
            }
            TagName::TableDataCell => {
                tracing::warn!("Summary can only be in an TableRow or an TableHeader element");
                return;
            }
            TagName::TableHeader => {
                tracing::warn!("Summary can only be in an TableRow element");
                return;
            }
            TagName::Underline => {
                inherited_state.text_options.underline = true;
                self.process_content(inherited_state, content);
            }
            TagName::Root => tracing::error!("Root element can't reach interpreter."),
        }
    }

    fn text(&self, text_box: &mut TextBox, mut state: InheritedState, mut string: String){
        let text_native_color = self.native_color(self.theme.text_color);
        if string == "\n" {
            if state.text_options.pre_formatted {
                text_box.texts.push(Text::new(
                    "\n".to_string(),
                    self.hidpi_scale,
                    text_native_color,
                ));
            }
            if let Some(last_text) = text_box.texts.last() {
                if let Some(last_char) = last_text.text.chars().last() {
                    if !last_char.is_whitespace() {
                        text_box.texts.push(Text::new(
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
            if let Some(last_text) = text_box.texts.last() {
                if let Some(last_char) = last_text.text.chars().last() {
                    if !last_char.is_whitespace() {
                        text_box.texts.push(Text::new(
                            " ".to_string(),
                            self.hidpi_scale,
                            text_native_color,
                        ));
                    }
                }
            }
        } else {
            if text_box.texts.is_empty() && !state.text_options.pre_formatted {
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
                text_box
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
            if let Some(link) = state.text_options.link.take() {
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
                text_box.font_size = 12.;
            }
            text_box.texts.push(text);
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
            self.current_textbox.borrow_mut().deref_mut(),
            TextBox::new(vec![], self.hidpi_scale),
        );
        self.current_textbox.borrow_mut().indent = state.global_indent;

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

    // https://html.spec.whatwg.org/multipage/tables.html#the-table-element
    fn process_table(&mut self, table: &mut Table, inherited_state: InheritedState, content: Content) {
        Self::process_node_content(
            content,
                |_| {},
                |node| match node.tag {
                TagName::TableHead | TagName::TableBody => {
                    self.process_table_head_body(table, inherited_state.clone(), node.content);
                }
                TagName::TableRow => {
                    table.rows.push(vec![]);
                    self.process_table_row(table, inherited_state.clone(), node.content)
                }
                _ => tracing::warn!("Only TableHead, TableBody, TableRow and TableFoot can be inside an table, found: {:?}", node.tag),
            }
        );
        // TODO: filter out empty rows. (without cloning)
    }
    fn process_table_head_body(&mut self, table: &mut Table, inherited_state: InheritedState, content: Content) {
        Self::process_node_content(
            content,
            |_| {},
            |node| match node.tag {
                TagName::TableRow => {
                    table.rows.push(vec![]);
                    self.process_table_row(table, inherited_state.clone(), node.content)
                }
                _ => tracing::warn!("Only TableRows can be inside an TableHead or TableBody, found: {:?}", node.tag)
            }
        );
    }

    // https://html.spec.whatwg.org/multipage/tables.html#the-tr-element
    fn process_table_row(&mut self, table: &mut Table, inherited_state: InheritedState, content: Content) {
        Self::process_node_content(
            content,
            |_| {},
            |node| {
                let mut inherited_state = inherited_state.clone();
                inherited_state.set_align(node.attributes.iter().find_map(|attr| attr.to_align()));
                match node.tag {
                    TagName::TableHeader => self.process_table_header(table, inherited_state, node.content),
                    TagName::TableDataCell => self.process_table_cell(table, inherited_state, node.content),
                    _ => tracing::warn!("Only TableHead, TableBody, TableRow and TableFoot can be inside an table, found: {:?}", node.tag),
                }
            }
        );
    }

    // https://html.spec.whatwg.org/multipage/tables.html#the-th-element
    fn process_table_header(&mut self, table: &mut Table, mut inherited_state: InheritedState, content: Content) {
        let row = table.rows.last_mut().expect("There should be at least one row.");
        // TODO allow anything inside tables not only text.
        inherited_state.text_options.bold = true;
        Self::process_node_content(
            content,
            |text| {
                let mut tb = TextBox::new(vec![], self.hidpi_scale);
                self.text(&mut tb, inherited_state.clone(), text);
                row.push(tb);
            },
            |_| tracing::warn!("Currently only text is allowed in an TableHeader."),
        );
    }
    
    // https://html.spec.whatwg.org/multipage/tables.html#the-td-element
    fn process_table_cell(&mut self, table: &mut Table, inherited_state: InheritedState, content: Content) {
        let row = table.rows.last_mut().expect("There should be at least one row.");
        // TODO allow anything inside tables not only text.
        // when doing this make process_node generic over some output so it can be use here

        Self::process_node_content(
            content,
            |text| {
                let mut tb = TextBox::new(vec![], self.hidpi_scale);
                self.text(&mut tb, inherited_state.clone(), text);
                row.push(tb);
            },
            |_| tracing::warn!("Currently only text is allowed in an TableDataCell.")
        );
    }
    fn process_ordered_list(&mut self, mut inherited_state: InheritedState, content: Content, attributes: Attributes) {
        let mut index = 1;
        for attr in attributes {
            if let Attr::Start(start) = attr {
                index = start;
            }
        }
        
        Self::process_node_content(
            content,
            |_| {}, 
            |node| match node.tag {
                TagName::ListItem => {
                    inherited_state.list_prefix = Some(NonZeroUsize::try_from(index).ok());
                    self.process_list_item(inherited_state.clone(), node.content, node.attributes);
                    index += 1;
                }
                _ => tracing::warn!("Only ListItems can be inside an List"),
            }
        )
    }
    fn process_unordered_list(&mut self, mut inherited_state: InheritedState, content: Content, attributes: Attributes) {
        Self::process_node_content(content, 
            |_| {},
            |node| match node.tag {
                TagName::ListItem => {
                    inherited_state.list_prefix = Some(None);
                    self.process_list_item(inherited_state.clone(), node.content, node.attributes);
                },
                _ => tracing::warn!("Only ListItems can be inside an List"),
            }
        );
    }
    fn process_list_item(&mut self, inherited_state: InheritedState, content: Content, attributes: Attributes) {
        tracing::warn!("No li impl");
        //let anchor = attributes.iter().find_map(|attr| attr.to_anchor());
        //self.push_text_box(inherited_state);
        //self.current_textbox.borrow_mut()
    }
    
    fn process_node_content<T, N>(content: Content, mut text_fn: T, mut node_fn: N)
        where T: FnMut(String), N: FnMut(HirNode)
    {
        for node in content {
            match node {
                TextOrHirNode::Text(text) => text_fn(text),
                TextOrHirNode::Hir(node) => node_fn(unwrap_hir_node(node)),
            }
        }
    }
}