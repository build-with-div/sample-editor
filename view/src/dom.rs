//! The DOM operations the view needs, on Div's DOM: the parts of
//! `attributes.ts` that write to elements (`setAttrs`), the child-list
//! updates of `CompositeTile.sync` in `tile.ts`, and the geometry and
//! scrolling helpers of `dom.ts`.
//!
//! Div's element API lacks some of the DOM these need (section 2 of the
//! design document, gaps 22–25); the functions marked `div-gap` are
//! scaffolds that end in the same DOM as upstream with what Div offers,
//! except where their comments say otherwise.

use div::typed_style::{Overflow, Position, StyleProperty};
use div::{Document, Element, ElementTag, TextNode, WeakTextNode};

use crate::attributes::Attrs;
use crate::extension::ScrollStrategy;
use crate::style_mod::parse_declaration;

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
    let serialized;
    let value = if name == "style" {
        serialized = css_text(value);
        serialized.as_str()
    } else {
        value
    };
    let old = dom.get_attribute(name)?;
    if old.as_deref() != Some(value) {
        dom.set_attribute(name, value)?;
        if name == "style" {
            apply_style_attribute(dom, old.as_deref(), Some(value))?;
        }
    }
    Ok(())
}

/// The `style` attribute after `dom.style.cssText = text`: each declaration
/// as `name: value;`, separated by spaces.
///
/// div-gap: 22 `CSSStyleDeclaration.cssText`. A browser serializes the
/// parsed declarations, normalizing values (colors become `rgb()`, for
/// one); this keeps each value as written, which is the same for the
/// lengths and numbers the view writes.
pub(crate) fn css_text(text: &str) -> String {
    text.split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            let (name, value) = (name.trim(), value.trim());
            (!name.is_empty() && !value.is_empty()).then(|| format!("{name}: {value};"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Remove the attribute `name`, and for `style`, the inline style it gave.
pub(crate) fn remove_attribute(dom: &Element, name: &str) -> Result<(), div::Error> {
    if name == "style" {
        let old = dom.get_attribute(name)?;
        apply_style_attribute(dom, old.as_deref(), None)?;
    }
    dom.remove_attribute(name)?;
    Ok(())
}

/// Apply a `style` attribute's declarations as the element's inline style,
/// removing those of its previous value that it no longer has.
///
/// div-gap: 22 the `style` attribute and `CSSStyleDeclaration.cssText`.
/// Div stores a `style` attribute without applying it. This splits the
/// declaration list at semicolons and colons, as the simple values the
/// view writes allow (`height: 18px`, `tab-size: 4`), and parses each value
/// with Div's own value parser; a declaration Div cannot parse is skipped.
fn apply_style_attribute(
    dom: &Element,
    old: Option<&str>,
    new: Option<&str>,
) -> Result<(), div::Error> {
    let parse = |text: Option<&str>| -> Vec<_> {
        text.map(|text| text.split(';').filter_map(parse_declaration).collect())
            .unwrap_or_default()
    };
    let new = parse(new);
    for (property, _) in parse(old) {
        if !new.iter().any(|(p, _)| *p == property) {
            dom.remove_property(property)?;
        }
    }
    for (property, value) in new {
        dom.set_property(property, value)?;
    }
    Ok(())
}

/// Set one inline style property, `dom.style[name] = value`; an empty
/// value removes it. As in a browser, the `style` attribute reflects the
/// inline declarations: a new property is appended, a changed one keeps
/// its place, and removing the last one leaves `style=""`.
///
/// div-gap: 22 `CSSStyleDeclaration`. The value is parsed with Div's value
/// parser; a value it cannot parse is ignored, as a browser ignores an
/// invalid one. Values are serialized as written (see [`css_text`]).
pub(crate) fn set_style(dom: &Element, name: &str, value: &str) -> Result<(), div::Error> {
    if StyleProperty::from_css_name(name).is_none() {
        return Ok(());
    }
    if !value.is_empty() && parse_declaration(&format!("{name}: {value}")).is_none() {
        return Ok(());
    }
    let old = dom.get_attribute("style")?;
    let mut declarations: Vec<(String, String)> = old
        .as_deref()
        .unwrap_or("")
        .split(';')
        .filter_map(|declaration| {
            let (n, v) = declaration.split_once(':')?;
            Some((n.trim().to_string(), v.trim().to_string()))
        })
        .filter(|(n, v)| !n.is_empty() && !v.is_empty())
        .collect();
    let index = declarations.iter().position(|(n, _)| n == name);
    match (index, value.is_empty()) {
        // Removing a property the element does not have changes nothing.
        (None, true) => return Ok(()),
        (Some(i), true) => {
            declarations.remove(i);
        }
        (Some(i), false) => declarations[i].1 = value.to_string(),
        (None, false) => declarations.push((name.to_string(), value.to_string())),
    }
    let text = declarations
        .iter()
        .map(|(n, v)| format!("{n}: {v};"))
        .collect::<Vec<_>>()
        .join(" ");
    set_attribute(dom, "style", &text)
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
            remove_attribute(dom, name)?;
        }
    }
    for (name, value) in attrs.iter() {
        set_attribute(dom, name, value)?;
    }
    Ok(())
}

/// Move `dom` from the attributes `prev` to `attrs`, writing only those
/// that changed (`updateAttrs`). Returns whether any did.
pub(crate) fn update_attrs(
    dom: &Element,
    prev: Option<&Attrs>,
    attrs: Option<&Attrs>,
) -> Result<bool, div::Error> {
    let mut changed = false;
    if let Some(prev) = prev {
        for (name, _) in prev.iter() {
            if !attrs.is_some_and(|attrs| attrs.contains(name)) {
                changed = true;
                remove_attribute(dom, name)?;
            }
        }
    }
    if let Some(attrs) = attrs {
        for (name, value) in attrs.iter() {
            if prev.is_none_or(|prev| prev.get(name) != Some(value)) {
                changed = true;
                set_attribute(dom, name, value)?;
            }
        }
    }
    Ok(changed)
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

/// A rectangle in viewport coordinates (`Rect`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    /// The left edge.
    pub left: f64,
    /// The right edge.
    pub right: f64,
    /// The top edge.
    pub top: f64,
    /// The bottom edge.
    pub bottom: f64,
}

impl Rect {
    pub(crate) const EMPTY: Rect = Rect {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 0.0,
    };

    /// The width.
    pub fn width(&self) -> f64 {
        self.right - self.left
    }

    /// The height.
    pub fn height(&self) -> f64 {
        self.bottom - self.top
    }
}

/// `getBoundingClientRect()`. An element without a box has an empty
/// rectangle at the origin.
pub(crate) fn bounding_rect(dom: &Element) -> Result<Rect, div::Error> {
    Ok(match dom.bounding_rect()? {
        Some(r) => Rect {
            left: r.x as f64,
            right: (r.x + r.width) as f64,
            top: r.y as f64,
            bottom: (r.y + r.height) as f64,
        },
        None => Rect::EMPTY,
    })
}

/// The window's size, `innerWidth` and `innerHeight`.
///
/// div-gap: 26 `window.innerWidth`/`innerHeight`. Div's document has a
/// viewport size it does not report, so the editor view takes it from its
/// configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowSize {
    /// `innerWidth`.
    pub width: f64,
    /// `innerHeight`.
    pub height: f64,
}

