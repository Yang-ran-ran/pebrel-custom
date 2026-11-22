//! CLI 配置导出保持独立于 UI；只在明确应用时写入，并保留原文件备份。

use super::*;
use std::path::Path;

pub fn apply_claude_provider(provider: &AiProvider) -> Result<PathBuf, String> {
    if provider.kind != ProviderKind::Anthropic {
        return Err("Claude Code requires an Anthropic Messages endpoint".into());
    }
    if provider
        .subagent_model
        .as_ref()
        .is_some_and(|model| !configured_models(provider).contains(model))
    {
        return Err("Subagent model is not configured for this provider".into());
    }
    let key = Zeroizing::new(
        String::from_utf8(
            load_api_key(&provider.id)
                .map_err(|_| "Credential read failed")?
                .ok_or("Save an API key first")?,
        )
        .map_err(|_| "Invalid credential encoding")?,
    );
    let home = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| crate::platform::dirs::home_dir().map(|p| p.join(".claude")))
        .ok_or("Claude configuration directory is not available")?;
    write_claude_settings(&home, provider, &key).map_err(|error| error.to_string())?;
    Ok(home)
}

fn write_claude_settings(home: &Path, provider: &AiProvider, key: &str) -> io::Result<()> {
    let path = home.join("settings.json");
    std::fs::create_dir_all(home)?;
    let _lock = crate::atomic_file::try_lock(&path)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            "Claude configuration is busy; retry after hook configuration finishes",
        )
    })?;
    let old = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let mut config: serde_json::Value = match old.as_ref() {
        Some(bytes) => serde_json::from_slice(bytes).map_err(io::Error::other)?,
        None => serde_json::json!({}),
    };
    let object = config
        .as_object_mut()
        .ok_or_else(|| io::Error::other("settings.json must be an object"))?;
    let env = object
        .entry("env")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| io::Error::other("settings.env must be an object"))?;
    env.insert("ANTHROPIC_API_KEY".into(), key.into());
    env.remove("ANTHROPIC_AUTH_TOKEN");
    env.insert(
        "ANTHROPIC_BASE_URL".into(),
        provider
            .base_url
            .trim_end_matches('/')
            .strip_suffix("/v1")
            .unwrap_or(provider.base_url.trim_end_matches('/'))
            .into(),
    );
    env.insert("ANTHROPIC_MODEL".into(), provider.model.as_str().into());
    for (name, value) in [
        ("CLAUDE_CODE_SUBAGENT_MODEL", provider.subagent_model.as_deref().unwrap_or("")),
        ("CLAUDE_CODE_EFFORT_LEVEL", provider.reasoning_effort.as_str()),
    ] {
        if value.is_empty() {
            env.remove(name);
        } else {
            env.insert(name.into(), value.into());
        }
    }
    // 选择的是默认值，保留 CLI 的显式子 Agent 模型覆盖能力。
    env.remove("CLAUDE_CODE_SUBAGENT_MODEL_FORCE");
    let text = Zeroizing::new(serde_json::to_string_pretty(&config).map_err(io::Error::other)?);
    if let Some(old) = old {
        crate::atomic_file::write_private(&home.join("settings.json.pebrel.bak"), &old)?;
    }
    crate::atomic_file::write_private(&path, text.as_bytes())
}


/// Apply a provider to the selected Agent's native configuration.
/// Every writer uses a private file, an exclusive lock and a recoverable backup.
/// A provider is never marked active until this function has completed.
pub fn apply_provider_for_agent(provider: &AiProvider) -> Result<PathBuf, String> {
    use crate::ai_agents::AgentKind;
    match provider.agent_kind() {
        AgentKind::Claude => apply_claude_provider(provider),
        AgentKind::Codex => crate::codex_config::apply_provider(provider),
        AgentKind::Copilot => apply_copilot_provider(provider),
        AgentKind::Qwen => apply_qwen_provider(provider),
        AgentKind::CodeBuddy => apply_codebuddy_provider(provider),
        AgentKind::Droid => apply_droid_provider(provider),
        AgentKind::Grok => apply_grok_provider(provider),
        AgentKind::Kimi => apply_kimi_provider(provider),
        AgentKind::Aider => apply_aider_provider(provider),
        AgentKind::Goose => apply_goose_provider(provider),
        _ => Err(format!(
            "{} does not expose a verified file-based BYOK provider interface in this build; use its own model setup flow.",
            provider.agent_kind().display_name()
        )),
    }
}

