use crate::color::{native_color, Theme};
use crate::interpreter::hir::{Hir, HirNode, TextOrHirNode};
use crate::interpreter::html::style::{FontStyle, FontWeight, Style, TextDecoration};
use crate::interpreter::html::{style, Attr, HeaderType, Picture, TagName};
use crate::interpreter::{html, Span};
use crate::positioner::{Positioned, Row, Section, Spacer, DEFAULT_MARGIN};
use crate::table::Table;
use crate::text::{Text, TextBox};
use crate::utils::Align;
use crate::{table, Element};
use comrak::Anchorizer;
use glyphon::FamilyOwned;
use html5ever::tendril::{SliceExt, StrTendril};
use lyon::geom::utils::tangent;
use parking_lot::Mutex;
use rayon::prelude::*;
use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};
use std::collections::VecDeque;
use std::num::{NonZeroU8, NonZeroUsize};
use std::ops::DerefMut;
use wgpu::TextureFormat;
use winit::event::VirtualKeyCode::F;

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
    pub link: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct InheritedState {
    global_indent: f32,
    text_options: TextOptions,
    span: Span,
}
impl InheritedState {
    fn with_span_color(span_color: [f32; 4]) -> Self {
        Self {
            span: Span::with_color(span_color),
            ..Default::default()
        }
    }
    fn set_align(&mut self, align: Option<Align>) {
        self.text_options.align = align.or(self.text_options.align);
    }
    fn set_align_from_attributes(&mut self, attributes: Attributes) {
        self.set_align(attributes.iter().find_map(|attr| attr.to_align()));
    }
}

type Content<'a> = &'a [TextOrHirNode];
type Attributes<'a> = &'a [Attr];
pub type Output<'a> = &'a mut Vec<Element>;
pub type Input<'a> = &'a [HirNode];
type State<'a> = Cow<'a, InheritedState>;
type Opts<'a> = &'a AstOpts;

pub struct AstOpts {
    pub anchorizer: Mutex<Anchorizer>,
    pub theme: Theme,
    pub hidpi_scale: f32,
    pub surface_format: TextureFormat,
}
impl AstOpts {
    fn new() -> Self {
        Self {
            anchorizer: Default::default(),
            hidpi_scale: Default::default(),
            theme: Theme::dark_default(),
            surface_format: TextureFormat::Bgra8UnormSrgb,
        }
    }
    fn native_color(&self, color: u32) -> [f32; 4] {
        native_color(color, &self.surface_format)
    }
}

pub struct Ast {
    pub opts: AstOpts,
}
impl Ast {
    pub fn new() -> Self {
        Self {
            opts: AstOpts::new(),
        }
    }
    pub fn interpret(&self, hir: Hir) -> Vec<Element> {
        let nodes = hir.content();
        let root = nodes.first().unwrap().content.clone();
        let mut state = State::Owned(InheritedState::with_span_color(
            self.opts.native_color(self.opts.theme.code_color),
        ));

        root.into_par_iter()
            .filter_map(|ton| {
                if let TextOrHirNode::Hir(node) = ton {
                    let mut out = vec![];
                    let mut tb = TextBox::new(vec![], self.opts.hidpi_scale);
                    FlowProcess::process(
                        &nodes,
                        &mut out,
                        &self.opts,
                        &mut tb,
                        FlowProcess::get_node(&nodes, node),
                        state.clone(),
                    );
                    Some(out)
                } else {
                    None
                }
            })
            .flatten()
            .collect()
    }
}

trait Process {
    type Context<'a>;
    fn process(
        input: Input,
        output: Output,
        opts: Opts,
        context: Self::Context<'_>,
        node: &HirNode,
        state: State,
    );
    fn process_content(input: Input, output: Output, opts: Opts, context: Self::Context<'_>,
        content: Content,
        state: State,
    ) { unimplemented!() }

