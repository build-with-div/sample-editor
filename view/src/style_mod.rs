//! A port of [`style-mod`](https://github.com/marijnh/style-mod) 4.1.4
//! (MIT), which CodeMirror 6 uses for its themes: a style module renders
//! a nested style spec to CSS rules, and mounting the modules puts their
//! rules in a document's stylesheet.
//!
//! Rendering the rules is pure and ported as is. Mounting needs a runtime
//! `<style>` element (div-gap: 14b) and selector matching (div-gap: 14a),
//! which Div lacks; [`StyleSet`] keeps the mounted modules and the text
//! their `<style>` element would hold, and [`InlineRules`] is the labelled
//! scaffolding that applies their rules as inline styles in the meantime.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use div::typed_style::{parse_literal, StyleProperty};
use div::Element;

/// A value in a [`StyleSpec`]: a property value, or a nested spec for a
/// nested selector or an at-rule.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleSpecValue {
    /// A property value, as JavaScript would print it.
    Value(String),
    /// A nested spec.
    Nested(StyleSpec),
}

/// A nested style spec (style-mod's `StyleSpec`): properties, in
/// camelCase as in JavaScript, and nested selectors, in the order they
/// were added, as a JavaScript object keeps its keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleSpec(pub Vec<(String, StyleSpecValue)>);

impl StyleSpec {
    /// An empty spec.
    pub fn new() -> StyleSpec {
        StyleSpec::default()
    }

    /// Add a property, such as `("fontSize", "13px")`.
    pub fn set(mut self, property: &str, value: impl Into<String>) -> StyleSpec {
        self.0
            .push((property.into(), StyleSpecValue::Value(value.into())));
        self
    }

    /// Add a property with a number value, printed as JavaScript prints
    /// numbers (`lineHeight: 1.4`, `zIndex: 0`).
    pub fn num(self, property: &str, value: f64) -> StyleSpec {
        self.set(property, js_number(value))
    }

    /// Add a nested selector or at-rule.
    pub fn nest(mut self, selector: &str, spec: StyleSpec) -> StyleSpec {
        self.0.push((selector.into(), StyleSpecValue::Nested(spec)));
        self
    }
}

// JavaScript's `String(number)` for the values themes use.
fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// A set of CSS rules (`StyleModule`).
#[derive(Debug, PartialEq)]
pub struct StyleModule {
    rules: Vec<String>,
}

const C: char = '\u{37c}';
// `top[COUNT]`: one counter per JavaScript realm, here per process.
static COUNT: AtomicUsize = AtomicUsize::new(1);

impl StyleModule {
    /// Render `spec` (`new StyleModule(spec, {finish})`). `finish`, when
    /// given, computes the final selector of each regular (non-`@`)
    /// selector. Fails, as upstream throws, when a nested spec is given
    /// for a property outside an at-rule.
    pub fn new(
        spec: &StyleSpec,
        finish: Option<&dyn Fn(&str) -> String>,
    ) -> Result<StyleModule, String> {
        let mut rules = Vec::new();
        for (prop, value) in &spec.0 {
            render(&split_selector(prop), value, &mut rules, false, finish)?;
        }
        Ok(StyleModule { rules })
    }

    /// The module's CSS rules, one per line (`getRules`).
    pub fn get_rules(&self) -> String {
        self.rules.join("\n")
    }

    /// The module's rules.
    pub fn rules(&self) -> &[String] {
        &self.rules
    }

    /// A new unique CSS class name (`newName`): `ͼ1`, `ͼ2` and so on.
    pub fn new_name() -> String {
        let id = COUNT.fetch_add(1, Ordering::Relaxed);
        format!("{C}{}", to_base36(id))
    }
}

fn to_base36(mut n: usize) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[n % 36]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

// `/^@/.test(selector) ? [selector] : selector.split(/,\s*/)`.
fn split_selector(selector: &str) -> Vec<String> {
    if selector.starts_with('@') {
        vec![selector.to_string()]
    } else {
        split_commas(selector)
    }
}

// `split(/,\s*/)`.
fn split_commas(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(',') {
        parts.push(rest[..i].to_string());
        rest = rest[i + 1..].trim_start_matches(|c: char| c.is_whitespace());
    }
    parts.push(rest.to_string());
    parts
}