fn api_key(provider: &AiProvider) -> Result<Zeroizing<String>, String> {
    let bytes = load_api_key(&provider.id)
        .map_err(|_| "Credential read failed")?
        .ok_or("Save an API key first")?;
    if bytes.is_empty() {
        return Err("Save an API key first".into());
    }
    String::from_utf8(bytes)
        .map(Zeroizing::new)
        .map_err(|_| "Invalid credential encoding".into())
}

fn home_dir() -> Result<PathBuf, String> {
    crate::platform::dirs::home_dir().ok_or_else(|| "Home directory is not available".into())
}

fn profile_path(variable: &str, fallback: PathBuf) -> Result<PathBuf, String> {
    match std::env::var_os(variable) {
        Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        _ => Ok(fallback),
    }
}

fn backup_name(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".pebrel.bak");
    PathBuf::from(name)
}

fn update_json_file<F>(path: &Path, update: F) -> io::Result<()>
where
    F: FnOnce(&mut serde_json::Value) -> io::Result<()>,
{
    use std::io;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _lock = crate::atomic_file::try_lock(path)?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::WouldBlock, "Agent API configuration is busy")
    })?;
    let old = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let mut document = match old.as_deref() {
        Some(bytes) => serde_json::from_slice::<serde_json::Value>(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        None => serde_json::json!({}),
    };
    if !document.is_object() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Agent JSON configuration must be an object"));
    }
    update(&mut document)?;
    let text = Zeroizing::new(
        serde_json::to_string_pretty(&document)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
    );
    if let Some(old) = old {
        crate::atomic_file::write_private(&backup_name(path), &old)?;
    }
    crate::atomic_file::write_private(path, text.as_bytes())
}

fn update_yaml_file<F>(path: &Path, update: F) -> io::Result<()>
where
    F: FnOnce(&mut serde_yaml::Value) -> io::Result<()>,
{
    use std::io;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _lock = crate::atomic_file::try_lock(path)?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::WouldBlock, "Agent API configuration is busy")
    })?;
    let old = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let mut document = match old.as_deref() {
        Some(bytes) => serde_yaml::from_slice::<serde_yaml::Value>(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        None => serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
    };
    if !document.is_mapping() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Agent YAML configuration must be a mapping"));
    }
    update(&mut document)?;
    let text = Zeroizing::new(
        serde_yaml::to_string(&document)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
    );
    if let Some(old) = old {
        crate::atomic_file::write_private(&backup_name(path), &old)?;
    }
    crate::atomic_file::write_private(path, text.as_bytes())
}

fn update_toml_file<F>(path: &Path, update: F) -> io::Result<()>
where
    F: FnOnce(&mut toml_edit::DocumentMut) -> io::Result<()>,
{
    use std::io;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _lock = crate::atomic_file::try_lock(path)?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::WouldBlock, "Agent API configuration is busy")
    })?;
    let old = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let source = old.as_deref().unwrap_or_default();
    let source = std::str::from_utf8(source)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut document = if source.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        source.parse::<toml_edit::DocumentMut>()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
    };
    update(&mut document)?;
    let text = Zeroizing::new(document.to_string());
    if let Some(old) = old {
        crate::atomic_file::write_private(&backup_name(path), &old)?;
    }
    crate::atomic_file::write_private(path, text.as_bytes())
}

fn provider_type(provider: &AiProvider) -> &'static str {
    match provider.kind {
        ProviderKind::Anthropic => "anthropic",
        _ => "openai",
    }
}

fn copilot_config_path() -> Result<PathBuf, String> {
    profile_path(
        "COPILOT_PROVIDERS_CONFIG",
        home_dir()?.join(".copilot").join("providers.json"),
    )
}