    fn process_node<T, N>(input: Input, node: &HirNode, mut text_fn: T, mut node_fn: N)
    where
        T: FnMut(&String),
        N: FnMut(&HirNode),
    {
        node.content.iter().for_each(|node| match node {
            TextOrHirNode::Text(text) => text_fn(text),
            TextOrHirNode::Hir(node) => node_fn(Self::get_node(input, *node)),
        })
    }
    fn get_node(input: Input, index: usize) -> &HirNode {
        input.get(index).unwrap()
    }
    fn push_element<T: Into<Element>>(output: Output, element: T) {
        output.push(element.into())
    }
    fn push_spacer(output: Output) {
        Self::push_element(output, Spacer::invisible())
    }
    fn text(text_box: &mut TextBox, mut string: &str, opts: Opts, mut state: State) {
        let text_native_color = opts.native_color(opts.theme.text_color);
        if string == "\n" {
            if state.text_options.pre_formatted {
                text_box.texts.push(Text::new(
                    "\n".to_string(),
                    opts.hidpi_scale,
                    text_native_color,
                ));
            }
            if let Some(last_text) = text_box.texts.last() {
                if let Some(last_char) = last_text.text.chars().last() {
                    if !last_char.is_whitespace() {
                        text_box.texts.push(Text::new(
                            " ".to_string(),
                            opts.hidpi_scale,
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
                            opts.hidpi_scale,
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
                    string = string.trim_start();
                }
            }

            let mut text = Text::new(string.to_string(), opts.hidpi_scale, text_native_color);

            if state.text_options.block_quote >= 1 {
                text_box.set_quote_block(state.text_options.block_quote as usize);
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
            if let Some(link) = state.to_mut().text_options.link.take() {
                text = text.with_link(link.to_string());
                text = text.with_color(opts.native_color(opts.theme.link_color));
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
    fn push_text_box(output: Output, text_box: &mut TextBox, opts: Opts, state: &State) {
        //if let Some((row, count)) = self.state.inline_images.take() {
        //    if count == 0 {
        //        self.push_element(row);
        //        self.push_spacer();
        //    } else {
        //        self.state.inline_images = Some((row, count))
        //    }
        //}

        let mut tb = std::mem::replace(text_box, TextBox::new(vec![], opts.hidpi_scale));
        text_box.indent = state.global_indent;

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

                Self::push_element(output, tb);
            }
        } else {
            text_box.is_checkbox = tb.is_checkbox;
        }
    }
}

struct FlowProcess;
impl Process for FlowProcess {
    type Context<'a> = &'a mut TextBox;
    fn process<'a>(
        input: Input,
        output: Output,
        opts: Opts,
        context: Self::Context<'a>,
        node: &HirNode,
        mut state: State,
    ) {
        let attributes = &node.attributes;
        match node.tag {
            TagName::Paragraph => {
                Self::push_text_box(output, context, opts, &state);
                state.to_mut().set_align_from_attributes(attributes);
                context.set_align_or_default(state.text_options.align);

                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());

                Self::push_text_box(output, context, opts, &state);
                Self::push_spacer(output);
            }
            TagName::Anchor => {
                for attr in attributes {
                    match attr {
                        Attr::Href(link) => {
                            state.to_mut().text_options.link = Some(link.to_owned())
                        }
                        Attr::Anchor(a) => context.set_anchor(a.to_owned()),
                        _ => {}
                    }
                }
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
            }
            TagName::Div => {
                Self::push_text_box(output, context, opts, &state);

                state.to_mut().set_align_from_attributes(&attributes);
                context.set_align_or_default(state.text_options.align);

                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
                Self::push_text_box(output, context, opts, &state);
            }
            TagName::BlockQuote => {
                Self::push_text_box(output, context, opts, &state);
                state.to_mut().text_options.block_quote += 1;
                state.to_mut().global_indent += DEFAULT_MARGIN / 2.;

                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());

                let indent = state.global_indent;

                Self::push_text_box(output, context, opts, &state);

                if indent == DEFAULT_MARGIN / 2. {
                    Self::push_spacer(output);
                }
            }
            TagName::BoldOrStrong => {
                state.to_mut().text_options.bold = true;
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
            }
            TagName::Break => {
                Self::push_text_box(output, context, opts, &state);
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
            }
            TagName::Code => {
                state.to_mut().text_options.code = true;
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
            }
            TagName::Details => {
                //TODO
                return;
                //self.push_text_box(out, inherited_state);
                //self.push_spacer(out);
                //let section = Section::bare(self.opts.hidpi_scale);
                //*section.hidden.borrow_mut() = true;
                //todo!("Details Implementation");
                //// handle_details(...)
                //self.push_element(out, section);
                return;
            }
            TagName::Summary => {
                tracing::warn!("Summary can only be in an Details element");
                return;
            }
            TagName::Section => {
                //TODO
                return;
            }
            TagName::EmphasisOrItalic => {
                state.to_mut().text_options.italic = true;
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
            }
            TagName::Header(header) => {
                Self::push_text_box(output, context, opts, &state);
                Self::push_spacer(output);

                state.to_mut().set_align_from_attributes(&attributes);
                context.set_align_or_default(state.text_options.align);

                state.to_mut().text_options.bold = true;
                context.font_size *= header.size_multiplier();

                if header == HeaderType::H1 {
                    state.to_mut().text_options.underline = true;
                }
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());

                let anchor = context.texts.iter().flat_map(|t| t.text.chars()).collect();
                let anchor = opts.anchorizer.lock().anchorize(anchor);
                context.set_anchor(format!("#{anchor}"));
                Self::push_text_box(output, context, opts, &state);
                Self::push_spacer(output);
            }
            TagName::HorizontalRuler => {
                Self::push_element(output, Spacer::visible());
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
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
                    context.set_checkbox(Some(is_checked));
                }
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
            }
            TagName::ListItem => {
                tracing::warn!("ListItem can only be in an List element");
                return;
            }
            TagName::OrderedList => {
                OrderedListProcess::process(input, output, opts, context, node, state.clone());
            }
            TagName::UnorderedList => {
                UnorderedListProcess::process(input, output, opts, context, node, state.clone());
            }
            TagName::PreformattedText => {
                Self::push_text_box(output, context, opts, &state);
                let style = attributes
                    .iter()
                    .find_map(|attr| attr.to_style())
                    .unwrap_or_default();
                for style in style::Iter::new(&style) {
                    if let Style::BackgroundColor(color) = style {
                        let native_color = opts.native_color(color);
                        context.set_background_color(native_color);
                    }
                }
                state.to_mut().text_options.pre_formatted = true;
                context.set_code_block(true);
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());

                Self::push_text_box(output, context, opts, &state);
                Self::push_spacer(output);
            }
            TagName::Small => {
                state.to_mut().text_options.small = true;
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
            }
            TagName::Span => {
                let style_str = attributes
                    .iter()
                    .find_map(|attr| attr.to_style())
                    .unwrap_or_default();
                for style in style::Iter::new(&style_str) {
                    match style {
                        Style::Color(color) => {
                            state.to_mut().span.color = opts.native_color(color);
                        }
                        Style::FontWeight(weight) => state.to_mut().span.weight = weight,
                        Style::FontStyle(style) => state.to_mut().span.style = style,
                        Style::TextDecoration(decor) => state.to_mut().span.decor = decor,
                        _ => {}
                    }
                }
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
            }
            TagName::Strikethrough => {
                state.to_mut().text_options.strike_through = true;
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
            }
            TagName::Table => {
                TableProcess::process(input, output, opts, (), node, state.clone());
            }
            TagName::TableHead | TagName::TableBody => {
                tracing::warn!("TableHead and TableBody can only be in an Table element");
                return;
            }
            TagName::TableRow => {
                tracing::warn!("TableRow can only be in an Table element");
                return;
            }
            TagName::TableDataCell => {
                tracing::warn!(
                    "TableDataCell can only be in an TableRow or an TableHeader element"
                );
                return;
            }
            TagName::TableHeader => {
                tracing::warn!("TableDataCell can only be in an TableRow element");
                return;
            }
            TagName::Underline => {
                state.to_mut().text_options.underline = true;
                FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());
            }
            TagName::Root => tracing::error!("Root element can't reach interpreter."),
        }
    }

    fn process_content(input: Input, output: Output, opts: Opts, context: Self::Context<'_>, content: Content, state: State) {
        for node in content {
            match node {
                TextOrHirNode::Text(string) => Self::text(context, string.as_str(), opts, state.clone()),
                TextOrHirNode::Hir(node_index) => {
                    let node = Self::get_node(input, *node_index);
                    Self::process(input, output, opts, context, node, state.clone());
                }
            }
        }
    }
}