/// The scale of the CSS transforms on `elt`, whose bounding rectangle is
/// `rect` (`getScale`).
///
/// div-gap: 27 `HTMLElement.offsetWidth`/`offsetHeight`. The scale is the
/// transformed size over the layout size, which Div does not report. This
/// returns 1, which is right for an editor with no transformed ancestor.
pub(crate) fn get_scale(_elt: &Element, _rect: &Rect) -> (f64, f64) {
    (1.0, 1.0)
}

fn is_scroll_overflow(overflow: Overflow) -> bool {
    matches!(overflow, Overflow::Auto | Overflow::Scroll)
}

/// The nearest vertically scrollable ancestor of `dom` below the body
/// (`scrollableParents(dom, false).y`).
pub(crate) fn scrollable_parent_y(
    dom: &Element,
    body: &Element,
) -> Result<Option<Element>, div::Error> {
    let mut cur = dom.parent()?;
    while let Some(node) = cur {
        if node.is_same_node(body)? {
            break;
        }
        if node.scroll_height()? > node.client_height()?
            && is_scroll_overflow(*node.computed_style()?.overflow_y())
        {
            return Ok(Some(node));
        }
        cur = node.parent()?;
    }
    Ok(None)
}

/// Whether `elt` is scrolled to the bottom (`isScrolledToBottom`). `None`
/// stands for the window.
pub(crate) fn is_scrolled_to_bottom(elt: Option<&Element>) -> Result<bool, div::Error> {
    Ok(match elt {
        Some(elt) => {
            (elt.scroll_height()? as f64 - elt.client_height()? as f64 - elt.scroll_top()?).abs()
                < 2.0
        }
        // div-gap: 26. Div does not report the window's scroll position;
        // the sample's window does not scroll.
        None => true,
    })
}

