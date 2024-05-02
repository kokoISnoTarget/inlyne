use crate::interpreter::html::{self, Attr, TagName};
use crate::utils::markdown_to_html;
use anyhow::Result;
use anyhow::{bail, Context};
use html5ever::{
    buffer_queue::BufferQueue,
    local_name,
    tendril::{fmt, Tendril},
    tokenizer::{Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts},
};
use smart_debug::SmartDebug;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::sync::Weak as ArcWeak;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    str::FromStr,
    sync::mpsc,
};
use syntect::highlighting::Theme;

#[derive(Debug, Clone)]
pub enum TextOrHirNode {
    Text(String),
    Hir(usize),
}

#[derive(SmartDebug, Clone)]
pub struct HirNode {
    pub tag: TagName,
    pub attributes: Vec<Attr>,
    pub content: Vec<TextOrHirNode>,
}
impl HirNode {
    const fn new(tag: TagName, attributes: Vec<Attr>) -> Self {
        Self {
            tag,
            attributes,
            content: vec![],
        }
    }
}

#[derive(SmartDebug, Clone)]
pub struct Hir {
    nodes: Vec<HirNode>,
    #[debug(skip)]
    parents: Vec<usize>,
    to_close: Vec<TagName>,
}
impl Hir {
    pub fn new() -> Self {
        let root = HirNode {
            tag: TagName::Root,
            attributes: vec![],
            content: vec![],
        };
        Self {
            nodes: vec![root],
            parents: vec![0],
            to_close: vec![TagName::Root],
        }
    }

    pub fn content(self) -> Vec<HirNode> {
        self.nodes
    }

    fn current_node(&mut self) -> &mut HirNode {
        self.nodes
            .get_mut(
                *self
                    .parents
                    .last()
                    .expect("There should be at least one parent"),
            )
            .expect("Any parent should be in nodes")
    }

    fn process_start_tag(&mut self, tag: Tag) {
        let tag_name = match TagName::try_from(&tag.name) {
            Ok(name) => name,
            Err(name) => {
                tracing::info!("Missing implementation for start tag: {name}");
                return;
            }
        };
        let attrs = html::attr::Iter::new(&tag.attrs).collect();

        let index = self.nodes.len();
        self.current_node().content.push(TextOrHirNode::Hir(index));

        self.nodes.push(HirNode::new(tag_name, attrs));

        if tag.self_closing || tag_name.is_void() {
            return;
        }
        self.parents.push(self.nodes.len() - 1);
        self.to_close.push(tag_name);
    }
    fn process_end_tag(&mut self, tag: Tag) -> anyhow::Result<()> {
        let tag_name = match TagName::try_from(&tag.name) {
            Ok(name) => name,
            Err(name) => {
                bail!("Missing implementation for end tag: {name}");
            }
        };
        if tag_name.is_void() {
            return Ok(());
        }

        let to_close = self.to_close.pop().context("Expected closing tag")?;

        if tag_name != to_close {
            bail!("Expected closing {to_close:?} tag but found {tag_name:?}")
        }
        self.parents.pop();
        Ok(())
    }
    fn on_text(&mut self, string: String) {
        let current_node = self.current_node();

        if string == "\n" && current_node.content.is_empty() {
            return;
        }

        current_node.content.push(TextOrHirNode::Text(string));
    }
    fn on_end(&mut self) {
        self.to_close.iter().skip(1).for_each(|unclosed_tag| {
            tracing::warn!("File contains unclosed html tag: {unclosed_tag:?}");
        })
    }
}

impl TokenSink for Hir {
    type Handle = ();

    fn process_token(&mut self, token: Token, _line_number: u64) -> TokenSinkResult<()> {
        match token {
            Token::TagToken(tag) => match tag.kind {
                TagKind::StartTag => self.process_start_tag(tag),
                TagKind::EndTag => {
                    let e = self.process_end_tag(tag);
                    if let Err(e) = e {
                        tracing::error!("{e}");
                    }
                }
            },
            Token::CharacterTokens(str) => self.on_text(str.to_string()),
            Token::EOFToken => self.on_end(),
            Token::ParseError(err) => tracing::warn!("HTML parser emitted error: {err}"),
            Token::DoctypeToken(_) | Token::CommentToken(_) | Token::NullCharacterToken => {}
        }
        TokenSinkResult::Continue
    }
}
impl Default for Hir {
    fn default() -> Self {
        Self::new()
    }
}
impl Display for Hir {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        fn fmt_inner(
            f: &mut Formatter<'_>,
            hir: &Hir,
            current: usize,
            mut indent: usize,
        ) -> std::fmt::Result {
            let node = hir.nodes.get(current).ok_or(std::fmt::Error)?;

            writeln!(f, "{:>indent$}{:?}:", "", node.tag)?;
            indent += 2;
            for ton in &node.content {
                match ton {
                    TextOrHirNode::Text(str) => writeln!(f, "{:>indent$}{str:?}", "")?,
                    TextOrHirNode::Hir(node) => fmt_inner(f, hir, *node, indent)?,
                }
            }
            Ok(())
        }
        fmt_inner(f, self, 0, 0)
    }
}