struct DetailsProcess;
impl Process for DetailsProcess {
    type Context<'a> = ();
    fn process(input: Input, output: Output, opts: Opts, context: Self::Context<'_>, node: &HirNode, state: State) {
        let mut section = Section::bare(opts.hidpi_scale);
        *section.hidden.get_mut() = true;

        let index = if let Some(first_child) = node.content.first() {
            match first_child {
                TextOrHirNode::Text(_) => 0,
                TextOrHirNode::Hir(node) => {
                    let node = Self::get_node(input, *node);
                    if node.tag == TagName::Summary {
                        section.
                        1
                    } else {
                        0
                    }
                }
            }
        };

        let mut section_content = vec![];
        let mut tb = TextBox::new(vec![], opts.hidpi_scale);
        FlowProcess::process_content(input, &mut section_content, opts, &mut tb, &node.content[index..], state);


        for elem in section_content {
            section.elements.push(Positioned::new(elem))
        }

    }
}

struct OrderedListProcess;
impl Process for OrderedListProcess {
    type Context<'a> = &'a mut TextBox;
    fn process<'a>(
        input: Input,
        output: Output,
        opts: Opts,
        context: Self::Context<'a>,
        node: &HirNode,
        mut state: State,
    ) {
        let mut index = 1;
        for attr in &node.attributes {
            if let Attr::Start(start) = attr {
                index = *start;
            }
        }
        Self::push_text_box(output, context, opts, &state);
        state.to_mut().global_indent += DEFAULT_MARGIN / 2.;

        Self::process_node(
            input,
            node,
            |_| {},
            |node| match node.tag {
                TagName::ListItem => {
                    ListItemProcess::process(
                        input,
                        output,
                        opts,
                        (context, Some(index)),
                        node,
                        state.clone(),
                    );
                    index += 1;
                }
                _ => tracing::warn!("Only ListItems can be inside an List"),
            },
        );
        if state.global_indent == DEFAULT_MARGIN / 2. {
            Self::push_spacer(output);
        }
    }
}
struct UnorderedListProcess;
impl Process for UnorderedListProcess {
    type Context<'a> = &'a mut TextBox;
    fn process<'a>(
        input: Input,
        output: Output,
        opts: Opts,
        context: Self::Context<'a>,
        node: &HirNode,
        mut state: State,
    ) {
        Self::push_text_box(output, context, opts, &state);
        state.to_mut().global_indent += DEFAULT_MARGIN / 2.;

        Self::process_node(
            input,
            node,
            |_| {},
            |node| match node.tag {
                TagName::ListItem => {
                    ListItemProcess::process(
                        input,
                        output,
                        opts,
                        (context, None),
                        node,
                        state.clone(),
                    );
                }
                _ => tracing::warn!("Only ListItems can be inside an List"),
            },
        );
        if state.global_indent == DEFAULT_MARGIN / 2. {
            Self::push_spacer(output);
        }
    }
}
struct ListItemProcess;
impl Process for ListItemProcess {
    type Context<'a> = (&'a mut TextBox, Option<usize>);
    fn process<'a>(
        input: Input,
        output: Output,
        opts: Opts,
        (context, list_prefix): Self::Context<'a>,
        node: &HirNode,
        state: State,
    ) {
        // TODO
        let anchor = node.attributes.iter().find_map(|attr| attr.to_anchor());

        let first_child_is_checkbox = if let Some(TextOrHirNode::Hir(node)) = node.content.first() {
            let node = Self::get_node(input, *node);
            if node.tag == TagName::Input {
                node.attributes
                    .iter()
                    .any(|attr| matches!(attr, Attr::IsCheckbox))
            } else {
                false
            }
        } else {
            false
        };

        if !first_child_is_checkbox {
            let prefix = match list_prefix {
                Some(num) => format!("{num}. "),
                None => String::from("· "),
            };
            context.texts.push(
                Text::new(
                    prefix,
                    opts.hidpi_scale,
                    opts.native_color(opts.theme.text_color),
                )
                .make_bold(true),
            )
        }
        FlowProcess::process_content(input, output, opts, context, &node.content, state.clone());

        Self::push_text_box(output, context, opts, &state)
    }
}