fn apply_copilot_provider(provider: &AiProvider) -> Result<PathBuf, String> {
    let key = api_key(provider)?;
    let path = copilot_config_path()?;
    let provider_name = format!("pebrel-{}", provider.agent_kind().slug());
    update_json_file(&path, |document| {
        let providers = document
            .get_mut("providers")
            .and_then(serde_json::Value::as_array_mut)
            .or_else(|| {
                document.as_object_mut()?.insert("providers".into(), serde_json::json!([]));
                document.get_mut("providers")?.as_array_mut()
            })
            .ok_or_else(|| io::Error::other("providers must be a JSON array"))?;
        providers.retain(|entry| {
            entry.get("name").and_then(serde_json::Value::as_str) != Some(provider_name.as_str())
        });
        let mut connection = serde_json::json!({
            "name": provider_name,
            "type": provider_type(provider),
            "baseUrl": provider.base_url.trim_end_matches('/'),
            "apiKey": key.as_str()
        });
        if provider.kind != ProviderKind::Anthropic {
            connection["wireApi"] = serde_json::Value::String(
                if provider.wire_api.eq_ignore_ascii_case("Responses") { "responses" } else { "completions" }.into()
            );
        }
        providers.push(connection);
        let models = document
            .get_mut("models")
            .and_then(serde_json::Value::as_array_mut)
            .or_else(|| {
                document.as_object_mut()?.insert("models".into(), serde_json::json!([]));
                document.get_mut("models")?.as_array_mut()
            })
            .ok_or_else(|| io::Error::other("models must be a JSON array"))?;
        models.retain(|entry| {
            entry.get("provider").and_then(serde_json::Value::as_str) != Some(provider_name.as_str())
        });
        models.push(serde_json::json!({"id": provider.model, "provider": provider_name}));
        Ok(())
    }).map_err(|error| error.to_string())?;
    Ok(path)
}

fn qwen_config_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".qwen").join("settings.json"))
}

fn apply_qwen_provider(provider: &AiProvider) -> Result<PathBuf, String> {
    let key = api_key(provider)?;
    let path = qwen_config_path()?;
    let auth_type = provider_type(provider);
    let env_key = format!(
        "PEBREL_{}_API_KEY",
        provider.agent_kind().slug().to_ascii_uppercase().replace('-', "_")
    );
    update_json_file(&path, |document| {
        let object = document.as_object_mut()
            .ok_or_else(|| io::Error::other("settings.json must be an object"))?;
        let model_providers = object.entry("modelProviders")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| io::Error::other("modelProviders must be an object"))?;
        let models = model_providers.entry(auth_type)
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or_else(|| io::Error::other("modelProviders entry must be an array"))?;
        models.retain(|entry| entry.get("id").and_then(serde_json::Value::as_str) != Some(provider.model.as_str()));
        let mut model = serde_json::json!({
            "id": provider.model,
            "name": provider.name,
            "envKey": env_key,
            "baseUrl": provider.base_url
        });
        if auth_type == "openai" {
            model["wireApi"] = serde_json::Value::String(
                if provider.wire_api.eq_ignore_ascii_case("Responses") { "responses" } else { "chat-completions" }.into()
            );
        }
        models.push(model);
        let env = object.entry("env").or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| io::Error::other("env must be an object"))?;
        env.insert(env_key, serde_json::Value::String(key.to_string()));
        object.entry("security").or_insert_with(|| serde_json::json!({}))
            .as_object_mut().ok_or_else(|| io::Error::other("security must be an object"))?
            .entry("auth").or_insert_with(|| serde_json::json!({}))
            .as_object_mut().ok_or_else(|| io::Error::other("security.auth must be an object"))?
            .insert("selectedType".into(), serde_json::Value::String(auth_type.into()));
        object.entry("model").or_insert_with(|| serde_json::json!({}))
            .as_object_mut().ok_or_else(|| io::Error::other("model must be an object"))?
            .insert("name".into(), serde_json::Value::String(provider.model.clone()));
        Ok(())
    }).map_err(|error| error.to_string())?;
    Ok(path)
}

fn codebuddy_config_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".codebuddy").join("models.json"))
}

fn apply_codebuddy_provider(provider: &AiProvider) -> Result<PathBuf, String> {
    let key = api_key(provider)?;
    let path = codebuddy_config_path()?;
    let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));
    update_json_file(&path, |document| {
        let object = document.as_object_mut()
            .ok_or_else(|| io::Error::other("models.json must be an object"))?;
        let models = object.entry("models").or_insert_with(|| serde_json::json!([]))
            .as_array_mut().ok_or_else(|| io::Error::other("models must be an array"))?;
        models.retain(|entry| entry.get("id").and_then(serde_json::Value::as_str) != Some(provider.model.as_str()));
        models.push(serde_json::json!({
            "id": provider.model,
            "name": provider.name,
            "vendor": "Pebrel",
            "url": url,
            "apiKey": key.as_str(),
            "maxInputTokens": 128000,
            "maxOutputTokens": 8192,
            "supportsToolCall": true,
            "supportsImages": false
        }));
        let available = object.entry("availableModels").or_insert_with(|| serde_json::json!([]))
            .as_array_mut().ok_or_else(|| io::Error::other("availableModels must be an array"))?;
        if !available.iter().any(|entry| entry.as_str() == Some(provider.model.as_str())) {
            available.push(serde_json::Value::String(provider.model.clone()));
        }
        Ok(())
    }).map_err(|error| error.to_string())?;
    Ok(path)
}

