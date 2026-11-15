//! The DOM operations the tile tree needs, on Div's DOM: the parts of
//! `attributes.ts` that write to elements (`setAttrs`) and the child-list
//! updates of `CompositeTile.sync` in `tile.ts`.
//!
//! Div's element API lacks some of the DOM these need (section 2 of the
//! design document, gaps 22–25); the functions marked `div-gap` are
//! scaffolds that end in the same DOM as upstream with what Div offers,
//! except where their comments say otherwise.

use div::{Document, Element, ElementTag, TextNode, WeakTextNode};

use crate::attributes::Attrs;

const STALE: div::Error = div::Error::InvalidArgument("the text node was removed");

/// A DOM node a tile draws into: an element or a text node.
#[derive(Clone)]
pub(crate) enum DomNode {
    Element(Element),
    Text(WeakTextNode),
}

impl DomNode {
    pub(crate) fn element(&self) -> Option<&Element> {
        match self {
            DomNode::Element(element) => Some(element),
            DomNode::Text(_) => None,
        }
    }

    pub(crate) fn text_node(&self) -> Result<TextNode, div::Error> {
        match self {
            DomNode::Text(text) => text.upgrade().ok_or(STALE),
            DomNode::Element(_) => Err(div::Error::InvalidArgument("not a text node")),
        }
    }

    /// Remove the node, and its subtree, from the document.
    pub(crate) fn remove(&self) -> Result<(), div::Error> {
        match self {
            DomNode::Element(element) => element.remove(),
            DomNode::Text(text) => match text.upgrade() {
                Some(node) => node.remove(),
                None => Ok(()),
            },
        }
    }
}

/// `document.createElement(tag)`.
///
/// div-gap: 25 `document.createElement` for any tag name, and
/// `Element.nodeName`. Div builds a fixed set of tags and reports a kind
/// that several tags share, so a mark or block wrapper with another tag
/// name fails here.
pub(crate) fn create_element(document: &Document, tag: &str) -> Result<Element, div::Error> {
    Element::create(document, tag)
}

/// `document.createTextNode(text)`.
pub(crate) fn create_text(document: &Document, text: &str) -> Result<WeakTextNode, div::Error> {
    // Div's handle does not remove the node when dropped; the tile that
    // owns the node removes it when it is destroyed.
    Ok(document.create_text_node(text)?.downgrade())
}

/// Whether `element` is a `<br>` (`nodeName == "BR"`).
pub(crate) fn is_br(element: &Element) -> Result<bool, div::Error> {
    Ok(element.kind()? == ElementTag::Break)
}

/// Set `name` to `value`, as an attribute or, for `style`, as the
/// element's inline style (`dom.style.cssText = value`).
pub(crate) fn set_attribute(dom: &Element, name: &str, value: &str) -> Result<(), div::Error> {
    // div-gap: 22 the `style` attribute and `CSSStyleDeclaration.cssText`.
    // Div stores a `style` attribute without applying it; the declarations
    // take effect once Div parses inline styles.
    if dom.get_attribute(name)?.as_deref() != Some(value) {
        dom.set_attribute(name, value)?;
    }
    Ok(())
}

/// Give `dom` exactly the attributes `attrs` (`setAttrs`). `written` holds
/// the attributes this function last gave the element.
///
/// div-gap: 23 `Element.attributes`. Upstream removes every attribute not
/// in `attrs`; without attribute enumeration, this removes those it wrote
/// before, which leaves attributes that other code set on the element.
pub(crate) fn set_attrs(dom: &Element, attrs: &Attrs, written: &Attrs) -> Result<(), div::Error> {
    for (name, _) in written.iter() {
        if !attrs.contains(name) {
            dom.remove_attribute(name)?;
        }
    }
    for (name, value) in attrs.iter() {
        set_attribute(dom, name, value)?;
    }
    Ok(())
}

/// Make `children` the child nodes of `parent`, in order, keeping nodes
/// that are already in place (the DOM part of `CompositeTile.sync`).
/// Nodes in `parent` that are not in `children` are detached.
///
/// div-gap: 24 `Node.firstChild`/`nextSibling` over text nodes and
/// `Node.insertBefore` with a text node as reference. Div's element API
/// walks and inserts before elements only. When every child is an element
/// this is upstream's algorithm; when there are text nodes, it appends the
/// children in order, which ends in the same child list after the stray
/// nodes are removed (the tile update removes them with their tiles) but
/// moves every child.
pub(crate) fn sync_children(parent: &Element, children: &[DomNode]) -> Result<(), div::Error> {
    if children.iter().all(|child| child.element().is_some()) {
        sync_element_children(parent, children.iter().filter_map(DomNode::element))
    } else {
        for child in children {
            match child {
                DomNode::Element(element) => parent.append_child(element)?,
                DomNode::Text(_) => parent.append_text_child(&child.text_node()?)?,
            }
        }
        Ok(())
    }
}

fn sync_element_children<'a>(
    parent: &Element,
    children: impl Iterator<Item = &'a Element>,
) -> Result<(), div::Error> {
    let mut prev: Option<Element> = None;
    for child in children {
        let mut next = match &prev {
            Some(prev) => prev.next_sibling()?,
            None => parent.first_child()?,
        };
        let in_parent = match child.parent()? {
            Some(current) => current.is_same_node(parent)?,
            None => false,
        };
        if in_parent {
            while let Some(node) = next {
                if node.is_same_node(child)? {
                    break;
                }
                next = rm(&node)?;
            }
        } else {
            match &next {
                Some(next) => parent.insert_before(child, next)?,
                None => parent.append_child(child)?,
            }
        }
        prev = Some(child.clone());
    }
    let mut next = match &prev {
        Some(prev) => prev.next_sibling()?,
        None => parent.first_child()?,
    };
    while let Some(node) = next {
        next = rm(&node)?;
    }
    Ok(())
}

// Detach a node and return its next sibling. The node is detached rather
// than removed, as another tile may still place it.
fn rm(dom: &Element) -> Result<Option<Element>, div::Error> {
    let next = dom.next_sibling()?;
    dom.detach()?;
    Ok(next)
}
