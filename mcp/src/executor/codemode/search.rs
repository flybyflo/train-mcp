use rquickjs::{prelude::MutFn, Ctx, Function, Object, Result as JsResult, Value as JsValue};
use serde_json::Value;

use super::json_to_js;

pub(super) fn install_search_tools<'js>(
    ctx: &Ctx<'js>,
    codemode: &Object<'js>,
    catalog: Value,
) -> JsResult<()> {
    let catalog_for_get = catalog.clone();
    let get_catalog_fn = Function::new(
        ctx.clone(),
        MutFn::new(move |ctx: Ctx<'js>| -> JsResult<JsValue<'js>> {
            json_to_js(&ctx, &catalog_for_get)
        }),
    )?;
    codemode.set("getCatalog", get_catalog_fn)?;

    let catalog_for_list = catalog;
    let list_tools_fn = Function::new(
        ctx.clone(),
        MutFn::new(move |ctx: Ctx<'js>| -> JsResult<JsValue<'js>> {
            let tools = catalog_for_list
                .get("tools")
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            let tool_names: Vec<Value> = tools
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.get("name"),
                        "description": t.get("description"),
                        "provider": t.get("provider"),
                    })
                })
                .collect();
            json_to_js(&ctx, &Value::Array(tool_names))
        }),
    )?;
    codemode.set("listTools", list_tools_fn)?;

    Ok(())
}
