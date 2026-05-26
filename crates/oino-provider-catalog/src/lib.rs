#![doc = r#"Provider metadata catalog for Oino auth and runtime selection.

This crate is deliberately data-only: it describes provider ids, display names, auth
methods, aliases, setup hints, OpenAI-compatible endpoints, and default model hints
without performing credential I/O, network requests, or TUI rendering.

The catalog values are normalized for Oino's provider-neutral auth/runtime UX.
"#]
#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAuthKind {
    OAuth,
    ApiKey,
    DeviceCode,
    Cli,
    Hybrid,
    Local,
}

impl ProviderAuthKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::OAuth => "OAuth",
            Self::ApiKey => "API key",
            Self::DeviceCode => "device code",
            Self::Cli => "CLI",
            Self::Hybrid => "API key / CLI",
            Self::Local => "local endpoint",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRuntimeSupport {
    OpenAiCompatible,
    Native,
    MetadataOnly,
    Import,
}

impl ProviderRuntimeSupport {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "OpenAI-compatible",
            Self::Native => "native",
            Self::MetadataOnly => "metadata-only",
            Self::Import => "import/setup",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTarget {
    AutoImport,
    Claude,
    OpenAi,
    OpenAiApiKey,
    OpenRouter,
    Bedrock,
    Azure,
    OpenAiCompatible { profile_id: &'static str },
    Cursor,
    Copilot,
    Gemini,
    Antigravity,
    Google,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub auth_kind: ProviderAuthKind,
    pub aliases: &'static [&'static str],
    pub menu_detail: &'static str,
    pub recommended: bool,
    pub target: ProviderTarget,
    pub runtime: ProviderRuntimeSupport,
}

impl ProviderDescriptor {
    #[must_use]
    pub fn openai_compatible_profile(self) -> Option<&'static OpenAiCompatibleProfile> {
        match self.target {
            ProviderTarget::OpenAiCompatible { profile_id } => {
                openai_compatible_profile_by_id(profile_id)
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn default_model(self) -> Option<&'static str> {
        match self.target {
            ProviderTarget::Claude => Some("claude-3-5-sonnet-latest"),
            ProviderTarget::OpenRouter => Some("openai/gpt-4o-mini"),
            ProviderTarget::OpenAiApiKey => Some("gpt-4o-mini"),

            ProviderTarget::OpenAiCompatible { profile_id } => {
                openai_compatible_profile_by_id(profile_id)
                    .and_then(|profile| profile.default_model)
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn credential_spec(self) -> ProviderCredentialSpec {
        provider_credential_spec(self.id).unwrap_or_else(|| ProviderCredentialSpec::none(self.id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCredentialSpec {
    pub provider_id: &'static str,
    pub auth_key: &'static str,
    pub env_var: Option<&'static str>,
    pub env_file: Option<&'static str>,
    pub setup_url: Option<&'static str>,
    pub requires_api_key: bool,
}

impl ProviderCredentialSpec {
    #[must_use]
    pub const fn api_key(
        provider_id: &'static str,
        auth_key: &'static str,
        env_var: &'static str,
        env_file: &'static str,
        setup_url: &'static str,
    ) -> Self {
        Self {
            provider_id,
            auth_key,
            env_var: Some(env_var),
            env_file: Some(env_file),
            setup_url: Some(setup_url),
            requires_api_key: true,
        }
    }

    #[must_use]
    pub const fn optional_api_key(
        provider_id: &'static str,
        auth_key: &'static str,
        env_var: &'static str,
        env_file: &'static str,
        setup_url: &'static str,
    ) -> Self {
        Self {
            provider_id,
            auth_key,
            env_var: Some(env_var),
            env_file: Some(env_file),
            setup_url: Some(setup_url),
            requires_api_key: false,
        }
    }

    #[must_use]
    pub const fn none(provider_id: &'static str) -> Self {
        Self {
            provider_id,
            auth_key: provider_id,
            env_var: None,
            env_file: None,
            setup_url: None,
            requires_api_key: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenAiCompatibleProfile {
    pub id: &'static str,
    pub display_name: &'static str,
    pub api_base: &'static str,
    pub api_key_env: &'static str,
    pub env_file: &'static str,
    pub setup_url: &'static str,
    pub default_model: Option<&'static str>,
    pub requires_api_key: bool,
    pub aliases: &'static [&'static str],
}

impl OpenAiCompatibleProfile {
    #[must_use]
    pub const fn credential_spec(self) -> ProviderCredentialSpec {
        let setup_url = self.setup_url;
        if self.requires_api_key {
            ProviderCredentialSpec::api_key(
                self.id,
                self.id,
                self.api_key_env,
                self.env_file,
                setup_url,
            )
        } else {
            ProviderCredentialSpec::optional_api_key(
                self.id,
                self.id,
                self.api_key_env,
                self.env_file,
                setup_url,
            )
        }
    }
}

pub const PROVIDER_COUNT: usize = 44;
pub const OPENAI_COMPATIBLE_PROFILE_COUNT: usize = 32;

pub const OPENAI_COMPATIBLE_PROFILES: [OpenAiCompatibleProfile; OPENAI_COMPATIBLE_PROFILE_COUNT] = [
    OpenAiCompatibleProfile {
        id: "opencode",
        display_name: "OpenCode Zen",
        api_base: "https://opencode.ai/zen/v1",
        api_key_env: "OPENCODE_API_KEY",
        env_file: "opencode.env",
        setup_url: "https://opencode.ai/docs/providers#opencode-zen",
        default_model: Some("minimax-m2.7"),
        requires_api_key: true,
        aliases: &["opencode-zen", "zen"],
    },
    OpenAiCompatibleProfile {
        id: "opencode-go",
        display_name: "OpenCode Go",
        api_base: "https://opencode.ai/zen/go/v1",
        api_key_env: "OPENCODE_GO_API_KEY",
        env_file: "opencode-go.env",
        setup_url: "https://opencode.ai/docs/providers#opencode-go",
        default_model: Some("kimi-k2.5"),
        requires_api_key: true,
        aliases: &["opencodego"],
    },
    OpenAiCompatibleProfile {
        id: "zai",
        display_name: "Z.AI",
        api_base: "https://api.z.ai/api/coding/paas/v4",
        api_key_env: "ZHIPU_API_KEY",
        env_file: "zai.env",
        setup_url: "https://docs.z.ai/guides/develop/openai/introduction",
        default_model: Some("glm-4.5"),
        requires_api_key: true,
        aliases: &["z.ai", "z-ai", "zai-coding", "zhipu"],
    },
    OpenAiCompatibleProfile {
        id: "kimi",
        display_name: "Kimi Code",
        api_base: "https://api.kimi.com/coding/v1",
        api_key_env: "KIMI_API_KEY",
        env_file: "kimi.env",
        setup_url: "https://www.kimi.com/coding/docs/en/more/third-party-agents.html",
        default_model: Some("kimi-for-coding"),
        requires_api_key: true,
        aliases: &[
            "kimi-code",
            "kimi-coding",
            "kimi-coding-plan",
            "kimi-for-coding",
            "moonshot-coding",
        ],
    },
    OpenAiCompatibleProfile {
        id: "302ai",
        display_name: "302.AI",
        api_base: "https://api.302.ai/v1",
        api_key_env: "302AI_API_KEY",
        env_file: "302ai.env",
        setup_url: "https://opencode.ai/docs/providers#302ai",
        default_model: Some("qwen3-235b-a22b-instruct-2507"),
        requires_api_key: true,
        aliases: &["302.ai"],
    },
    OpenAiCompatibleProfile {
        id: "baseten",
        display_name: "Baseten",
        api_base: "https://inference.baseten.co/v1",
        api_key_env: "BASETEN_API_KEY",
        env_file: "baseten.env",
        setup_url: "https://opencode.ai/docs/providers#baseten",
        default_model: Some("zai-org/GLM-4.7"),
        requires_api_key: true,
        aliases: &[],
    },
    OpenAiCompatibleProfile {
        id: "cortecs",
        display_name: "Cortecs",
        api_base: "https://api.cortecs.ai/v1",
        api_key_env: "CORTECS_API_KEY",
        env_file: "cortecs.env",
        setup_url: "https://opencode.ai/docs/providers#cortecs",
        default_model: Some("kimi-k2.5"),
        requires_api_key: true,
        aliases: &[],
    },
    OpenAiCompatibleProfile {
        id: "deepseek",
        display_name: "DeepSeek",
        api_base: "https://api.deepseek.com",
        api_key_env: "DEEPSEEK_API_KEY",
        env_file: "deepseek.env",
        setup_url: "https://api-docs.deepseek.com/",
        default_model: Some("deepseek-v4-flash"),
        requires_api_key: true,
        aliases: &[],
    },
    OpenAiCompatibleProfile {
        id: "comtegra",
        display_name: "Comtegra GPU Cloud",
        api_base: "https://llm.comtegra.cloud/v1",
        api_key_env: "COMTEGRA_API_KEY",
        env_file: "comtegra.env",
        setup_url: "https://docs.cgc.comtegra.cloud/llm-api",
        default_model: Some("glm-51-nvfp4"),
        requires_api_key: true,
        aliases: &["cgc", "comtegra-gpu-cloud"],
    },
    OpenAiCompatibleProfile {
        id: "fpt",
        display_name: "FPT AI Marketplace",
        api_base: "https://mkp-api.fptcloud.com",
        api_key_env: "FPT_API_KEY",
        env_file: "fpt.env",
        setup_url: "https://ai-docs.fptcloud.com/api-reference/ai-marketplace/api-reference/api-integration-large-language-model-md",
        default_model: Some("GLM-5.1"),
        requires_api_key: true,
        aliases: &["fpt-ai", "fptcloud", "fpt-cloud"],
    },
    OpenAiCompatibleProfile {
        id: "firmware",
        display_name: "Firmware",
        api_base: "https://app.frogbot.ai/api/v1",
        api_key_env: "FIRMWARE_API_KEY",
        env_file: "firmware.env",
        setup_url: "https://opencode.ai/docs/providers#firmware",
        default_model: Some("kimi-k2.5"),
        requires_api_key: true,
        aliases: &[],
    },
    OpenAiCompatibleProfile {
        id: "huggingface",
        display_name: "Hugging Face",
        api_base: "https://router.huggingface.co/v1",
        api_key_env: "HF_TOKEN",
        env_file: "huggingface.env",
        setup_url: "https://opencode.ai/docs/providers#hugging-face",
        default_model: Some("zai-org/GLM-4.7"),
        requires_api_key: true,
        aliases: &["hugging-face", "hf"],
    },
    OpenAiCompatibleProfile {
        id: "moonshotai",
        display_name: "Moonshot AI",
        api_base: "https://api.moonshot.ai/v1",
        api_key_env: "MOONSHOT_API_KEY",
        env_file: "moonshotai.env",
        setup_url: "https://opencode.ai/docs/providers#moonshot-ai",
        default_model: Some("kimi-k2.5"),
        requires_api_key: true,
        aliases: &["moonshot"],
    },
    OpenAiCompatibleProfile {
        id: "nebius",
        display_name: "Nebius Token Factory",
        api_base: "https://api.tokenfactory.nebius.com/v1",
        api_key_env: "NEBIUS_API_KEY",
        env_file: "nebius.env",
        setup_url: "https://opencode.ai/docs/providers#nebius-token-factory",
        default_model: Some("openai/gpt-oss-120b"),
        requires_api_key: true,
        aliases: &[],
    },
    OpenAiCompatibleProfile {
        id: "scaleway",
        display_name: "Scaleway",
        api_base: "https://api.scaleway.ai/v1",
        api_key_env: "SCALEWAY_API_KEY",
        env_file: "scaleway.env",
        setup_url: "https://opencode.ai/docs/providers#scaleway",
        default_model: Some("qwen3-coder-30b-a3b-instruct"),
        requires_api_key: true,
        aliases: &[],
    },
    OpenAiCompatibleProfile {
        id: "stackit",
        display_name: "STACKIT",
        api_base: "https://api.openai-compat.model-serving.eu01.onstackit.cloud/v1",
        api_key_env: "STACKIT_API_KEY",
        env_file: "stackit.env",
        setup_url: "https://opencode.ai/docs/providers#stackit",
        default_model: Some("openai/gpt-oss-120b"),
        requires_api_key: true,
        aliases: &[],
    },
    OpenAiCompatibleProfile {
        id: "groq",
        display_name: "Groq",
        api_base: "https://api.groq.com/openai/v1",
        api_key_env: "GROQ_API_KEY",
        env_file: "groq.env",
        setup_url: "https://console.groq.com/docs/openai",
        default_model: Some("llama-3.1-8b-instant"),
        requires_api_key: true,
        aliases: &[],
    },
    OpenAiCompatibleProfile {
        id: "mistral",
        display_name: "Mistral",
        api_base: "https://api.mistral.ai/v1",
        api_key_env: "MISTRAL_API_KEY",
        env_file: "mistral.env",
        setup_url: "https://docs.mistral.ai/getting-started/models/",
        default_model: Some("devstral-medium-2507"),
        requires_api_key: true,
        aliases: &["mistralai"],
    },
    OpenAiCompatibleProfile {
        id: "perplexity",
        display_name: "Perplexity",
        api_base: "https://api.perplexity.ai",
        api_key_env: "PERPLEXITY_API_KEY",
        env_file: "perplexity.env",
        setup_url: "https://docs.perplexity.ai/docs/agent-api/openai-compatibility",
        default_model: Some("sonar"),
        requires_api_key: true,
        aliases: &["pplx"],
    },
    OpenAiCompatibleProfile {
        id: "togetherai",
        display_name: "Together AI",
        api_base: "https://api.together.xyz/v1",
        api_key_env: "TOGETHER_API_KEY",
        env_file: "togetherai.env",
        setup_url: "https://docs.together.ai/docs/openai-api-compatibility",
        default_model: Some("moonshotai/Kimi-K2-Instruct"),
        requires_api_key: true,
        aliases: &["together", "together-ai"],
    },
    OpenAiCompatibleProfile {
        id: "deepinfra",
        display_name: "Deep Infra",
        api_base: "https://api.deepinfra.com/v1/openai",
        api_key_env: "DEEPINFRA_API_KEY",
        env_file: "deepinfra.env",
        setup_url: "https://deepinfra.com/docs/api-reference",
        default_model: Some("moonshotai/Kimi-K2-Instruct"),
        requires_api_key: true,
        aliases: &["deep-infra"],
    },
    OpenAiCompatibleProfile {
        id: "fireworks",
        display_name: "Fireworks",
        api_base: "https://api.fireworks.ai/inference/v1",
        api_key_env: "FIREWORKS_API_KEY",
        env_file: "fireworks.env",
        setup_url: "https://docs.fireworks.ai/tools-sdks/openai-compatibility",
        default_model: Some("accounts/fireworks/routers/kimi-k2p5-turbo"),
        requires_api_key: true,
        aliases: &["fireworks-ai", "fireworks.ai"],
    },
    OpenAiCompatibleProfile {
        id: "minimax",
        display_name: "MiniMax",
        api_base: "https://api.minimax.io/v1",
        api_key_env: "OPENAI_API_KEY",
        env_file: "minimax.env",
        setup_url: "https://platform.minimax.io/docs/guides/text-generation",
        default_model: Some("MiniMax-M2.7"),
        requires_api_key: true,
        aliases: &["minimaxi", "minimax-ai"],
    },
    OpenAiCompatibleProfile {
        id: "xai",
        display_name: "xAI",
        api_base: "https://api.x.ai/v1",
        api_key_env: "XAI_API_KEY",
        env_file: "xai.env",
        setup_url: "https://docs.x.ai/developers/quickstart",
        default_model: Some("grok-code-fast-1"),
        requires_api_key: true,
        aliases: &["x.ai", "x-ai", "grok"],
    },
    OpenAiCompatibleProfile {
        id: "lmstudio",
        display_name: "LM Studio",
        api_base: "http://localhost:1234/v1",
        api_key_env: "LMSTUDIO_API_KEY",
        env_file: "lmstudio.env",
        setup_url: "https://lmstudio.ai/docs/app/api/endpoints/openai",
        default_model: None,
        requires_api_key: false,
        aliases: &["lm-studio"],
    },
    OpenAiCompatibleProfile {
        id: "ollama",
        display_name: "Ollama",
        api_base: "http://localhost:11434/v1",
        api_key_env: "OLLAMA_API_KEY",
        env_file: "ollama.env",
        setup_url: "https://docs.ollama.com/api/openai-compatibility",
        default_model: None,
        requires_api_key: false,
        aliases: &[],
    },
    OpenAiCompatibleProfile {
        id: "chutes",
        display_name: "Chutes",
        api_base: "https://llm.chutes.ai/v1",
        api_key_env: "CHUTES_API_KEY",
        env_file: "chutes.env",
        setup_url: "https://chutes.ai",
        default_model: None,
        requires_api_key: true,
        aliases: &[],
    },
    OpenAiCompatibleProfile {
        id: "cerebras",
        display_name: "Cerebras",
        api_base: "https://api.cerebras.ai/v1",
        api_key_env: "CEREBRAS_API_KEY",
        env_file: "cerebras.env",
        setup_url: "https://inference-docs.cerebras.ai/introduction",
        default_model: Some("qwen-3-235b-a22b-instruct-2507"),
        requires_api_key: true,
        aliases: &["cerebrascode", "cerberascode"],
    },
    OpenAiCompatibleProfile {
        id: "alibaba-coding-plan",
        display_name: "Alibaba Cloud Coding Plan",
        api_base: "https://coding-intl.dashscope.aliyuncs.com/v1",
        api_key_env: "BAILIAN_CODING_PLAN_API_KEY",
        env_file: "alibaba-coding-plan.env",
        setup_url: "https://www.alibabacloud.com/help/en/model-studio/coding-plan-quickstart",
        default_model: Some("qwen3-coder-plus"),
        requires_api_key: true,
        aliases: &["bailian", "aliyun-bailian", "coding-plan", "alibaba-coding"],
    },
    OpenAiCompatibleProfile {
        id: "nvidia-nim",
        display_name: "NVIDIA NIM",
        api_base: "https://integrate.api.nvidia.com/v1",
        api_key_env: "NVIDIA_API_KEY",
        env_file: "nvidia-nim.env",
        setup_url: "https://build.nvidia.com/explore/discover",
        default_model: Some("nvidia/llama-3.1-nemotron-ultra-253b-v1"),
        requires_api_key: true,
        aliases: &["nvidia", "nim"],
    },
    OpenAiCompatibleProfile {
        id: "xiaomi-mimo",
        display_name: "Xiaomi MiMo",
        api_base: "https://api.xiaomimimo.com/v1",
        api_key_env: "XIAOMI_MIMO_API_KEY",
        env_file: "xiaomi-mimo.env",
        setup_url: "https://platform.xiaomimimo.com",
        default_model: Some("mimo-v2.5"),
        requires_api_key: true,
        aliases: &["xiaomi", "mimo", "xiaomi-mimo-api"],
    },
    OpenAiCompatibleProfile {
        id: "openai-compatible",
        display_name: "OpenAI-compatible",
        api_base: "https://api.openai.com/v1",
        api_key_env: "OPENAI_COMPAT_API_KEY",
        env_file: "openai-compatible.env",
        setup_url: "https://platform.openai.com/docs/api-reference/chat",
        default_model: None,
        requires_api_key: true,
        aliases: &["openai_compatible", "compat", "custom"],
    },
];

pub const PROVIDERS: [ProviderDescriptor; PROVIDER_COUNT] = [
    ProviderDescriptor {
        id: "auto-import",
        display_name: "Auto Import",
        auth_kind: ProviderAuthKind::Local,
        aliases: &["import", "reuse", "autoimport"],
        menu_detail: "review and reuse logins from other tools",
        recommended: false,
        target: ProviderTarget::AutoImport,
        runtime: ProviderRuntimeSupport::Import,
    },
    ProviderDescriptor {
        id: "claude",
        display_name: "Anthropic/Claude",
        auth_kind: ProviderAuthKind::Hybrid,
        aliases: &["anthropic"],
        menu_detail: "Anthropic API key runtime; Claude OAuth staged",
        recommended: true,
        target: ProviderTarget::Claude,
        runtime: ProviderRuntimeSupport::Native,
    },
    ProviderDescriptor {
        id: "openai",
        display_name: "OpenAI",
        auth_kind: ProviderAuthKind::OAuth,
        aliases: &[],
        menu_detail: "requires ChatGPT Plus or Pro subscription",
        recommended: true,
        target: ProviderTarget::OpenAi,
        runtime: ProviderRuntimeSupport::Native,
    },
    ProviderDescriptor {
        id: "openai-api",
        display_name: "OpenAI API",
        auth_kind: ProviderAuthKind::ApiKey,
        aliases: &[
            "openai-key",
            "openai-apikey",
            "openai-platform",
            "platform-openai",
        ],
        menu_detail: "native OpenAI API key, pay-per-token",
        recommended: false,
        target: ProviderTarget::OpenAiApiKey,
        runtime: ProviderRuntimeSupport::OpenAiCompatible,
    },
    ProviderDescriptor {
        id: "openrouter",
        display_name: "OpenRouter",
        auth_kind: ProviderAuthKind::ApiKey,
        aliases: &[],
        menu_detail: "API key, pay-per-token, 200+ models",
        recommended: false,
        target: ProviderTarget::OpenRouter,
        runtime: ProviderRuntimeSupport::OpenAiCompatible,
    },
    ProviderDescriptor {
        id: "bedrock",
        display_name: "AWS Bedrock",
        auth_kind: ProviderAuthKind::ApiKey,
        aliases: &["aws-bedrock", "aws_bedrock"],
        menu_detail: "Bedrock API key or AWS credentials, pay-per-token",
        recommended: false,
        target: ProviderTarget::Bedrock,
        runtime: ProviderRuntimeSupport::Native,
    },
    ProviderDescriptor {
        id: "azure",
        display_name: "Azure OpenAI",
        auth_kind: ProviderAuthKind::Hybrid,
        aliases: &["azure-openai", "azure_openai", "aoai"],
        menu_detail: "Microsoft Entra ID or Azure OpenAI API key",
        recommended: false,
        target: ProviderTarget::Azure,
        runtime: ProviderRuntimeSupport::Native,
    },
    profile_provider("opencode"),
    profile_provider("opencode-go"),
    profile_provider("zai"),
    profile_provider("kimi"),
    profile_provider("chutes"),
    profile_provider("cerebras"),
    profile_provider("alibaba-coding-plan"),
    profile_provider("302ai"),
    profile_provider("baseten"),
    profile_provider("cortecs"),
    profile_provider("deepseek"),
    profile_provider("comtegra"),
    profile_provider("fpt"),
    profile_provider("firmware"),
    profile_provider("huggingface"),
    profile_provider("moonshotai"),
    profile_provider("nebius"),
    profile_provider("scaleway"),
    profile_provider("stackit"),
    profile_provider("groq"),
    profile_provider("mistral"),
    profile_provider("perplexity"),
    profile_provider("togetherai"),
    profile_provider("deepinfra"),
    profile_provider("fireworks"),
    profile_provider("minimax"),
    profile_provider("xai"),
    profile_provider("nvidia-nim"),
    profile_provider("lmstudio"),
    profile_provider("ollama"),
    profile_provider("openai-compatible"),
    ProviderDescriptor {
        id: "cursor",
        display_name: "Cursor",
        auth_kind: ProviderAuthKind::Hybrid,
        aliases: &[],
        menu_detail: "browser login or API key",
        recommended: false,
        target: ProviderTarget::Cursor,
        runtime: ProviderRuntimeSupport::Native,
    },
    ProviderDescriptor {
        id: "copilot",
        display_name: "GitHub Copilot",
        auth_kind: ProviderAuthKind::DeviceCode,
        aliases: &[],
        menu_detail: "GitHub device flow",
        recommended: false,
        target: ProviderTarget::Copilot,
        runtime: ProviderRuntimeSupport::Native,
    },
    ProviderDescriptor {
        id: "gemini",
        display_name: "Google Gemini",
        auth_kind: ProviderAuthKind::OAuth,
        aliases: &[],
        menu_detail: "Google Gemini Code Assist OAuth login",
        recommended: false,
        target: ProviderTarget::Gemini,
        runtime: ProviderRuntimeSupport::Native,
    },
    ProviderDescriptor {
        id: "antigravity",
        display_name: "Antigravity",
        auth_kind: ProviderAuthKind::OAuth,
        aliases: &[],
        menu_detail: "Google Antigravity OAuth login",
        recommended: false,
        target: ProviderTarget::Antigravity,
        runtime: ProviderRuntimeSupport::Native,
    },
    profile_provider("xiaomi-mimo"),
    ProviderDescriptor {
        id: "google",
        display_name: "Google/Gmail",
        auth_kind: ProviderAuthKind::OAuth,
        aliases: &["gmail"],
        menu_detail: "read, draft, and send emails",
        recommended: false,
        target: ProviderTarget::Google,
        runtime: ProviderRuntimeSupport::MetadataOnly,
    },
];

const fn profile_provider(profile_id: &'static str) -> ProviderDescriptor {
    ProviderDescriptor {
        id: profile_id,
        display_name: profile_display_name(profile_id),
        auth_kind: profile_auth_kind(profile_id),
        aliases: profile_aliases(profile_id),
        menu_detail: profile_menu_detail(profile_id),
        recommended: false,
        target: ProviderTarget::OpenAiCompatible { profile_id },
        runtime: ProviderRuntimeSupport::OpenAiCompatible,
    }
}

const fn profile_display_name(profile_id: &'static str) -> &'static str {
    match profile_id.as_bytes() {
        b"opencode" => "OpenCode Zen",
        b"opencode-go" => "OpenCode Go",
        b"zai" => "Z.AI",
        b"kimi" => "Kimi Code",
        b"302ai" => "302.AI",
        b"baseten" => "Baseten",
        b"cortecs" => "Cortecs",
        b"deepseek" => "DeepSeek",
        b"comtegra" => "Comtegra GPU Cloud",
        b"fpt" => "FPT AI Marketplace",
        b"firmware" => "Firmware",
        b"huggingface" => "Hugging Face",
        b"moonshotai" => "Moonshot AI",
        b"nebius" => "Nebius Token Factory",
        b"scaleway" => "Scaleway",
        b"stackit" => "STACKIT",
        b"groq" => "Groq",
        b"mistral" => "Mistral",
        b"perplexity" => "Perplexity",
        b"togetherai" => "Together AI",
        b"deepinfra" => "Deep Infra",
        b"fireworks" => "Fireworks",
        b"minimax" => "MiniMax",
        b"xai" => "xAI",
        b"lmstudio" => "LM Studio",
        b"ollama" => "Ollama",
        b"chutes" => "Chutes",
        b"cerebras" => "Cerebras",
        b"alibaba-coding-plan" => "Alibaba Cloud Coding Plan",
        b"nvidia-nim" => "NVIDIA NIM",
        b"xiaomi-mimo" => "Xiaomi MiMo",
        b"openai-compatible" => "OpenAI-compatible",
        _ => "OpenAI-compatible",
    }
}

const fn profile_auth_kind(profile_id: &'static str) -> ProviderAuthKind {
    match profile_id.as_bytes() {
        b"lmstudio" | b"ollama" => ProviderAuthKind::Local,
        b"openai-compatible" => ProviderAuthKind::Hybrid,
        _ => ProviderAuthKind::ApiKey,
    }
}

const fn profile_aliases(profile_id: &'static str) -> &'static [&'static str] {
    match profile_id.as_bytes() {
        b"opencode" => &["opencode-zen", "zen"],
        b"opencode-go" => &["opencodego"],
        b"zai" => &["z.ai", "z-ai", "zai-coding", "zhipu"],
        b"kimi" => &[
            "kimi-code",
            "kimi-coding",
            "kimi-coding-plan",
            "kimi-for-coding",
            "moonshot-coding",
        ],
        b"302ai" => &["302.ai"],
        b"comtegra" => &["cgc", "comtegra-gpu-cloud"],
        b"fpt" => &["fpt-ai", "fptcloud", "fpt-cloud"],
        b"huggingface" => &["hugging-face", "hf"],
        b"moonshotai" => &["moonshot"],
        b"mistral" => &["mistralai"],
        b"perplexity" => &["pplx"],
        b"togetherai" => &["together", "together-ai"],
        b"deepinfra" => &["deep-infra"],
        b"fireworks" => &["fireworks-ai", "fireworks.ai"],
        b"minimax" => &["minimaxi", "minimax-ai"],
        b"xai" => &["x.ai", "x-ai", "grok"],
        b"nvidia-nim" => &["nvidia", "nim"],
        b"lmstudio" => &["lm-studio"],
        b"openai-compatible" => &["openai_compatible", "compat", "custom"],
        b"cerebras" => &["cerebrascode", "cerberascode"],
        b"alibaba-coding-plan" => &["bailian", "aliyun-bailian", "coding-plan", "alibaba-coding"],
        b"xiaomi-mimo" => &["xiaomi", "mimo", "xiaomi-mimo-api"],
        _ => &[],
    }
}

const fn profile_menu_detail(profile_id: &'static str) -> &'static str {
    match profile_id.as_bytes() {
        b"kimi" => "API key, dedicated Kimi coding endpoint",
        b"alibaba-coding-plan" => "API key, dedicated Alibaba coding endpoint",
        b"comtegra" => "OpenAI-compatible LLM API",
        b"fpt" => "OpenAI-compatible FPT AI Marketplace API",
        b"lmstudio" | b"ollama" => "local OpenAI-compatible endpoint",
        b"openai-compatible" => "custom endpoint setup: base URL first, then API key",
        b"xiaomi-mimo" => "OpenAI-compatible Xiaomi MiMo API",
        _ => "API key",
    }
}

#[must_use]
pub fn providers() -> &'static [ProviderDescriptor] {
    &PROVIDERS
}

#[must_use]
pub fn openai_compatible_profiles() -> &'static [OpenAiCompatibleProfile] {
    &OPENAI_COMPATIBLE_PROFILES
}

#[must_use]
pub fn provider_by_id(id: &str) -> Option<&'static ProviderDescriptor> {
    let normalized = normalize_provider_input(id)?;
    providers()
        .iter()
        .find(|provider| provider.id == normalized)
}

#[must_use]
pub fn resolve_provider(input: &str) -> Option<&'static ProviderDescriptor> {
    let normalized = normalize_provider_input(input)?;
    providers().iter().find(|provider| {
        provider.id == normalized || provider.aliases.iter().any(|alias| *alias == normalized)
    })
}

#[must_use]
pub fn openai_compatible_profile_by_id(id: &str) -> Option<&'static OpenAiCompatibleProfile> {
    let normalized = normalize_provider_input(id)?;
    openai_compatible_profiles()
        .iter()
        .find(|profile| profile.id == normalized)
}

#[must_use]
pub fn resolve_openai_compatible_profile(input: &str) -> Option<&'static OpenAiCompatibleProfile> {
    let normalized = normalize_provider_input(input)?;
    openai_compatible_profiles().iter().find(|profile| {
        profile.id == normalized || profile.aliases.iter().any(|alias| *alias == normalized)
    })
}

#[must_use]
pub fn provider_credential_spec(provider_id: &str) -> Option<ProviderCredentialSpec> {
    let provider = provider_by_id(provider_id)?;
    match provider.target {
        ProviderTarget::AutoImport => Some(ProviderCredentialSpec::none(provider.id)),

        ProviderTarget::Claude => Some(ProviderCredentialSpec::api_key(
            "claude",
            "claude",
            "ANTHROPIC_API_KEY",
            "anthropic.env",
            "https://console.anthropic.com/settings/keys",
        )),
        ProviderTarget::OpenAi => Some(ProviderCredentialSpec::optional_api_key(
            "openai",
            "openai",
            "OPENAI_API_KEY",
            "openai.env",
            "https://platform.openai.com/api-keys",
        )),
        ProviderTarget::OpenAiApiKey => Some(ProviderCredentialSpec::api_key(
            "openai-api",
            "openai-api",
            "OPENAI_API_KEY",
            "openai.env",
            "https://platform.openai.com/api-keys",
        )),
        ProviderTarget::OpenRouter => Some(ProviderCredentialSpec::api_key(
            "openrouter",
            "openrouter",
            "OPENROUTER_API_KEY",
            "openrouter.env",
            "https://openrouter.ai/keys",
        )),
        ProviderTarget::Bedrock => Some(ProviderCredentialSpec::optional_api_key(
            "bedrock",
            "bedrock",
            "AWS_BEARER_TOKEN_BEDROCK",
            "bedrock.env",
            "https://docs.aws.amazon.com/bedrock/latest/userguide/",
        )),
        ProviderTarget::Azure => Some(ProviderCredentialSpec::api_key(
            "azure",
            "azure",
            "AZURE_OPENAI_API_KEY",
            "azure-openai.env",
            "https://learn.microsoft.com/azure/ai-services/openai/",
        )),
        ProviderTarget::OpenAiCompatible { profile_id } => {
            openai_compatible_profile_by_id(profile_id).map(|profile| profile.credential_spec())
        }
        ProviderTarget::Cursor => Some(ProviderCredentialSpec::optional_api_key(
            "cursor",
            "cursor",
            "CURSOR_API_KEY",
            "cursor.env",
            "https://cursor.com/",
        )),
        ProviderTarget::Copilot => Some(ProviderCredentialSpec::optional_api_key(
            "copilot",
            "copilot",
            "GITHUB_TOKEN",
            "copilot.env",
            "https://github.com/settings/copilot",
        )),
        ProviderTarget::Gemini => Some(ProviderCredentialSpec::none("gemini")),
        ProviderTarget::Antigravity => Some(ProviderCredentialSpec::none("antigravity")),
        ProviderTarget::Google => Some(ProviderCredentialSpec::none("google")),
    }
}

#[must_use]
pub fn is_safe_env_key_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

#[must_use]
pub fn is_safe_env_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
}

fn normalize_provider_input(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const EXPECTED_PROVIDER_IDS: &[&str] = &[
        "auto-import",
        "claude",
        "openai",
        "openai-api",
        "openrouter",
        "bedrock",
        "azure",
        "opencode",
        "opencode-go",
        "zai",
        "kimi",
        "chutes",
        "cerebras",
        "alibaba-coding-plan",
        "302ai",
        "baseten",
        "cortecs",
        "deepseek",
        "comtegra",
        "fpt",
        "firmware",
        "huggingface",
        "moonshotai",
        "nebius",
        "scaleway",
        "stackit",
        "groq",
        "mistral",
        "perplexity",
        "togetherai",
        "deepinfra",
        "fireworks",
        "minimax",
        "xai",
        "nvidia-nim",
        "lmstudio",
        "ollama",
        "openai-compatible",
        "cursor",
        "copilot",
        "gemini",
        "antigravity",
        "xiaomi-mimo",
        "google",
    ];

    #[test]
    fn provider_catalog_contains_expected_openai_compatible_provider_ids() {
        assert_eq!(providers().len(), PROVIDER_COUNT);
        assert_eq!(providers().len(), EXPECTED_PROVIDER_IDS.len());
        for id in EXPECTED_PROVIDER_IDS {
            assert!(provider_by_id(id).is_some(), "missing provider id {id}");
        }
    }

    #[test]
    fn provider_ids_are_unique_and_aliases_resolve() {
        let mut ids = BTreeSet::new();
        for provider in providers() {
            assert!(
                ids.insert(provider.id),
                "duplicate provider id {}",
                provider.id
            );
            assert!(!provider.display_name.trim().is_empty());
            assert!(!provider.menu_detail.trim().is_empty());
            for alias in provider.aliases {
                assert!(!alias.trim().is_empty());
                assert_eq!(
                    resolve_provider(alias).map(|resolved| resolved.id),
                    Some(provider.id),
                    "alias {alias} should resolve to {}",
                    provider.id
                );
            }
        }
        assert_eq!(
            resolve_provider("z.ai").map(|provider| provider.id),
            Some("zai")
        );
        assert_eq!(
            resolve_provider("openai_compatible").map(|provider| provider.id),
            Some("openai-compatible")
        );
    }

    #[test]
    fn openai_compatible_profiles_have_safe_metadata() {
        assert_eq!(
            openai_compatible_profiles().len(),
            OPENAI_COMPATIBLE_PROFILE_COUNT
        );
        let mut ids = BTreeSet::new();
        for profile in openai_compatible_profiles() {
            assert!(
                ids.insert(profile.id),
                "duplicate profile id {}",
                profile.id
            );
            assert!(!profile.display_name.trim().is_empty());
            assert!(is_safe_env_key_name(profile.api_key_env));
            assert!(is_safe_env_file_name(profile.env_file));
            assert!(
                profile.api_base.starts_with("https://") || profile.api_base.starts_with("http://")
            );
            assert!(
                profile.setup_url.starts_with("https://")
                    || profile.setup_url.starts_with("http://")
            );
            if let Some(default_model) = profile.default_model {
                assert!(!default_model.trim().is_empty());
            }
            for alias in profile.aliases {
                assert_eq!(
                    resolve_openai_compatible_profile(alias).map(|resolved| resolved.id),
                    Some(profile.id),
                    "profile alias {alias} should resolve to {}",
                    profile.id
                );
            }
        }
    }

    #[test]
    fn credential_specs_cover_providers() {
        for provider in providers() {
            let spec = provider.credential_spec();
            assert_eq!(spec.provider_id, provider.id);
            assert!(!spec.auth_key.trim().is_empty());
            if let Some(env_var) = spec.env_var {
                assert!(is_safe_env_key_name(env_var));
            }
            if let Some(env_file) = spec.env_file {
                assert!(is_safe_env_file_name(env_file));
            }
            if spec.requires_api_key {
                assert!(
                    spec.env_var.is_some(),
                    "{} requires an env var",
                    provider.id
                );
                assert!(
                    spec.env_file.is_some(),
                    "{} requires an env file",
                    provider.id
                );
            }
        }

        let openrouter = provider_credential_spec("openrouter");
        assert_eq!(
            openrouter.and_then(|spec| spec.env_var),
            Some("OPENROUTER_API_KEY")
        );
        let ollama = provider_credential_spec("ollama");
        assert_eq!(ollama.map(|spec| spec.requires_api_key), Some(false));
    }

    #[test]
    fn helpers_expose_defaults_and_runtime_tiers() {
        assert_eq!(
            provider_by_id("deepseek").and_then(|provider| provider.default_model()),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            provider_by_id("openrouter").map(|provider| provider.runtime),
            Some(ProviderRuntimeSupport::OpenAiCompatible)
        );
        assert_eq!(
            provider_by_id("claude").and_then(|provider| provider.default_model()),
            Some("claude-3-5-sonnet-latest")
        );
        assert_eq!(
            provider_by_id("claude").map(|provider| provider.runtime),
            Some(ProviderRuntimeSupport::Native)
        );
        assert_eq!(
            provider_by_id("google").map(|provider| provider.runtime),
            Some(ProviderRuntimeSupport::MetadataOnly)
        );
    }

    #[test]
    fn claude_api_key_spec_matches_anthropic_runtime() {
        let spec = provider_credential_spec("claude")
            .unwrap_or_else(|| panic!("claude credential spec missing"));
        assert_eq!(spec.auth_key, "claude");
        assert_eq!(spec.env_var, Some("ANTHROPIC_API_KEY"));
        assert_eq!(spec.env_file, Some("anthropic.env"));
        assert!(spec.requires_api_key);
        assert!(spec
            .setup_url
            .is_some_and(|url| url.contains("anthropic.com")));
    }
}