struct TableProcess;
impl Process for TableProcess {
    type Context<'a> = ();
    fn process(
        input: Input,
        output: Output,
        opts: Opts,
        context: Self::Context<'_>,
        node: &HirNode,
        state: State,
    ) {
        let mut table = Table::new();
        Self::process_node(
            input,
            node,
            |_| {},
            |node| {
                match node.tag {
                    TagName::TableHead | TagName::TableBody => {
                        TableHeadProcess::process(input, output, opts, &mut table, node, state.clone());
                    }
                    TagName::TableRow => {
                        table.rows.push(vec![]);
                        TableRowProcess::process(input, output, opts, &mut table, node, state.clone())
                    }
                    _ => tracing::warn!("Only TableHead, TableBody, TableRow and TableFoot can be inside an table, found: {:?}", node.tag),
                }
            },
        );
        Self::push_element(output, table);
        Self::push_spacer(output);
    }
}

struct TableHeadProcess;
impl Process for TableHeadProcess {
    type Context<'a> = &'a mut Table;
    fn process<'a>(
        input: Input,
        output: Output,
        opts: Opts,
        context: Self::Context<'a>,
        node: &HirNode,
        mut state: State,
    ) {
        Self::process_node(
            input,
            node,
            |_| {},
            |node| match node.tag {
                TagName::TableRow => {
                    context.rows.push(vec![]);
                    TableRowProcess::process(input, output, opts, context, node, state.clone())
                },
                _ => tracing::warn!("Only TableRows can be inside an TableHead or TableBody, found {:?}", node.tag),
            }
        );
    }
}