fn droid_config_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".factory").join("settings.json"))
}

fn apply_droid_provider(provider: &AiProvider) -> Result<PathBuf, String> {
    let key = api_key(provider)?;
    let path = droid_config_path()?;
    update_json_file(&path, |document| {
        let object = document.as_object_mut()
            .ok_or_else(|| io::Error::other("settings.json must be an object"))?;
        let models = object.entry("customModels").or_insert_with(|| serde_json::json!([]))
            .as_array_mut().ok_or_else(|| io::Error::other("customModels must be an array"))?;
        models.retain(|entry| entry.get("model").and_then(serde_json::Value::as_str) != Some(provider.model.as_str()));
        models.push(serde_json::json!({
            "model": provider.model,
            "displayName": provider.name,
            "baseUrl": provider.base_url,
            "apiKey": key.as_str(),
            "provider": "generic-chat-completion-api",
            "maxOutputTokens": 8192
        }));
        Ok(())
    }).map_err(|error| error.to_string())?;
    Ok(path)
}

fn grok_config_path() -> Result<PathBuf, String> {
    let directory = std::env::var_os("GROK_HOME")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".grok"));
    Ok(directory.join("config.toml"))
}

fn apply_grok_provider(provider: &AiProvider) -> Result<PathBuf, String> {
    let key = api_key(provider)?;
    let path = grok_config_path()?;
    update_toml_file(&path, |document| {
        if !document.get("models").is_some_and(|item| item.is_table()) {
            document["models"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        document["models"]["default"] = toml_edit::value(provider.model.as_str());
        if !document.get("model").is_some_and(|item| item.is_table()) {
            document["model"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let entry = &mut document["model"][provider.model.as_str()];
        if !entry.is_table() {
            *entry = toml_edit::Item::Table(toml_edit::Table::new());
        }
        entry["model"] = toml_edit::value(provider.model.as_str());
        entry["name"] = toml_edit::value(provider.name.as_str());
        entry["base_url"] = toml_edit::value(provider.base_url.trim_end_matches('/'));
        entry["api_backend"] = toml_edit::value(
            if provider.wire_api.eq_ignore_ascii_case("Responses") { "responses" } else { "chat_completions" }
        );
        let mut headers = toml_edit::InlineTable::new();
        headers.insert("Authorization", toml_edit::Value::from(format!("Bearer {}", key.as_str())));
        entry["extra_headers"] = toml_edit::value(headers);
        Ok(())
    }).map_err(|error| error.to_string())?;
    Ok(path)
}

fn kimi_config_path() -> Result<PathBuf, String> {
    let directory = std::env::var_os("KIMI_CODE_HOME")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".kimi-code"));
    Ok(directory.join("config.toml"))
}

fn apply_kimi_provider(provider: &AiProvider) -> Result<PathBuf, String> {
    let key = api_key(provider)?;
    let path = kimi_config_path()?;
    let provider_name = format!("pebrel_{}", provider.agent_kind().slug().replace('-', "_"));
    let model_name = format!("{provider_name}/{}", provider.model);
    update_toml_file(&path, |document| {
        if !document.get("providers").is_some_and(|item| item.is_table()) {
            document["providers"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let entry = &mut document["providers"][provider_name.as_str()];
        if !entry.is_table() { *entry = toml_edit::Item::Table(toml_edit::Table::new()); }
        entry["type"] = toml_edit::value(match provider.kind {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Google => "google-genai",
            _ if provider.wire_api.eq_ignore_ascii_case("Responses") => "openai_responses",
            _ => "openai",
        });
        entry["base_url"] = toml_edit::value(provider.base_url.trim_end_matches('/'));
        entry["api_key"] = toml_edit::value(key.as_str());
        if !document.get("models").is_some_and(|item| item.is_table()) {
            document["models"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let model = &mut document["models"][model_name.as_str()];
        if !model.is_table() { *model = toml_edit::Item::Table(toml_edit::Table::new()); }
        model["provider"] = toml_edit::value(provider_name.as_str());
        model["model"] = toml_edit::value(provider.model.as_str());
        document["default_model"] = toml_edit::value(model_name.as_str());
        Ok(())
    }).map_err(|error| error.to_string())?;
    Ok(path)
}

fn apply_aider_provider(provider: &AiProvider) -> Result<PathBuf, String> {
    let key = api_key(provider)?;
    let path = home_dir()?.join(".aider.conf.yml");
    let model = match provider.kind {
        ProviderKind::Anthropic => format!("anthropic/{}", provider.model),
        _ => format!("openai/{}", provider.model),
    };
    update_yaml_file(&path, |document| {
        let mapping = document.as_mapping_mut()
            .ok_or_else(|| io::Error::other(".aider.conf.yml must be a YAML mapping"))?;
        let set = |mapping: &mut serde_yaml::Mapping, name: &str, value: serde_yaml::Value| {
            mapping.insert(serde_yaml::Value::String(name.into()), value);
        };
        set(mapping, "model", serde_yaml::Value::String(model));
        match provider.kind {
            ProviderKind::Anthropic => {
                set(mapping, "anthropic-api-key", serde_yaml::Value::String(key.to_string()));
            },
            _ => {
                set(mapping, "openai-api-key", serde_yaml::Value::String(key.to_string()));
                set(mapping, "openai-api-base", serde_yaml::Value::String(provider.base_url.trim_end_matches('/').into()));
            },
        }
        Ok(())
    }).map_err(|error| error.to_string())?;
    Ok(path)
}

fn goose_config_directory() -> Result<PathBuf, String> {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        Ok(PathBuf::from(appdata).join("Block").join("goose").join("config").join("custom_providers"))
    } else {
        Ok(home_dir()?.join(".config").join("goose").join("custom_providers"))
    }
}

fn apply_goose_provider(provider: &AiProvider) -> Result<PathBuf, String> {
    let key = api_key(provider)?;
    let directory = goose_config_directory()?;
    let provider_name = format!("pebrel_{}", provider.agent_kind().slug().replace('-', "_"));
    let path = directory.join(format!("{provider_name}.json"));
    let endpoint = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));
    update_json_file(&path, |document| {
        *document = serde_json::json!({
            "name": provider_name,
            "engine": "openai",
            "display_name": provider.name,
            "description": "Managed by Pebrel. Reapply from Pebrel to update.",
            "base_url": endpoint,
            "api_key_env": format!("PEBREL_GOOSE_{}_API_KEY", provider.agent_kind().slug().to_ascii_uppercase().replace('-', "_")),
            "models": [{"name": provider.model, "context_limit": 128000}],
            "headers": {"Authorization": format!("Bearer {}", key.as_str())},
            "supports_streaming": true,
            "requires_auth": true
        });
        Ok(())
    }).map_err(|error| error.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_export_respects_hook_writer_and_preserves_its_update() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        let guard = crate::atomic_file::try_lock(&path).unwrap().unwrap();
        let provider = AiProvider::preset(ProviderKind::Anthropic, "shared-lock");
        let error = std::thread::scope(|scope| {
            scope
                .spawn(|| write_claude_settings(temp.path(), &provider, "key"))
                .join()
                .unwrap()
                .unwrap_err()
        });
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        crate::atomic_file::write(&path, br#"{"hooks":{"Stop":[{"command":"hook-updated"}]}}"#)
            .unwrap();
        drop(guard);
        write_claude_settings(temp.path(), &provider, "key").unwrap();
        assert!(std::fs::read_to_string(path).unwrap().contains("hook-updated"));
    }

    #[test]
    fn claude_export_keeps_hooks_and_uses_same_provider_subagent_model() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("settings.json"),
            r#"{"hooks":{"Stop":[]},"env":{"KEEP":"yes"}}"#,
        )
        .unwrap();
        let mut provider = AiProvider::preset(ProviderKind::Anthropic, "claude");
        provider.models = vec!["claude-small".into()];
        provider.subagent_model = Some("claude-small".into());
        provider.reasoning_effort = "max".into();
        write_claude_settings(temp.path(), &provider, "test-key").unwrap();
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temp.path().join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(result["hooks"]["Stop"], serde_json::json!([]));
        assert_eq!(result["env"]["KEEP"], "yes");
        assert_eq!(result["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "claude-small");
        assert_eq!(result["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "max");
        assert_eq!(result["env"]["ANTHROPIC_BASE_URL"], "https://api.anthropic.com");
        assert!(temp.path().join("settings.json.pebrel.bak").exists());
        {
            for name in ["settings.json", "settings.json.pebrel.bak"] {
                crate::platform::private_file::assert_private(&temp.path().join(name));
            }
        }
    }
}
