//! A port of `theme.ts`: the theme facets, the class names of the base
//! theme, and the base theme's rules.

use std::sync::{Arc, LazyLock};

use editor_state::{Facet, FacetConfig};

use crate::style_mod::{StyleModule, StyleSpec};

/// A mounted style module, compared by identity as upstream compares
/// facet values.
#[derive(Clone, Debug)]
pub struct ModuleRef(pub Arc<StyleModule>);

impl PartialEq for ModuleRef {
    fn eq(&self, other: &ModuleRef) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The class names of the active themes, joined by spaces (`theme`).
pub(crate) static THEME: LazyLock<Facet<String, String>> =
    LazyLock::new(|| Facet::define_with(FacetConfig::new(|values: &[String]| values.join(" "))));

/// Whether a dark theme is active (`darkTheme`).
pub(crate) static DARK_THEME: LazyLock<Facet<bool, bool>> = LazyLock::new(|| {
    Facet::define_with(FacetConfig::new(|values: &[bool]| values.contains(&true)))
});

/// The style modules to mount (`styleModule`).
pub(crate) static STYLE_MODULE: LazyLock<Facet<ModuleRef>> = LazyLock::new(Facet::define);

/// The class names upstream creates when its modules load, in the order
/// the bundled package creates them.
pub(crate) struct ModuleNames {
    /// `baseThemeID`.
    pub base: String,
    /// `baseLightID`.
    pub light: String,
    /// `baseDarkID`.
    pub dark: String,
}

/// The names upstream's modules take when they load: the base theme's
/// three, then the one `draw-selection.ts` takes for `hideNativeSelection`
/// (`EditorView.theme` at load time). Taking them first keeps the names of
/// themes created later equal to upstream's (the first is `ͼ5`).
pub(crate) static NAMES: LazyLock<ModuleNames> = LazyLock::new(|| {
    let names = ModuleNames {
        base: StyleModule::new_name(),
        light: StyleModule::new_name(),
        dark: StyleModule::new_name(),
    };
    // `hideNativeSelection`'s theme, ported with `draw-selection.ts`.
    let _hide_native_selection = StyleModule::new_name();
    names
});

/// Build a theme module for the editor element matched by `main`
/// (`buildTheme`). `&` in a selector stands for the editor element;
/// `&light` and `&dark`, when `scopes` gives their selectors, for an
/// editor with a light or dark theme. Other selectors are scoped under
/// the editor.
pub(crate) fn build_theme(
    main: &str,
    spec: &StyleSpec,
    scopes: Option<(&str, &str)>,
) -> Result<StyleModule, String> {
    let error = std::cell::RefCell::new(None);
    let finish = |sel: &str| -> String {
        let Some(at) = sel.find('&') else {
            return format!("{main} {sel}");
        };
        let rest = &sel[at + 1..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let m = &sel[at..at + 1 + end];
        let replacement = match (m, scopes) {
            ("&", _) => main.to_string(),
            ("&light", Some((light, _))) => light.to_string(),
            ("&dark", Some((_, dark))) => dark.to_string(),
            _ => {
                *error.borrow_mut() = Some(format!("Unsupported selector: {m}"));
                m.to_string()
            }
        };
        format!("{}{replacement}{}", &sel[..at], &sel[at + m.len()..])
    };
    let module = StyleModule::new(spec, Some(&finish))?;
    match error.into_inner() {
        Some(error) => Err(error),
        None => Ok(module),
    }
}

/// The base theme module (`baseTheme`).
pub(crate) static BASE_THEME: LazyLock<Arc<StyleModule>> = LazyLock::new(|| {
    let names = &*NAMES;
    let light = format!(".{}", names.light);
    let dark = format!(".{}", names.dark);
    Arc::new(
        build_theme(
            &format!(".{}", names.base),
            &base_theme_spec(),
            Some((&light, &dark)),
        )
        .expect("the base theme"),
    )
});

fn rule() -> StyleSpec {
    StyleSpec::new()
}

#[rustfmt::skip]
fn base_theme_spec() -> StyleSpec {
    StyleSpec::new()
        .nest("&", rule()
            .set("position", "relative !important")
            .set("boxSizing", "border-box")
            .nest("&.cm-focused", rule().set("outline", "1px dotted #212121"))
            .set("display", "flex !important")
            .set("flexDirection", "column"))
        .nest(".cm-scroller", rule()
            .set("display", "flex !important")
            .set("alignItems", "flex-start !important")
            .set("fontFamily", "monospace")
            .num("lineHeight", 1.4)
            .set("height", "100%")
            .set("overflowX", "auto")
            .set("position", "relative")
            .num("zIndex", 0.0)
            .set("overflowAnchor", "none"))
        .nest(".cm-content", rule()
            .num("margin", 0.0)
            .num("flexGrow", 2.0)
            .num("flexShrink", 0.0)
            .set("display", "block")
            .set("whiteSpace", "pre")
            .set("wordWrap", "normal")
            .set("boxSizing", "border-box")
            .set("minHeight", "100%")
            .set("padding", "4px 0")
            .set("outline", "none")
            .nest("&[contenteditable=true]", rule()
                .set("WebkitUserModify", "read-write-plaintext-only")))
        .nest(".cm-lineWrapping", rule()
            .set("whiteSpace_fallback", "pre-wrap")
            .set("whiteSpace", "break-spaces")
            .set("wordBreak", "break-word")
            .set("overflowWrap", "anywhere")
            .num("flexShrink", 1.0))
        .nest("&light .cm-content", rule().set("caretColor", "black"))
        .nest("&dark .cm-content", rule().set("caretColor", "white"))
        .nest(".cm-line", rule()
            .set("display", "block")
            .set("padding", "0 2px 0 6px"))
        .nest(".cm-layer", rule()
            .set("userSelect", "none")
            .set("position", "absolute")
            .num("left", 0.0)
            .num("top", 0.0)
            .set("contain", "size style")
            .nest("& > *", rule().set("position", "absolute")))
        .nest("&light .cm-selectionBackground", rule().set("background", "#d9d9d9"))
        .nest("&dark .cm-selectionBackground", rule().set("background", "#222"))
        .nest("&light.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground",
            rule().set("background", "#d7d4f0"))
        .nest("&dark.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground",
            rule().set("background", "#233"))
        .nest(".cm-cursorLayer", rule().set("pointerEvents", "none"))
        .nest("&.cm-focused > .cm-scroller > .cm-cursorLayer",
            rule().set("animation", "steps(1) cm-blink 1.2s infinite"))
        .nest("@keyframes cm-blink", rule()
            .nest("0%", rule())
            .nest("50%", rule().num("opacity", 0.0))
            .nest("100%", rule()))
        .nest("@keyframes cm-blink2", rule()
            .nest("0%", rule())
            .nest("50%", rule().num("opacity", 0.0))
            .nest("100%", rule()))
        .nest(".cm-cursor, .cm-dropCursor", rule()
            .set("borderLeft", "1.2px solid black")
            .set("marginLeft", "-0.6px")
            .set("pointerEvents", "none"))
        .nest(".cm-cursor", rule().set("display", "none"))
        .nest("&dark .cm-cursor", rule().set("borderLeftColor", "#ddd"))
        .nest(".cm-selectionHandle", rule()
            .set("backgroundColor", "currentColor")
            .set("width", "1.5px"))
        .nest(".cm-selectionHandle-start::before, .cm-selectionHandle-end::before", rule()
            .set("content", "\"\"")
            .set("backgroundColor", "inherit")
            .set("borderRadius", "50%")
            .set("width", "8px")
            .set("height", "8px")
            .set("position", "absolute")
            .set("left", "-3.25px"))
        .nest(".cm-selectionHandle-start::before", rule().set("top", "-8px"))
        .nest(".cm-selectionHandle-end::before", rule().set("bottom", "-8px"))
        .nest(".cm-dropCursor", rule().set("position", "absolute"))
        .nest("&.cm-focused > .cm-scroller > .cm-cursorLayer .cm-cursor",
            rule().set("display", "block"))
        .nest(".cm-iso", rule().set("unicodeBidi", "isolate"))
        .nest(".cm-announced", rule()
            .set("position", "fixed")
            .set("top", "-10000px"))
        .nest("@media print", rule()
            .nest(".cm-announced", rule().set("display", "none")))
        .nest("&light .cm-activeLine", rule().set("backgroundColor", "#cceeff44"))
        .nest("&dark .cm-activeLine", rule().set("backgroundColor", "#99eeff33"))
        .nest("&light .cm-specialChar", rule().set("color", "red"))
        .nest("&dark .cm-specialChar", rule().set("color", "#f78"))
        .nest(".cm-gutters", rule()
            .num("flexShrink", 0.0)
            .set("display", "flex")
            .set("height", "100%")
            .set("boxSizing", "border-box")
            .num("zIndex", 200.0))
        .nest(".cm-gutters-before", rule().num("insetInlineStart", 0.0))
        .nest(".cm-gutters-after", rule().num("insetInlineEnd", 0.0))
        .nest("&light .cm-gutters", rule()
            .set("backgroundColor", "#f5f5f5")
            .set("color", "#6c6c6c")
            .set("border", "0px solid #ddd")
            .nest("&.cm-gutters-before", rule().set("borderRightWidth", "1px"))
            .nest("&.cm-gutters-after", rule().set("borderLeftWidth", "1px")))
        .nest("&dark .cm-gutters", rule()
            .set("backgroundColor", "#333338")
            .set("color", "#ccc"))
        .nest(".cm-gutter", rule()
            .set("display", "flex !important")
            .set("flexDirection", "column")
            .num("flexShrink", 0.0)
            .set("boxSizing", "border-box")
            .set("minHeight", "100%")
            .set("overflow", "hidden"))
        .nest(".cm-gutterElement", rule().set("boxSizing", "border-box"))
        .nest(".cm-lineNumbers .cm-gutterElement", rule()
            .set("padding", "0 3px 0 5px")
            .set("minWidth", "20px")
            .set("textAlign", "right")
            .set("whiteSpace", "nowrap"))
        .nest("&light .cm-activeLineGutter", rule().set("backgroundColor", "#e2f2ff"))
        .nest("&dark .cm-activeLineGutter", rule().set("backgroundColor", "#222227"))
        .nest(".cm-panels", rule()
            .set("boxSizing", "border-box")
            .set("position", "sticky")
            .num("left", 0.0)
            .num("right", 0.0)
            .num("zIndex", 300.0))
        .nest("&light .cm-panels", rule()
            .set("backgroundColor", "#f5f5f5")
            .set("color", "black"))
        .nest(".cm-panels-top", rule().set("top", "0"))
        .nest(".cm-panels-bottom", rule().set("bottom", "0"))
        .nest("&light .cm-panels-top", rule().set("borderBottom", "1px solid #ddd"))
        .nest("&light .cm-panels-bottom", rule().set("borderTop", "1px solid #ddd"))
        .nest("&dark .cm-panels", rule()
            .set("backgroundColor", "#333338")
            .set("color", "white"))
        .nest(".cm-dialog", rule()
            .set("padding", "2px 19px 4px 6px")
            .set("position", "relative")
            .nest("& label", rule().set("fontSize", "80%")))
        .nest(".cm-dialog-close", rule()
            .set("position", "absolute")
            .set("top", "3px")
            .set("right", "4px")
            .set("backgroundColor", "inherit")
            .set("border", "none")
            .set("font", "inherit")
            .set("fontSize", "14px")
            .set("padding", "0"))
        .nest(".cm-tab", rule()
            .set("display", "inline-block")
            .set("overflow", "hidden")
            .set("verticalAlign", "bottom"))
        .nest(".cm-widgetBuffer", rule()
            .set("verticalAlign", "baseline")
            .set("height", "1em")
            .num("width", 0.0)
            .set("display", "inline"))
        .nest(".cm-placeholder", rule()
            .set("color", "#888")
            .set("display", "inline-block")
            .set("verticalAlign", "top")
            .set("userSelect", "none"))
        .nest(".cm-highlightSpace", rule()
            .set("background", "radial-gradient(circle at 50% 55%, #aaa 20%, transparent 0) no-repeat")
            .set("backgroundSize", ".4em")
            .set("backgroundPosition", "calc(min(50%, 0px)) center"))
        .nest(".cm-highlightTab", rule()
            .set("backgroundImage", "url('data:image/svg+xml,<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"20\"><path stroke=\"%23888\" stroke-width=\"1\" fill=\"none\" d=\"M1 10H196L190 5M190 15L196 10M197 4L197 16\"/></svg>')")
            .set("backgroundSize", "auto 100%")
            .set("backgroundPosition", "right 90%")
            .set("backgroundRepeat", "no-repeat"))
        .nest(".cm-trailingSpace", rule().set("backgroundColor", "#ff332255"))
        .nest(".cm-button", rule()
            .set("verticalAlign", "middle")
            .set("color", "inherit")
            .set("fontSize", "70%")
            .set("padding", ".2em 1em")
            .set("borderRadius", "1px"))
        .nest("&light .cm-button", rule()
            .set("backgroundImage", "linear-gradient(#eff1f5, #d9d9df)")
            .set("border", "1px solid #888")
            .nest("&:active", rule().set("backgroundImage", "linear-gradient(#b4b4b4, #d0d3d6)")))
        .nest("&dark .cm-button", rule()
            .set("backgroundImage", "linear-gradient(#393939, #111)")
            .set("border", "1px solid #888")
            .nest("&:active", rule().set("backgroundImage", "linear-gradient(#111, #333)")))
        .nest(".cm-textfield", rule()
            .set("verticalAlign", "middle")
            .set("color", "inherit")
            .set("fontSize", "70%")
            .set("border", "1px solid silver")
            .set("padding", ".2em .5em"))
        .nest("&light .cm-textfield", rule().set("backgroundColor", "white"))
        .nest("&dark .cm-textfield", rule()
            .set("border", "1px solid #555")
            .set("backgroundColor", "inherit"))
}