/// Scroll the ancestors of `dom` so that `rect` is in view
/// (`scrollRectIntoView`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn scroll_rect_into_view(
    dom: &Element,
    body: &Element,
    window: WindowSize,
    mut rect: Rect,
    side: i32,
    mut x: ScrollStrategy,
    mut y: ScrollStrategy,
    x_margin: f64,
    y_margin: f64,
    ltr: bool,
) -> Result<(), div::Error> {
    let mut cur = Some(dom.clone());
    while let Some(node) = cur {
        let top = node.is_same_node(body)?;
        let (mut scale_x, mut scale_y) = (1.0, 1.0);
        let (mut x_scroll, mut y_scroll) = (true, true);
        let mut stop = false;
        let bounding = if top {
            Rect {
                left: 0.0,
                right: window.width,
                top: 0.0,
                bottom: window.height,
            }
        } else {
            let style = node.computed_style()?;
            if matches!(*style.position(), Position::Fixed | Position::Sticky) {
                stop = true;
            }
            if node.scroll_height()? <= node.client_height()?
                || !is_scroll_overflow(*style.overflow_y())
            {
                y_scroll = false;
            }
            if node.scroll_width()? <= node.client_width()?
                || !is_scroll_overflow(*style.overflow_x())
            {
                x_scroll = false;
            }
            if !x_scroll && !y_scroll {
                if stop {
                    break;
                }
                cur = node.parent()?;
                continue;
            }
            let r = bounding_rect(&node)?;
            (scale_x, scale_y) = get_scale(&node, &r);
            Rect {
                left: r.left,
                right: r.left + node.client_width()? as f64 * scale_x,
                top: r.top,
                bottom: r.top + node.client_height()? as f64 * scale_y,
            }
        };

        let mut move_x = 0.0;
        let mut move_y = 0.0;
        if !y_scroll {
            // Skip
        } else if y == ScrollStrategy::Nearest {
            if rect.top < bounding.top + y_margin {
                move_y = rect.top - (bounding.top + y_margin);
                if side > 0 && rect.bottom > bounding.bottom + move_y {
                    move_y = rect.bottom - bounding.bottom + y_margin;
                }
            } else if rect.bottom > bounding.bottom - y_margin {
                move_y = rect.bottom - bounding.bottom + y_margin;
                if side < 0 && (rect.top - move_y) < bounding.top {
                    move_y = rect.top - (bounding.top + y_margin);
                }
            }
        } else {
            let rect_height = rect.bottom - rect.top;
            let bounding_height = bounding.bottom - bounding.top;
            let target_top = if y == ScrollStrategy::Center && rect_height <= bounding_height {
                rect.top + rect_height / 2.0 - bounding_height / 2.0
            } else if y == ScrollStrategy::Start || y == ScrollStrategy::Center && side < 0 {
                rect.top - y_margin
            } else {
                rect.bottom - bounding_height + y_margin
            };
            move_y = target_top - bounding.top;
        }
        if !x_scroll {
            // Skip
        } else if x == ScrollStrategy::Nearest {
            if rect.left < bounding.left + x_margin {
                move_x = rect.left - (bounding.left + x_margin);
                if side > 0 && rect.right > bounding.right + move_x {
                    move_x = rect.right - bounding.right + x_margin;
                }
            } else if rect.right > bounding.right - x_margin {
                move_x = rect.right - bounding.right + x_margin;
                if side < 0 && rect.left < bounding.left + move_x {
                    move_x = rect.left - (bounding.left + x_margin);
                }
            }
        } else {
            let target_left = if x == ScrollStrategy::Center {
                rect.left + (rect.right - rect.left) / 2.0 - (bounding.right - bounding.left) / 2.0
            } else if (x == ScrollStrategy::Start) == ltr {
                rect.left - x_margin
            } else {
                rect.right - (bounding.right - bounding.left) + x_margin
            };
            move_x = target_left - bounding.left;
        }
        if move_x != 0.0 || move_y != 0.0 {
            if top {
                // `win.scrollBy` (div-gap: 26): Div's window does not
                // scroll, so a target outside the window stays there.
            } else {
                let start_left = node.scroll_left()?;
                let start_top = node.scroll_top()?;
                let left = start_left + if move_x != 0.0 { move_x / scale_x } else { 0.0 };
                let scroll_top = start_top + if move_y != 0.0 { move_y / scale_y } else { 0.0 };
                node.scroll_to(left, scroll_top)?;
                let moved_y = if move_y != 0.0 {
                    (node.scroll_top()? - start_top) * scale_y
                } else {
                    0.0
                };
                let moved_x = if move_x != 0.0 {
                    (node.scroll_left()? - start_left) * scale_x
                } else {
                    0.0
                };
                rect = Rect {
                    left: rect.left - moved_x,
                    top: rect.top - moved_y,
                    right: rect.right - moved_x,
                    bottom: rect.bottom - moved_y,
                };
                if moved_x != 0.0 && (moved_x - move_x).abs() < 1.0 {
                    x = ScrollStrategy::Nearest;
                }
                if moved_y != 0.0 && (moved_y - move_y).abs() < 1.0 {
                    y = ScrollStrategy::Nearest;
                }
            }
        }
        if top || stop {
            break;
        }
        if rect.top < bounding.top
            || rect.bottom > bounding.bottom
            || rect.left < bounding.left
            || rect.right > bounding.right
        {
            rect = Rect {
                left: rect.left.max(bounding.left),
                right: rect.right.min(bounding.right),
                top: rect.top.max(bounding.top),
                bottom: rect.bottom.min(bounding.bottom),
            };
        }
        cur = node.parent()?;
    }
    Ok(())
}
