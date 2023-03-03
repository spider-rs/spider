//! HTML nodes.

use hashbrown::{hash_map::Iter, hash_set, HashMap, HashSet};

use fast_html5ever::{Attribute, LocalName, QualName};
use selectors::attr::CaseSensitivity;
use std::fmt;
use std::ops::Deref;

use fast_html5ever::tendril::{fmt::UTF8, Atomic, Tendril};

/// Atomic StrTendril type
pub type AtomicStrTendril = Tendril<UTF8, Atomic>;

/// An HTML node.
// `Element` is usally the most common variant and hence boxing it
// will most likely not improve performance overall.
#[allow(variant_size_differences)]
#[derive(Clone)]
pub enum Node {
    /// The document root.
    Document,

    /// The fragment root.
    Fragment,

    /// A doctype.
    Doctype(Doctype),

    /// A comment.
    Comment(Comment),

    /// Text.
    Text(Text),

    /// An element.
    Element(Element),

    /// A processing instruction.
    ProcessingInstruction(ProcessingInstruction),
}

impl Node {
    /// Returns true if node is the document root.
    pub fn is_document(&self) -> bool {
        matches!(*self, Node::Document)
    }

    /// Returns true if node is the fragment root.
    pub fn is_fragment(&self) -> bool {
        matches!(*self, Node::Fragment)
    }

    /// Returns true if node is a doctype.
    pub fn is_doctype(&self) -> bool {
        matches!(*self, Node::Doctype(_))
    }

    /// Returns true if node is a comment.
    pub fn is_comment(&self) -> bool {
        matches!(*self, Node::Comment(_))
    }

    /// Returns true if node is text.
    pub fn is_text(&self) -> bool {
        matches!(*self, Node::Text(_))
    }

    /// Returns true if node is an element.
    pub fn is_element(&self) -> bool {
        matches!(*self, Node::Element(_))
    }

    /// Returns self as a doctype.
    pub fn as_doctype(&self) -> Option<&Doctype> {
        match *self {
            Node::Doctype(ref d) => Some(d),
            _ => None,
        }
    }

    /// Returns self as a comment.
    pub fn as_comment(&self) -> Option<&Comment> {
        match *self {
            Node::Comment(ref c) => Some(c),
            _ => None,
        }
    }

    /// Returns self as text.
    pub fn as_text(&self) -> Option<&Text> {
        match *self {
            Node::Text(ref t) => Some(t),
            _ => None,
        }
    }

    /// Returns self as an element.
    pub fn as_element(&self) -> Option<&Element> {
        match *self {
            Node::Element(ref e) => Some(e),
            _ => None,
        }
    }

    /// Returns self as an element.
    pub fn as_processing_instruction(&self) -> Option<&ProcessingInstruction> {
        match *self {
            Node::ProcessingInstruction(ref pi) => Some(pi),
            _ => None,
        }
    }
}

// Always use one line.
impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match *self {
            Node::Document => write!(f, "Document"),
            Node::Fragment => write!(f, "Fragment"),
            Node::Doctype(ref d) => write!(f, "Doctype({:?})", d),
            Node::Comment(ref c) => write!(f, "Comment({:?})", c),
            Node::Text(ref t) => write!(f, "Text({:?})", t),
            Node::Element(ref e) => write!(f, "Element({:?})", e),
            Node::ProcessingInstruction(ref pi) => write!(f, "ProcessingInstruction({:?})", pi),
        }
    }
}

/// A doctype.
#[derive(Clone)]
pub struct Doctype {
    /// The doctype name.
    pub name: AtomicStrTendril,

    /// The doctype public ID.
    pub public_id: AtomicStrTendril,

    /// The doctype system ID.
    pub system_id: AtomicStrTendril,
}

impl Doctype {
    /// Returns the doctype name.
    pub fn name(&self) -> &str {
        self.name.deref()
    }

    /// Returns the doctype public ID.
    pub fn public_id(&self) -> &str {
        self.public_id.deref()
    }

    /// Returns the doctype system ID.
    pub fn system_id(&self) -> &str {
        self.system_id.deref()
    }
}

impl fmt::Debug for Doctype {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "<!DOCTYPE {} PUBLIC {:?} {:?}>",
            self.name(),
            self.public_id(),
            self.system_id()
        )
    }
}

