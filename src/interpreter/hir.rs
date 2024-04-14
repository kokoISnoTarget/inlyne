use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::str::FromStr;
use std::sync::mpsc;
use anyhow::{bail, Context};
use html5ever::buffer_queue::BufferQueue;
use html5ever::tendril::{fmt, Tendril};
use html5ever::tokenizer::{Tag, TagKind, Token, Tokenizer, TokenizerOpts, TokenSink, TokenSinkResult};
use parking_lot::Mutex;
use smart_debug::SmartDebug;
use syntect::highlighting::Theme;
use crate::interpreter::{html, State};
use crate::interpreter::html::{Attr, TagName};
use crate::text::TextBox;
use crate::utils::markdown_to_html;

type RcNode = Rc<RefCell<HirNode>>;
type WeakNode = Weak<RefCell<HirNode>>;

#[derive(Debug, Clone)]
enum TextOrHirNode {
    Text(String),
    Hir(RcNode)
}

#[derive(SmartDebug, Clone)]
struct HirNode {
    #[debug(skip)]
    parent: WeakNode,
    tag: TagName,
    attributes: Vec<Attr>,
    content: Vec<TextOrHirNode>
}

#[derive(SmartDebug, Clone)]
struct Hir {
    content: RcNode,
    #[debug(skip)]
    current: RcNode,
    to_close: Vec<TagName>,
}
impl Hir {
    pub fn new() -> Self {
        let root = Rc::new(RefCell::new(HirNode {
            parent: Default::default(),
            tag: TagName::Root,
            attributes: vec![],
            content: vec![],
        }));
        Self {
            content: Rc::clone(&root),
            current: root,
            to_close: vec![TagName::Root],
        }
    }

    pub fn transpile_md(self, receiver: mpsc::Receiver<String>, sender: mpsc::Sender<Hir>) {
        let mut input = BufferQueue::default();

        let mut tok = Tokenizer::new(self, TokenizerOpts::default());

        for md_string in receiver {
            tracing::debug!(
                "Received markdown for interpretation: {} bytes",
                md_string.len()
            );

            let html = markdown_to_html(&md_string, Theme::default());

            input.push_back(
                Tendril::from_str(&html)
                    .unwrap()
                    .try_reinterpret::<fmt::UTF8>()
                    .unwrap(),
            );

            let _ = tok.feed(&mut input);
            assert!(input.is_empty());
            tok.end();

            sender.send(tok.sink.clone()).unwrap();
        }
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

        let node = Rc::new(RefCell::new(HirNode {
            parent: Rc::downgrade(&self.current),
            tag: tag_name.clone(),
            attributes: attrs,
            content: vec![],
        }));

        self.current.borrow_mut().content.push(TextOrHirNode::Hir(
            Rc::clone(&node)
        ));

        if tag.self_closing { return; }

        self.current = node;
        self.to_close.push(tag_name);
    }
    fn process_end_tag(&mut self, tag: Tag) -> anyhow::Result<()> {
        let tag_name = match TagName::try_from(&tag.name) {
            Ok(name) => name,
            Err(name) => {
                bail!("Missing implementation for start tag: {name}");
            }
        };

        let to_close = self.to_close.pop().context("Expected closing tag")?;
        if tag_name != to_close {
            bail!("Expected {to_close:?} but found {tag_name:?}")
        }
        let parent = {
            self.current.borrow().parent.upgrade().context("Node has no parent")?
        };
        self.current = parent;

        Ok(())
    }
    fn process_text(&mut self, string: String) {
        self.current.borrow_mut().content.push(TextOrHirNode::Text(string))
    }
    fn on_end(&mut self) {
        for unclosed_tag in &self.to_close {
            tracing::warn!("File contains unclosed html tag: {unclosed_tag:?}");
        }
    }
}

impl TokenSink for Hir {
    type Handle = ();

    fn process_token(&mut self, token: Token, _line_number: u64) -> TokenSinkResult<()> {
        match token {
            Token::TagToken(tag) => match tag.kind {
                TagKind::StartTag => self.process_start_tag(tag),
                TagKind::EndTag => {
                    let _ = self.process_end_tag(tag);
                },
            },
            Token::CharacterTokens(str) => self.process_text(str.to_string()),
            Token::EOFToken => self.on_end(),
            Token::ParseError(err) => tracing::warn!("HTML parser emitted error: {err}"),
            Token::DoctypeToken(_) | Token::CommentToken(_) | Token::NullCharacterToken => {}
        }
        TokenSinkResult::Continue
    }
}
unsafe impl Send for Hir {}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test() {
        let (send, recv) = mpsc::channel();
        let (hsend, hrecv) = mpsc::channel();

        let join = std::thread::spawn(|| {
            let hir = Hir::new();
            hir.transpile_md(recv, hsend);
        });

        send.send(String::from(r#"<p>In a paragraph <a href="https://example.org">https://example.org</a></p>"#)).unwrap();
        let hir = hrecv.recv().unwrap();
        std::fs::write("output.hir", &format!("{:#?}", hir.content)).expect("TODO: panic message");
    }
}