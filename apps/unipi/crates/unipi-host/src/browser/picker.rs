//! The element picker (browser.md §9.3, WP-B8): what is under a point of the picture.
//!
//! Hover (`pick: false`) uses the DOM domain only — `DOM.getNodeForLocation`,
//! `DOM.getBoxModel`, `DOM.describeNode` — so no script runs in the page at all. A pick
//! adds one function call in an **isolated world** of the element's frame (CDP hygiene
//! H2/H3: never the main world, no `Runtime.enable`, nothing left behind) to collect the
//! ZCode web-element field set: selector, XPath, role, accessible name, visible text,
//! safe attributes and a sanitized HTML excerpt. Cookies, storage and form values are
//! never read; password fields are masked.

use serde_json::{json, Value};
use unipi_proto::{BrowserElement, BrowserElementDetail};

use super::cdp::Cdp;
use super::session::VisualViewport;

/// Name of the picker's isolated world.
const WORLD: &str = "unipi-picker";

/// Runs on the picked element (`this`) in the isolated world; returns the detail JSON.
const DESCRIBE_JS: &str = r#"function () {
  const el = this.nodeType === 1 ? this : this.parentElement;
  if (!el) return null;
  const clip = (s, n) => { s = String(s || '').replace(/\s+/g, ' ').trim(); return s.length > n ? s.slice(0, n - 1) + '…' : s; };
  const tag = el.tagName.toLowerCase();
  const secret = tag === 'input' && (el.type || '').toLowerCase() === 'password';
  const implicit = { a: el.hasAttribute('href') ? 'link' : '', button: 'button', select: 'combobox', textarea: 'textbox', img: 'img', nav: 'navigation', main: 'main', header: 'banner', footer: 'contentinfo', form: 'form', table: 'table', ul: 'list', ol: 'list', li: 'listitem', dialog: 'dialog' };
  let role = el.getAttribute('role') || implicit[tag] || '';
  if (!role && /^h[1-6]$/.test(tag)) role = 'heading';
  if (!role && tag === 'input') { const t = (el.type || 'text').toLowerCase(); role = { checkbox: 'checkbox', radio: 'radio', button: 'button', submit: 'button', range: 'slider', search: 'searchbox' }[t] || 'textbox'; }
  const byIds = (ids) => ids.split(/\s+/).map((id) => document.getElementById(id)).filter(Boolean).map((n) => n.innerText).join(' ');
  let name = el.getAttribute('aria-label') || (el.getAttribute('aria-labelledby') ? byIds(el.getAttribute('aria-labelledby')) : '') || el.getAttribute('alt') || '';
  if (!name && el.id) { const l = document.querySelector('label[for="' + CSS.escape(el.id) + '"]'); if (l) name = l.innerText; }
  if (!name) name = el.getAttribute('title') || el.getAttribute('placeholder') || '';
  const text = secret ? '••••' : clip(el.innerText || el.textContent || '', 400);
  if (!name && text.length <= 80) name = text;
  const unique = (sel) => { try { return document.querySelectorAll(sel).length === 1; } catch (e) { return false; } };
  let selector = '';
  if (el.id && unique('#' + CSS.escape(el.id))) selector = '#' + CSS.escape(el.id);
  else {
    const parts = [];
    let cur = el;
    for (let depth = 0; cur && cur.nodeType === 1 && depth < 8; depth++) {
      let part = cur.tagName.toLowerCase();
      if (cur.id && unique('#' + CSS.escape(cur.id))) { parts.unshift('#' + CSS.escape(cur.id)); break; }
      const testid = cur.getAttribute('data-testid');
      if (testid && unique('[data-testid="' + testid.replace(/"/g, '\\"') + '"]')) { parts.unshift('[data-testid="' + testid.replace(/"/g, '\\"') + '"]'); break; }
      const parent = cur.parentElement;
      if (parent) {
        const same = Array.from(parent.children).filter((c) => c.tagName === cur.tagName);
        if (same.length > 1) part += ':nth-of-type(' + (same.indexOf(cur) + 1) + ')';
      }
      parts.unshift(part);
      if (unique(parts.join(' > '))) break;
      cur = parent;
    }
    selector = parts.join(' > ');
  }
  const xp = [];
  for (let cur = el; cur && cur.nodeType === 1; cur = cur.parentElement) {
    const same = cur.parentElement ? Array.from(cur.parentElement.children).filter((c) => c.tagName === cur.tagName) : [cur];
    xp.unshift(cur.tagName.toLowerCase() + (same.length > 1 ? '[' + (same.indexOf(cur) + 1) + ']' : ''));
  }
  const safe = /^(id|class|name|type|href|src|alt|title|role|placeholder|for|action|method|target|rel|lang|data-testid|aria-[a-z-]+)$/;
  const attributes = {};
  for (const a of Array.from(el.attributes)) if (safe.test(a.name)) attributes[a.name] = clip(a.value, 200);
  const copy = el.cloneNode(true);
  for (const n of copy.querySelectorAll ? copy.querySelectorAll('script,style,noscript,svg') : []) n.remove();
  const scrub = (n) => { if (n.nodeType !== 1) return; for (const a of Array.from(n.attributes)) if (/^on/i.test(a.name) || a.name === 'value' || a.name === 'style' || a.name === 'srcdoc') n.removeAttribute(a.name); for (const c of Array.from(n.children)) scrub(c); };
  scrub(copy);
  let html = copy.outerHTML || '';
  if (html.length > 800) html = html.slice(0, 799) + '…';
  return { url: location.href, title: document.title, tag, role: role || null, name: clip(name, 160) || null, selector: selector || null, xpath: '/' + xp.join('/'), text: text || null, attributes, html: html || null };
}"#;

/// `tag#id.class.class` (≤ 2 classes, ≤ 60 chars) from `DOM.describeNode` attributes.
pub fn label_of(node_name: &str, attributes: &[Value]) -> String {
    let mut label = node_name.to_ascii_lowercase();
    let pairs = attributes.chunks(2).filter_map(|kv| match kv {
        [k, v] => Some((k.as_str()?, v.as_str()?)),
        _ => None,
    });
    let mut classes = String::new();
    for (key, value) in pairs {
        match key {
            "id" if !value.is_empty() => {
                label.push('#');
                label.push_str(value);
            }
            "class" => {
                for class in value.split_whitespace().take(2) {
                    classes.push('.');
                    classes.push_str(class);
                }
            }
            _ => {}
        }
    }
    label.push_str(&classes);
    if label.chars().count() > 60 {
        label = label.chars().take(59).collect::<String>() + "…";
    }
    label
}

/// The border quad's bounding box, mapped from layout-viewport px to the picture's px.
pub fn rect_of(quad: &[Value], visual: &VisualViewport) -> Option<(f64, f64, f64, f64)> {
    let nums: Vec<f64> = quad.iter().filter_map(Value::as_f64).collect();
    if nums.len() != 8 {
        return None;
    }
    let xs = [nums[0], nums[2], nums[4], nums[6]];
    let ys = [nums[1], nums[3], nums[5], nums[7]];
    let min = |v: &[f64; 4]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = |v: &[f64; 4]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (x0, y0, x1, y1) = (min(&xs), min(&ys), max(&xs), max(&ys));
    let to = |v: f64, offset: f64| (v - offset) * visual.scale;
    Some((
        to(x0, visual.offset_x),
        to(y0, visual.offset_y),
        (x1 - x0) * visual.scale,
        (y1 - y0) * visual.scale,
    ))
}

/// What is at picture point (x, y) of the tab behind `session` (`None`: nothing, or the
/// page refused). `pick` adds the details.
pub async fn inspect(
    cdp: &Cdp,
    session: &str,
    visual: &VisualViewport,
    x: f64,
    y: f64,
    pick: bool,
) -> Option<BrowserElement> {
    let s = Some(session);
    let (lx, ly) = visual.to_layout(x, y);
    let hit = cdp
        .call(
            "DOM.getNodeForLocation",
            json!({
                "x": lx.round() as i64,
                "y": ly.round() as i64,
                "includeUserAgentShadowDOM": false,
                "ignorePointerEventsNone": true,
            }),
            s,
        )
        .await
        .ok()?;
    let backend = hit.get("backendNodeId")?.as_i64()?;
    let frame = hit
        .get("frameId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let node = cdp
        .call("DOM.describeNode", json!({ "backendNodeId": backend }), s)
        .await
        .ok()?;
    let node = node.get("node")?;
    let name = node.get("nodeName").and_then(Value::as_str).unwrap_or("");
    let attributes = node
        .get("attributes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let model = cdp
        .call("DOM.getBoxModel", json!({ "backendNodeId": backend }), s)
        .await
        .ok()?;
    let quad = model.get("model")?.get("border")?.as_array()?;
    let (ex, ey, width, height) = rect_of(quad, visual)?;
    let detail = if pick {
        describe(cdp, session, backend, frame.as_deref()).await
    } else {
        None
    };
    Some(BrowserElement {
        x: ex,
        y: ey,
        width,
        height,
        label: label_of(name, &attributes),
        detail,
    })
}

/// The pick's details, computed in an isolated world of the element's frame.
async fn describe(
    cdp: &Cdp,
    session: &str,
    backend: i64,
    frame: Option<&str>,
) -> Option<BrowserElementDetail> {
    let s = Some(session);
    let frame = frame?;
    let world = cdp
        .call(
            "Page.createIsolatedWorld",
            json!({ "frameId": frame, "worldName": WORLD, "grantUniveralAccess": false }),
            s,
        )
        .await
        .ok()?;
    let context = world.get("executionContextId")?.as_i64()?;
    let object = cdp
        .call(
            "DOM.resolveNode",
            json!({ "backendNodeId": backend, "executionContextId": context }),
            s,
        )
        .await
        .ok()?;
    let object_id = object.get("object")?.get("objectId")?.as_str()?.to_owned();
    let reply = cdp
        .call(
            "Runtime.callFunctionOn",
            json!({
                "objectId": object_id,
                "functionDeclaration": DESCRIBE_JS,
                "returnByValue": true,
                "silent": true,
            }),
            s,
        )
        .await;
    let _ = cdp
        .call("Runtime.releaseObject", json!({ "objectId": object_id }), s)
        .await;
    let value = reply.ok()?.get("result")?.get("value")?.clone();
    serde_json::from_value(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_short_css_like() {
        let attrs = [
            json!("id"),
            json!("send"),
            json!("class"),
            json!("btn primary big"),
            json!("type"),
            json!("submit"),
        ];
        assert_eq!(label_of("BUTTON", &attrs), "button#send.btn.primary");
        assert_eq!(label_of("H1", &[]), "h1");
        let long = [json!("id"), json!("x".repeat(100))];
        assert_eq!(label_of("DIV", &long).chars().count(), 60);
    }

    #[test]
    fn rects_follow_the_visual_viewport() {
        let quad: Vec<Value> = [8.0, 8.0, 772.0, 8.0, 772.0, 45.0, 8.0, 45.0]
            .iter()
            .map(|n| json!(n))
            .collect();
        let plain = VisualViewport::default();
        assert_eq!(rect_of(&quad, &plain), Some((8.0, 8.0, 764.0, 37.0)));
        let zoomed = VisualViewport {
            offset_x: 4.0,
            offset_y: 0.0,
            scale: 2.0,
        };
        assert_eq!(rect_of(&quad, &zoomed), Some((8.0, 16.0, 1528.0, 74.0)));
        assert_eq!(rect_of(&quad[..4], &plain), None);
    }

    #[test]
    fn details_parse_from_the_page_reply() {
        let detail: BrowserElementDetail = serde_json::from_value(json!({
            "url": "http://x/", "title": "T", "tag": "h1", "role": "heading", "name": "Hi",
            "selector": "#hd", "xpath": "/html/body/h1", "text": "Hi", "attributes": {"id": "hd"},
            "html": "<h1 id=\"hd\">Hi</h1>"
        }))
        .unwrap();
        assert_eq!(detail.selector.as_deref(), Some("#hd"));
        assert_eq!(detail.attributes["id"], "hd");
        let sparse: BrowserElementDetail = serde_json::from_value(json!({
            "url": "u", "title": "", "tag": "div", "role": null, "name": null, "selector": null,
            "xpath": "/html", "text": null, "attributes": {}, "html": null
        }))
        .unwrap();
        assert!(sparse.role.is_none());
    }
}
