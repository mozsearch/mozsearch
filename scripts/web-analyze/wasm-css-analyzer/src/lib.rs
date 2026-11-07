use tools::css_analyzer;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
/// A CSS source's analysis (see `css_analyzer::analyze_css`), its custom
/// properties in `namespace`, unless it's empty.
pub fn analyze_css_source(
    text: String,
    first_line: u32,
    namespace: String,
    callback: &js_sys::Function,
) {
    let mut callback = |s| {
        let this = JsValue::null();
        let arg = JsValue::from(s);
        let _ = callback.call1(&this, &arg);
    };
    let namespace = (!namespace.is_empty()).then_some(namespace.as_str());
    css_analyzer::analyze_css("".to_string(), first_line, text, namespace, &mut callback);
}
