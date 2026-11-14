//! Port of `attributes.ts`: DOM attribute sets for decorations.
//!
//! `setAttrs`, `updateAttrs` and `getAttrs` write to and read from DOM
//! elements; they come with the view's DOM.

/// A set of DOM attributes (`Attrs`), in the order they were added, as a
/// JavaScript object keeps its string keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attrs(Vec<(String, String)>);

impl Attrs {
    /// The empty set (`noAttrs`).
    pub fn new() -> Attrs {
        Attrs(Vec::new())
    }

    /// The value of `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Set `name` to `value`, keeping its place if it was already set.
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let (name, value) = (name.into(), value.into());
        match self.0.iter_mut().find(|(key, _)| *key == name) {
            Some(entry) => entry.1 = value,
            None => self.0.push((name, value)),
        }
    }

    /// Whether `name` is set.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The attributes in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// The number of attributes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no attributes.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Attrs {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut attrs = Attrs::new();
        for (name, value) in iter {
            attrs.set(name, value);
        }
        attrs
    }
}

/// Add `source`'s attributes to `target` and return it (`combineAttrs`).
/// A class is appended to `target`'s with a space and a style with a
/// semicolon; any other attribute replaces `target`'s.
pub fn combine_attrs(source: &Attrs, mut target: Attrs) -> Attrs {
    for (name, value) in source.iter() {
        let joined = match (name, target.get(name)) {
            ("class", Some(class)) if !class.is_empty() => format!("{class} {value}"),
            ("style", Some(style)) if !style.is_empty() => format!("{style};{value}"),
            _ => value.to_string(),
        };
        target.set(name, joined);
    }
    target
}

/// Whether two attribute sets hold the same attributes, in any order,
/// leaving out `ignore` (`attrsEq`). `None` is the empty set.
pub fn attrs_eq(a: Option<&Attrs>, b: Option<&Attrs>, ignore: Option<&str>) -> bool {
    let empty = Attrs::new();
    let (a, b) = (a.unwrap_or(&empty), b.unwrap_or(&empty));
    if std::ptr::eq(a, b) {
        return true;
    }
    let ignored = |attrs: &Attrs| usize::from(ignore.is_some_and(|name| attrs.contains(name)));
    if a.len() - ignored(a) != b.len() - ignored(b) {
        return false;
    }
    let same = a
        .iter()
        .all(|(key, value)| Some(key) == ignore || b.get(key) == Some(value));
    same
}
