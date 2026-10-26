//! Guest-side SDK for the `provider-extension` world of
//! `oino:extension@0.1.0`. Sits next to `oino-extension-guest` (separate crate
//! because two `wit_bindgen::generate!` invocations in one crate collide on the
//! generated cabi macro names).

#![allow(unsafe_code)]

pub mod prov {
    //! Bindings for the `provider-extension` world.
    wit_bindgen::generate!({
        world: "provider-extension",
        path: "../../../wit",
        generate_all,
        export_macro_name: "export_provider_extension",
        pub_export_macro: true,
    });
}

use oino_extension_guest::ext;
pub use oino_extension_guest::{json_in, json_out, Extension};
use serde_json::Value;

/// Pull-based event stream produced by [`Provider::stream`].
pub struct ResponseStream(
    pub  std::sync::Mutex<
        Box<
            dyn Iterator<Item = Result<Vec<prov::oino::extension::types::AssistantEvent>, String>>
                + Send,
        >,
    >,
);

/// Provider-side trait; a provider extension exports both `guest` and
/// `provider` interfaces, so the same type also implements [`Extension`].
pub trait Provider {
    fn list_models() -> Result<Value, String>;
    fn stream(request: Value) -> Result<ResponseStream, String>;
}

fn prov_tool_declaration(
    tool: &ext::oino::extension::types::ToolDeclaration,
) -> prov::oino::extension::types::ToolDeclaration {
    prov::oino::extension::types::ToolDeclaration {
        name: tool.name.clone(),
        description: tool.description.clone(),
        input_schema: tool.input_schema.clone(),
    }
}

fn prov_content_block(
    block: &ext::oino::extension::types::ContentBlock,
) -> prov::oino::extension::types::ContentBlock {
    use prov::oino::extension::types::ContentBlock as Prov;
    match block {
        ext::oino::extension::types::ContentBlock::Text(text) => Prov::Text(text.clone()),
        ext::oino::extension::types::ContentBlock::Thinking(text) => Prov::Thinking(text.clone()),
        ext::oino::extension::types::ContentBlock::ToolCall(json) => Prov::ToolCall(json.clone()),
        ext::oino::extension::types::ContentBlock::Image(json) => Prov::Image(json.clone()),
        ext::oino::extension::types::ContentBlock::Custom(json) => Prov::Custom(json.clone()),
    }
}

#[doc(hidden)]
pub mod dispatch {
    use super::*;
    use prov::exports::oino::extension::guest as pguest;
    use prov::exports::oino::extension::provider as pprovider;
    use prov::oino::extension::types as ptypes;

    pub fn init<T: Extension>(config: ptypes::Json) -> Result<pguest::Registration, String> {
        let registration = T::init(json_in(&config)?)?;
        Ok(pguest::Registration {
            tools: registration
                .tools
                .iter()
                .map(prov_tool_declaration)
                .collect(),
            commands: registration
                .commands
                .into_iter()
                .map(|(name, description)| pguest::CommandDecl { name, description })
                .collect(),
            hooks: registration.hooks,
            providers: registration.providers,
        })
    }

    pub fn handle_event<T: Extension>(
        kind: String,
        payload: ptypes::Json,
    ) -> Result<ptypes::Json, String> {
        Ok(json_out(&T::handle_event(&kind, json_in(&payload)?)?))
    }

    pub fn run_tool<T: Extension>(
        name: String,
        call_id: String,
        args: ptypes::Json,
    ) -> Result<pguest::ToolOutput, String> {
        let output = T::run_tool(&name, &call_id, json_in(&args)?)?;
        Ok(pguest::ToolOutput {
            content: output.content.iter().map(prov_content_block).collect(),
            is_error: output.is_error,
            details: output.details.map(|v| json_out(&v)),
            added_tool_names: output.added_tool_names,
            terminate: output.terminate,
        })
    }

    pub fn run_command<T: Extension>(name: String, args: String) -> Result<ptypes::Json, String> {
        Ok(json_out(&T::run_command(&name, &args)?))
    }

    pub fn list_models<T: Provider>() -> Result<ptypes::Json, String> {
        Ok(json_out(&T::list_models()?))
    }

    pub fn stream<T: Provider>(request: ptypes::Json) -> Result<pprovider::Response, String> {
        let stream = T::stream(json_in(&request)?)?;
        Ok(pprovider::Response::new(ResponseImpl(stream)))
    }
}

#[doc(hidden)]
pub struct ResponseImpl(pub ResponseStream);

impl prov::exports::oino::extension::provider::GuestResponse for ResponseImpl {
    fn next(&self) -> Result<Option<Vec<prov::oino::extension::types::AssistantEvent>>, String> {
        self.0
             .0
            .lock()
            .map_err(|_| "stream lock".to_string())?
            .next()
            .transpose()
    }

    fn cancel(&self) {}
}

/// Export `T` as the `provider-extension` world's guest + provider interfaces.
/// `T` must implement both [`Extension`] and [`Provider`].
#[macro_export]
macro_rules! export_provider {
    ($t:ty) => {
        struct OinoProviderExport;
        impl $crate::prov::exports::oino::extension::guest::Guest for OinoProviderExport {
            fn init(
                config: $crate::prov::oino::extension::types::Json,
            ) -> Result<$crate::prov::exports::oino::extension::guest::Registration, String> {
                $crate::dispatch::init::<$t>(config)
            }
            fn handle_event(
                kind: String,
                payload: $crate::prov::oino::extension::types::Json,
            ) -> Result<$crate::prov::oino::extension::types::Json, String> {
                $crate::dispatch::handle_event::<$t>(kind, payload)
            }
            fn run_tool(
                name: String,
                call_id: String,
                args: $crate::prov::oino::extension::types::Json,
            ) -> Result<$crate::prov::exports::oino::extension::guest::ToolOutput, String> {
                $crate::dispatch::run_tool::<$t>(name, call_id, args)
            }
            fn run_command(
                name: String,
                args: String,
            ) -> Result<$crate::prov::oino::extension::types::Json, String> {
                $crate::dispatch::run_command::<$t>(name, args)
            }
        }
        impl $crate::prov::exports::oino::extension::provider::Guest for OinoProviderExport {
            type Response = $crate::ResponseImpl;
            fn list_models() -> Result<$crate::prov::oino::extension::types::Json, String> {
                $crate::dispatch::list_models::<$t>()
            }
            fn stream(
                request: $crate::prov::oino::extension::types::Json,
            ) -> Result<$crate::prov::exports::oino::extension::provider::Response, String> {
                $crate::dispatch::stream::<$t>(request)
            }
        }
        $crate::prov::export_provider_extension!(OinoProviderExport with_types_in $crate::prov);
    };
}
