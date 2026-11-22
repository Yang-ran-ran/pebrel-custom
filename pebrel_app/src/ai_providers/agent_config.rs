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
        AgentKind::DeepSeekHarness => apply_deepseek_harness_provider(provider),
        _ => Err(format!(
            "{} does not expose a verified file-based BYOK provider interface in this build; use its own model setup flow.",
            provider.agent_kind().display_name()
        )),
    }
}


/// DeepSeek Harness reads this reference through its own credential service.
/// Keep the key in the Harness home store, not in Cordis plugin configuration.
fn apply_deepseek_harness_provider(provider: &AiProvider) -> Result<PathBuf, String> {
    if provider.kind != ProviderKind::DeepSeek {
        return Err("DeepSeek Harness requires the DeepSeek API provider preset".into());
    }
    let key = api_key(provider)?;
    let home = std::env::var_os("DSH_HOME")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".dsh"));
    let path = home.join(".credentials.yaml");
    write_harness_credential_ref(&path, "DEEPSEEK_API_KEY", key.as_str())
        .map_err(|error| format!("Could not update the DeepSeek Harness credential store: {error}"))?;
    Ok(path)
}

/// Serialize with the same filename.lock convention as dsh-atomic-write.
/// The Harness watcher reloads the versioned YAML document after replacement.
fn write_harness_credential_ref(path: &Path, name: &str, secret: &str) -> io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write as _;
    use std::time::{Duration, Instant};

    let parent = path.parent().ok_or_else(|| io::Error::other("credential path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let mut lock_name = path.as_os_str().to_owned();
    lock_name.push(".lock");
    let lock_path = PathBuf::from(lock_name);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut delay = Duration::from_millis(20);
    let _lock = loop {
        match OpenOptions::new().write(true).create_new(true).open(&lock_path) {
            Ok(mut file) => {
                let guard = HarnessCredentialLock(lock_path.clone());
                writeln!(file, "{}", std::process::id())?;
                file.sync_all()?;
                break guard;
            },
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let stale = std::fs::metadata(&lock_path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age >= Duration::from_secs(30));
                if stale {
                    let _ = std::fs::remove_file(&lock_path);
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "DeepSeek Harness credentials are being updated by another process",
                    ));
                }
                std::thread::sleep(delay);
                delay = std::cmp::min(delay * 2, Duration::from_millis(200));
            },
            Err(error) => return Err(error),
        }
    };

    let old = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let source = old.as_deref().unwrap_or_default();
    let source = std::str::from_utf8(source)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if !source.trim().is_empty() {
        let parsed: serde_yaml::Value = serde_yaml::from_str(source)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if !parsed.is_mapping() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "DeepSeek Harness credentials document must be a YAML mapping",
            ));
        }
    }

    let yaml_secret = serde_yaml::to_string(&serde_yaml::Value::String(secret.to_owned()))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let yaml_secret = yaml_secret.trim_end();
    let next = patch_harness_refs(source, name, yaml_secret);
    let parsed: serde_yaml::Value = serde_yaml::from_str(&next)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let value = parsed
        .get("refs")
        .and_then(serde_yaml::Value::as_mapping)
        .and_then(|refs| refs.get(&serde_yaml::Value::String(name.to_owned())))
        .and_then(serde_yaml::Value::as_str);
    if value != Some(secret) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "DeepSeek Harness credential reference did not round-trip",
        ));
    }
    crate::atomic_file::write_private(path, next.as_bytes())?;
    drop(_lock);
    Ok(())
}

struct HarnessCredentialLock(PathBuf);

impl Drop for HarnessCredentialLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Patch only one entry under refs, retaining unrelated refs, records and
/// comments. The complete document is parsed again before the atomic commit.
fn patch_harness_refs(source: &str, name: &str, encoded: &str) -> String {
    let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
    let refs_index = lines.iter().position(|line| line == "refs:");
    if let Some(start) = refs_index {
        let end = lines.iter().enumerate().skip(start + 1).find_map(|(index, line)| {
            (!line.trim().is_empty()
                && !line.starts_with(' ')
                && !line.starts_with('\t'))
                .then_some(index)
        }).unwrap_or(lines.len());
        let key_prefix = format!("{name}:");
        let quoted_prefix = format!("\"{name}\":");
        if let Some(index) = (start + 1..end).find(|index| {
            let entry = lines[*index].trim_start();
            entry.starts_with(&key_prefix) || entry.starts_with(&quoted_prefix)
        }) {
            let suffix = lines[index].find(" #").map(|position| &lines[index][position..]).unwrap_or("");
            lines[index] = format!("  {name}: {encoded}{suffix}");
        } else {
            lines.insert(end, format!("  {name}: {encoded}"));
        }
    } else {
        if !lines.iter().any(|line| line.starts_with("version:")) {
            lines.insert(0, "version: 1".into());
        }
        let records = lines.iter().position(|line| line == "records:");
        let mut insert = records.unwrap_or(lines.len());
        lines.insert(insert, "refs:".into());
        insert += 1;
        lines.insert(insert, format!("  {name}: {encoded}"));
    }
    format!("{}\n", lines.join("\n"))
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

    #[test]
    fn deepseek_harness_credential_write_preserves_other_refs_and_records() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".credentials.yaml");
        std::fs::write(
            &path,
            "version: 1\n# keep this note\nrefs:\n  OPENAI_API_KEY: existing\nrecords:\n  llm-pi-ai/openai-codex:\n    kind: grant\n    payload:\n      access: grant-token\n",
        ).unwrap();

        write_harness_credential_ref(&path, "DEEPSEEK_API_KEY", "sk-deepseek-test").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert_eq!(parsed["refs"]["OPENAI_API_KEY"], "existing");
        assert_eq!(parsed["refs"]["DEEPSEEK_API_KEY"], "sk-deepseek-test");
        assert_eq!(parsed["records"]["llm-pi-ai/openai-codex"]["payload"]["access"], "grant-token");
        assert!(content.contains("# keep this note"));
        assert!(path.with_file_name(".credentials.yaml.lock").exists() == false);
        crate::platform::private_file::assert_private(&path);
    }

    #[test]
    fn deepseek_harness_credential_write_updates_only_its_own_reference() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".credentials.yaml");
        std::fs::write(
            &path,
            "version: 1\nrefs:\n  DEEPSEEK_API_KEY: old-key # credential note\n  OTHER_API_KEY: keep\n",
        ).unwrap();

        write_harness_credential_ref(&path, "DEEPSEEK_API_KEY", "new-key").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert_eq!(parsed["refs"]["DEEPSEEK_API_KEY"], "new-key");
        assert_eq!(parsed["refs"]["OTHER_API_KEY"], "keep");
        assert!(content.contains("# credential note"));
    }
}
