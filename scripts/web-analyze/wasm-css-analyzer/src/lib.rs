use tools::css_analyzer;
use js_sys;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn analyze_css_source(text: String, first_line: u32, callback: &js_sys::Function) {
    let mut callback = |s| {
        let this = JsValue::null();
        let arg = JsValue::from(s);
        let _ = callback.call1(&this, &arg);
    };
    css_analyzer::analyze_css("".to_string(), first_line, text, &mut callback);
}