// The name of an at-rule at the start of `selector` (`/^@(\w+)\b/`).
fn at_rule(selector: &str) -> Option<&str> {
    let rest = selector.strip_prefix('@')?;
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

// `prop.replace(/_.*/, "").replace(/[A-Z]/g, l => "-" + l.toLowerCase())`.
fn css_name(prop: &str) -> String {
    let prop = prop.split('_').next().unwrap_or("");
    let mut out = String::new();
    for ch in prop.chars() {
        if ch.is_ascii_uppercase() {
            out.push('-');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn render(
    selectors: &[String],
    value: &StyleSpecValue,
    target: &mut Vec<String>,
    is_keyframes: bool,
    finish: Option<&dyn Fn(&str) -> String>,
) -> Result<(), String> {
    let spec = match value {
        StyleSpecValue::Nested(spec) => spec,
        // A primitive at the top level is ignored, as `for (prop in
        // "string")` iterates characters that are never rendered.
        StyleSpecValue::Value(_) => return Ok(()),
    };
    let mut local = Vec::new();
    let at = at_rule(&selectors[0]);
    let is_at = at.is_some();
    let keyframes = at == Some("keyframes");
    for (prop, value) in &spec.0 {
        if prop.contains('&') {
            let expanded: Vec<String> = split_commas(prop)
                .iter()
                .flat_map(|part| selectors.iter().map(move |sel| part.replacen('&', sel, 1)))
                .collect();
            render(&expanded, value, target, false, finish)?;
        } else if let StyleSpecValue::Nested(_) = value {
            if !is_at {
                return Err(format!(
                    "The value of a property ({prop}) should be a primitive value."
                ));
            }
            render(&split_selector(prop), value, &mut local, keyframes, finish)?;
        } else if let StyleSpecValue::Value(value) = value {
            local.push(format!("{}: {value};", css_name(prop)));
        }
    }
    if !local.is_empty() || keyframes {
        let selectors: Vec<String> = match finish {
            Some(finish) if !is_at && !is_keyframes => {
                selectors.iter().map(|sel| finish(sel)).collect()
            }
            _ => selectors.to_vec(),
        };
        target.push(format!("{} {{{}}}", selectors.join(", "), local.join(" ")));
    }
    Ok(())
}

/// The modules mounted in a document (`StyleSet`), in the order their
/// rules take effect.
///
/// div-gap: 14b runtime `<style>` element text. Upstream keeps one set per
/// document or shadow root and writes the rules to a `<style>` element;
/// Div has no runtime stylesheets, so each editor view keeps its own set,
/// and [`text`](StyleSet::text) is what the element would hold.
#[derive(Default)]
pub struct StyleSet {
    modules: Vec<Arc<StyleModule>>,
}

impl StyleSet {
    /// Mount `modules` (`StyleModule.mount`): add those not mounted yet,
    /// and move those mounted out of order, so that later modules take
    /// precedence. Returns whether the set changed.
    pub fn mount(&mut self, modules: &[Arc<StyleModule>]) -> bool {
        let mut j = 0;
        let mut changed = false;
        for module in modules {
            let mut index = self.modules.iter().position(|m| Arc::ptr_eq(m, module));
            if let Some(i) = index {
                if i < j {
                    self.modules.remove(i);
                    changed = true;
                    j -= 1;
                    index = None;
                }
            }
            match index {
                None => {
                    self.modules.insert(j, module.clone());
                    j += 1;
                    changed = true;
                }
                Some(i) => j = i + 1,
            }
        }
        changed
    }

    /// The text of the `<style>` element.
    pub fn text(&self) -> String {
        let mut text = String::new();
        for module in &self.modules {
            text.push_str(&module.get_rules());
            text.push('\n');
        }
        text
    }

    /// The mounted modules, in order.
    pub fn modules(&self) -> &[Arc<StyleModule>] {
        &self.modules
    }
}

/// The mounted rules, applied as inline styles.
///
/// div-gap: 14a stylesheets and selectors. Until Div matches selectors,
/// this applies the rules whose selectors are classes joined by the
/// descendant combinator (`.ͼ1 .cm-scroller`) to the elements the view
/// asks for, in the order the rules are mounted, ignoring specificity.
/// Rules with other combinators, pseudo-classes or attribute selectors,
/// and declarations Div cannot parse, are skipped and counted.
pub struct InlineRules {
    rules: Vec<InlineRule>,
    /// The number of rules skipped for their selectors.
    pub skipped_rules: usize,
    /// The number of declarations skipped because Div cannot parse them.
    pub skipped_declarations: usize,
    properties: Vec<StyleProperty>,
}

struct InlineRule {
    // Alternatives, each a list of compounds, each a list of classes.
    selectors: Vec<Vec<Vec<String>>>,
    declarations: Vec<(StyleProperty, div::StyleValue)>,
}

impl InlineRules {
    /// Read the rules of `set`.
    pub fn new(set: &StyleSet) -> InlineRules {
        let mut out = InlineRules {
            rules: Vec::new(),
            skipped_rules: 0,
            skipped_declarations: 0,
            properties: Vec::new(),
        };
        for module in set.modules() {
            for rule in module.rules() {
                out.read_rule(rule);
            }
        }
        out
    }

    // A rule as `render` writes it: `selectors {name: value; ...}`.
    fn read_rule(&mut self, rule: &str) {
        let (Some(open), true) = (rule.find(" {"), rule.ends_with('}')) else {
            self.skipped_rules += 1;
            return;
        };
        let mut selectors = Vec::new();
        for selector in split_commas(&rule[..open]) {
            match class_compounds(&selector) {
                Some(compounds) => selectors.push(compounds),
                None => {
                    self.skipped_rules += 1;
                    return;
                }
            }
        }
        let mut declarations = Vec::new();
        for declaration in rule[open + 2..rule.len() - 1].split(';') {
            let declaration = declaration.trim();
            if declaration.is_empty() {
                continue;
            }
            match parse_declaration(declaration) {
                Some(parsed) => {
                    if !self.properties.contains(&parsed.0) {
                        self.properties.push(parsed.0);
                    }
                    declarations.push(parsed);
                }
                None => self.skipped_declarations += 1,
            }
        }
        self.rules.push(InlineRule {
            selectors,
            declarations,
        });
    }

    /// Give `element` the inline styles of the rules that match it, after
    /// removing those any rule could have given it before.
    pub fn apply(&self, element: &Element) -> Result<(), div::Error> {
        for &property in &self.properties {
            element.remove_property(property)?;
        }
        for rule in &self.rules {
            if rule
                .selectors
                .iter()
                .any(|compounds| matches(element, compounds).unwrap_or(false))
            {
                for (property, value) in &rule.declarations {
                    element.set_property(*property, value.clone())?;
                }
            }
        }
        // The element's `style` attribute comes after the rules in the
        // cascade, so its declarations are applied again over theirs.
        if let Some(style) = element.get_attribute("style")? {
            for (property, value) in style.split(';').filter_map(parse_declaration) {
                element.set_property(property, value)?;
            }
        }
        Ok(())
    }
}

// The compounds of a selector made of classes and descendant combinators,
// or `None` for any other selector.
fn class_compounds(selector: &str) -> Option<Vec<Vec<String>>> {
    let mut compounds = Vec::new();
    for compound in selector.split_whitespace() {
        let rest = compound.strip_prefix('.')?;
        let classes: Vec<String> = rest.split('.').map(str::to_string).collect();
        if classes.iter().any(|class| {
            class.is_empty()
                || !class
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == C)
        }) {
            return None;
        }
        compounds.push(classes);
    }
    (!compounds.is_empty()).then_some(compounds)
}

/// Parse `name: value` with Div's value parser, dropping `!important`.
pub(crate) fn parse_declaration(declaration: &str) -> Option<(StyleProperty, div::StyleValue)> {
    let (name, value) = declaration.split_once(':')?;
    let property = StyleProperty::from_css_name(&name.trim().to_ascii_lowercase())?;
    let value = value.trim();
    let value = value.strip_suffix("!important").unwrap_or(value).trim();
    parse_literal(property, value)
        .ok()
        .map(|value| (property, value))
}

fn has_classes(element: &Element, classes: &[String]) -> Result<bool, div::Error> {
    let Some(list) = element.get_attribute("class")? else {
        return Ok(false);
    };
    Ok(classes
        .iter()
        .all(|class| list.split_whitespace().any(|c| c == class)))
}

fn matches(element: &Element, compounds: &[Vec<String>]) -> Result<bool, div::Error> {
    let (last, ancestors) = compounds.split_last().expect("a compound");
    if !has_classes(element, last)? {
        return Ok(false);
    }
    let mut current = element.parent()?;
    for compound in ancestors.iter().rev() {
        loop {
            let Some(node) = current else {
                return Ok(false);
            };
            current = node.parent()?;
            if has_classes(&node, compound)? {
                break;
            }
        }
    }
    Ok(true)
}