// https://html.spec.whatwg.org/multipage/tables.html#the-tr-element
struct TableRowProcess;
impl Process for TableRowProcess {
    type Context<'a> = &'a mut Table;
    fn process<'a>(
        input: Input,
        output: Output,
        opts: Opts,
        context: Self::Context<'a>,
        node: &HirNode,
        state: State,
    ) {
        Self::process_node(
            input,
            node,
            |_| {},
            |node| {
                let mut state = state.clone();
                state.to_mut().set_align_from_attributes(&node.attributes);
                match node.tag {
                    TagName::TableHeader => TableCellProcess::process(input, output, opts, (context, true), node, state),
                    TagName::TableDataCell => TableCellProcess::process(input, output, opts, (context, false), node, state),
                    _ => tracing::warn!("Only TableHead, TableBody, TableRow and TableFoot can be inside an table, found: {:?}", node.tag),
                }
            },
        );
    }
}

// https://html.spec.whatwg.org/multipage/tables.html#the-th-element
// https://html.spec.whatwg.org/multipage/tables.html#the-td-element
struct TableCellProcess;
impl Process for TableCellProcess {
    /// (Table, IsHeader)
    type Context<'a> = (&'a mut Table, bool);
    fn process<'a>(
        input: Input,
        output: Output,
        opts: Opts,
        (context, header): Self::Context<'a>,
        node: &HirNode,
        mut state: State,
    ) {
        let row = context
            .rows
            .last_mut()
            .expect("There should be at least one row.");
        // TODO allow anything inside tables not only text.
        if header {
            state.to_mut().text_options.bold = true;
        }
        Self::process_node(
            input,
            node,
            |text| {
                let mut tb = TextBox::new(vec![], opts.hidpi_scale);
                tb.set_align_or_default(state.text_options.align);
                Self::text(&mut tb, text, opts, state.clone());
                row.push(tb);
            },
            |_| tracing::warn!("Currently only text is allowed in an TableHeader."),
        );
    }
}