/// An HTML comment.
#[derive(Clone)]
pub struct Comment {
    /// The comment text.
    pub comment: AtomicStrTendril,
}

impl Deref for Comment {
    type Target = str;

    fn deref(&self) -> &str {
        self.comment.deref()
    }
}

impl fmt::Debug for Comment {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "<!-- {:?} -->", self.deref())
    }
}

/// HTML text.
#[derive(Clone, PartialEq, Eq)]
pub struct Text {
    /// The text.
    pub text: AtomicStrTendril,
}

impl Deref for Text {
    type Target = str;

    fn deref(&self) -> &str {
        self.text.deref()
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{:?}", self.deref())
    }
}

/// A Map of attributes that doesn't preserve the order of the attributes.
/// Please enable the `deterministic` feature for order-preserving
/// (de)serialization.
pub type Attributes = HashMap<QualName, AtomicStrTendril>;

/// An HTML element.
#[derive(Clone, PartialEq, Eq)]
pub struct Element {
    /// The element name.
    pub name: QualName,

    /// The element ID.
    pub id: Option<LocalName>,

    /// The element classes.
    pub classes: HashSet<LocalName>,

    /// The element attributes.
    pub attrs: Attributes,
}

impl Element {
    #[doc(hidden)]
    pub fn new(name: QualName, attributes: Vec<Attribute>) -> Self {
        let mut classes: HashSet<LocalName> = HashSet::new();
        let mut attrs: HashMap<QualName, AtomicStrTendril> =
            HashMap::with_capacity(attributes.len());
        let mut id: Option<LocalName> = None;

        for a in attributes {
            match a.name.local.deref() {
                "id" => {
                    id = Some(LocalName::from(a.value.deref()));
                }
                "class" => {
                    classes.extend(a.value.deref().split_whitespace().map(LocalName::from));
                }
                _ => (),
            };
            attrs.insert(a.name, a.value.into_send().into());
        }

        Element {
            attrs,
            name,
            id,
            classes,
        }
    }

    /// Returns the element name.
    pub fn name(&self) -> &str {
        self.name.local.deref()
    }

    /// Returns the element ID.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Returns true if element has the class.
    pub fn has_class(&self, class: &str, case_sensitive: CaseSensitivity) -> bool {
        self.classes()
            .any(|c| case_sensitive.eq(c.as_bytes(), class.as_bytes()))
    }

    /// Returns an iterator over the element's classes.
    pub fn classes(&self) -> Classes {
        Classes {
            inner: self.classes.iter(),
        }
    }

    /// Returns the value of an attribute.
    pub fn attr(&self, attr: &str) -> Option<&str> {
        let qualname = QualName::new(None, ns!(), LocalName::from(attr));
        self.attrs.get(&qualname).map(Deref::deref)
    }

    /// Returns an iterator over the element's attributes.
    pub fn attrs(&self) -> Attrs {
        Attrs {
            inner: self.attrs.iter(),
        }
    }
}

/// Iterator over classes.
#[allow(missing_debug_implementations)]
#[derive(Clone)]
pub struct Classes<'a> {
    inner: hash_set::Iter<'a, LocalName>,
}

impl<'a> Iterator for Classes<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        self.inner.next().map(Deref::deref)
    }
}

/// An iterator over a node's attributes.
pub type AttributesIter<'a> = Iter<'a, QualName, AtomicStrTendril>;

/// Iterator over attributes.
#[allow(missing_debug_implementations)]
#[derive(Clone)]
pub struct Attrs<'a> {
    inner: AttributesIter<'a>,
}

impl<'a> Iterator for Attrs<'a> {
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<(&'a str, &'a str)> {
        self.inner.next().map(|(k, v)| (k.local.deref(), v.deref()))
    }
}

impl fmt::Debug for Element {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "<{}", self.name())?;
        for (key, value) in self.attrs() {
            write!(f, " {}={:?}", key, value)?;
        }
        write!(f, ">")
    }
}

/// HTML Processing Instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessingInstruction {
    /// The PI target.
    pub target: AtomicStrTendril,
    /// The PI data.
    pub data: AtomicStrTendril,
}

impl Deref for ProcessingInstruction {
    type Target = str;

    fn deref(&self) -> &str {
        self.data.deref()
    }
}

pub(crate) mod serializable;
